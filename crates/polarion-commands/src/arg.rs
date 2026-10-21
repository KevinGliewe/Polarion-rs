//! Typed arguments: references, rich text, text that may come from a file, field selections,
//! binary data and the deliberately dynamic JSON values. Each deserializes from the forms a person
//! or an agent would write, parses from a command-line string, and describes itself as JSON
//! Schema.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use polarion::{DocumentRef, Fields, ItemRef, Query, Text};
use polarion_commands_derive::cmd_args;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::command::Context;
use crate::error::Result;

/// `Deserialize` for types written as a string (parsed with `FromStr`) or as an object (read into
/// `Repr`, then converted); `Serialize` as the string (`Display`).
macro_rules! string_or_object {
    ($ty:ty, $repr:ty, $expecting:literal) => {
        deserialize_string_or_object!($ty, $repr, $expecting);

        impl Serialize for $ty {
            fn serialize<S: Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
    };
}

/// The `Deserialize` of [`string_or_object!`] alone, for types whose string form is lossy.
macro_rules! deserialize_string_or_object {
    ($ty:ty, $repr:ty, $expecting:literal) => {
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(
                deserializer: D,
            ) -> std::result::Result<Self, D::Error> {
                struct V;
                impl<'de> Visitor<'de> for V {
                    type Value = $ty;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str($expecting)
                    }
                    fn visit_str<E: de::Error>(self, s: &str) -> std::result::Result<$ty, E> {
                        s.parse().map_err(E::custom)
                    }
                    fn visit_map<A: MapAccess<'de>>(
                        self,
                        map: A,
                    ) -> std::result::Result<$ty, A::Error> {
                        let repr =
                            <$repr>::deserialize(de::value::MapAccessDeserializer::new(map))?;
                        <$ty>::try_from(repr).map_err(de::Error::custom)
                    }
                }
                deserializer.deserialize_any(V)
            }
        }
    };
}

/// A schema that is always inlined.
macro_rules! inline_schema {
    ($ty:ty, $name:literal, $schema:tt) => {
        impl JsonSchema for $ty {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed($name)
            }
            fn inline_schema() -> bool {
                true
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!($schema)
            }
        }
    };
}

// ---- work items

/// A work item: `ID` (in the default project), `PROJECT/ID`, a URI, or `{project, id}`/`{uri}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemArg {
    /// An id in the default project.
    Id(String),
    Qualified {
        project: String,
        id: String,
    },
    Uri(String),
}

impl ItemArg {
    pub fn resolve(&self, cx: &Context<'_>) -> Result<ItemRef> {
        Ok(match self {
            ItemArg::Id(id) => ItemRef::new(cx.default_project_for(id)?, id),
            ItemArg::Qualified { project, id } => ItemRef::new(project, id),
            ItemArg::Uri(uri) => ItemRef::from_uri(uri.clone()),
        })
    }

    pub fn resolve_all(items: &[ItemArg], cx: &Context<'_>) -> Result<Vec<ItemRef>> {
        items.iter().map(|i| i.resolve(cx)).collect()
    }
}

impl FromStr for ItemArg {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let s = s.trim();
        if s.starts_with("subterra:") {
            return Ok(ItemArg::Uri(s.to_owned()));
        }
        match s.split('/').collect::<Vec<_>>()[..] {
            [project, id] if !project.is_empty() && !id.is_empty() => Ok(ItemArg::Qualified {
                project: project.to_owned(),
                id: id.to_owned(),
            }),
            [id] if !id.is_empty() => Ok(ItemArg::Id(id.to_owned())),
            _ => Err(format!("expected ID, PROJECT/ID or a URI, got `{s}`")),
        }
    }
}

impl fmt::Display for ItemArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ItemArg::Id(id) | ItemArg::Uri(id) => f.write_str(id),
            ItemArg::Qualified { project, id } => write!(f, "{project}/{id}"),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ItemRepr {
    Ref { project: String, id: String },
    Uri { uri: String },
}

impl TryFrom<ItemRepr> for ItemArg {
    type Error = String;
    fn try_from(repr: ItemRepr) -> std::result::Result<Self, String> {
        Ok(match repr {
            ItemRepr::Ref { project, id } => ItemArg::Qualified { project, id },
            ItemRepr::Uri { uri } => ItemArg::Uri(uri),
        })
    }
}

string_or_object!(
    ItemArg,
    ItemRepr,
    "ID, PROJECT/ID, a URI, {project, id} or {uri}"
);
inline_schema!(ItemArg, "ItemArg", {"type": "string", "format": "work-item"});

// ---- follow-ups of new work items

/// `LEFT=RIGHT`, split at the first `=`, both sides non-empty.
fn split_assignment<'s>(s: &'s str, form: &str) -> std::result::Result<(&'s str, &'s str), String> {
    match s.trim().split_once('=') {
        Some((left, right)) if !left.trim().is_empty() && !right.trim().is_empty() => {
            Ok((left.trim(), right.trim()))
        }
        _ => Err(format!("expected {form}, got `{s}`")),
    }
}

/// A role: no `:` or `/`, which catches a URL given without its role.
fn role(role: &str, form: &str) -> std::result::Result<String, String> {
    if role.contains([':', '/']) {
        return Err(format!("expected {form}: `{role}` is not a role id"));
    }
    Ok(role.to_owned())
}

/// A link to another work item: `ROLE=ITEM` or `ROLE=ITEM@REVISION`, or
/// `{role, item, revision?, suspect?}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkArg {
    pub role: String,
    pub item: ItemArg,
    pub revision: Option<String>,
    pub suspect: bool,
}

impl LinkArg {
    pub fn resolve(&self, cx: &Context<'_>) -> Result<polarion::NewLink> {
        Ok(polarion::NewLink {
            role: self.role.clone(),
            target: self.item.resolve(cx)?,
            revision: self.revision.clone(),
            suspect: self.suspect,
        })
    }
}

impl FromStr for LinkArg {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        const FORM: &str = "ROLE=ITEM or ROLE=ITEM@REVISION";
        let (role_id, rest) = split_assignment(s, FORM)?;
        let (item, revision) = match rest.rsplit_once('@') {
            Some((item, revision)) if !revision.is_empty() => (item, Some(revision.to_owned())),
            _ => (rest, None),
        };
        Ok(Self {
            role: role(role_id, FORM)?,
            item: item.parse()?,
            revision,
            suspect: false,
        })
    }
}

impl fmt::Display for LinkArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.role, self.item)?;
        match &self.revision {
            Some(revision) => write!(f, "@{revision}"),
            None => Ok(()),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkRepr {
    role: String,
    item: ItemArg,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    suspect: bool,
}

impl TryFrom<LinkRepr> for LinkArg {
    type Error = String;
    fn try_from(r: LinkRepr) -> std::result::Result<Self, String> {
        Ok(Self {
            role: role(&r.role, "a link role")?,
            item: r.item,
            revision: r.revision,
            suspect: r.suspect,
        })
    }
}

deserialize_string_or_object!(
    LinkArg,
    LinkRepr,
    "ROLE=ITEM, ROLE=ITEM@REVISION or {role, item, revision?, suspect?}"
);

impl Serialize for LinkArg {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        if !self.suspect {
            return serializer.collect_str(self);
        }
        // `suspect` has no string form.
        let mut map = Map::new();
        map.insert("role".into(), self.role.clone().into());
        map.insert("item".into(), self.item.to_string().into());
        if let Some(revision) = &self.revision {
            map.insert("revision".into(), revision.clone().into());
        }
        map.insert("suspect".into(), true.into());
        Value::Object(map).serialize(serializer)
    }
}

inline_schema!(LinkArg, "LinkArg", {"anyOf": [
    {"type": "string", "description": "ROLE=ITEM or ROLE=ITEM@REVISION (ITEM: ID, PROJECT/ID or URI)"},
    {"type": "object", "properties": {
        "role": {"type": "string"},
        "item": {"type": "string", "format": "work-item"},
        "revision": {"type": "string"},
        "suspect": {"type": "boolean"}
    }, "required": ["role", "item"], "additionalProperties": false}
]});

/// A hyperlink, or a link to a work item on another server: `ROLE=URL` or `{role, url}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperlinkArg {
    pub role: String,
    pub url: String,
}

impl FromStr for HyperlinkArg {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        const FORM: &str = "ROLE=URL";
        let (role_id, url) = split_assignment(s, FORM)?;
        Ok(Self {
            role: role(role_id, FORM)?,
            url: url.to_owned(),
        })
    }
}

impl fmt::Display for HyperlinkArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.role, self.url)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HyperlinkRepr {
    role: String,
    url: String,
}

impl TryFrom<HyperlinkRepr> for HyperlinkArg {
    type Error = String;
    fn try_from(r: HyperlinkRepr) -> std::result::Result<Self, String> {
        Ok(Self {
            role: role(&r.role, "a role")?,
            url: r.url,
        })
    }
}

string_or_object!(HyperlinkArg, HyperlinkRepr, "ROLE=URL or {role, url}");
inline_schema!(HyperlinkArg, "HyperlinkArg", {"anyOf": [
    {"type": "string", "description": "ROLE=URL"},
    {"type": "object", "properties": {"role": {"type": "string"}, "url": {"type": "string"}},
     "required": ["role", "url"], "additionalProperties": false}
]});

/// A link to an OSLC resource: `ROLE=LABEL=URL` or `{role, label, url}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OslcLinkArg {
    pub role: String,
    pub label: String,
    pub url: String,
}

impl FromStr for OslcLinkArg {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        const FORM: &str = "ROLE=LABEL=URL";
        let (role_id, rest) = split_assignment(s, FORM)?;
        let (label, url) = split_assignment(rest, FORM)?;
        Ok(Self {
            role: role(role_id, FORM)?,
            label: label.to_owned(),
            url: url.to_owned(),
        })
    }
}

impl fmt::Display for OslcLinkArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}={}", self.role, self.label, self.url)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OslcLinkRepr {
    role: String,
    label: String,
    url: String,
}

impl TryFrom<OslcLinkRepr> for OslcLinkArg {
    type Error = String;
    fn try_from(r: OslcLinkRepr) -> std::result::Result<Self, String> {
        if r.label.contains('=') {
            return Err(format!("an OSLC link label has no `=`: `{}`", r.label));
        }
        Ok(Self {
            role: role(&r.role, "a role")?,
            label: r.label,
            url: r.url,
        })
    }
}

string_or_object!(
    OslcLinkArg,
    OslcLinkRepr,
    "ROLE=LABEL=URL or {role, label, url}"
);
inline_schema!(OslcLinkArg, "OslcLinkArg", {"anyOf": [
    {"type": "string", "description": "ROLE=LABEL=URL"},
    {"type": "object", "properties": {
        "role": {"type": "string"}, "label": {"type": "string"}, "url": {"type": "string"}
    }, "required": ["role", "label", "url"], "additionalProperties": false}
]});

/// A planning constraint: `CONSTRAINT=DATE` or `{constraint, date}` (date `YYYY-MM-DD`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintArg {
    pub constraint: String,
    pub date: String,
}

impl FromStr for ConstraintArg {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let (constraint, date) = split_assignment(s, "CONSTRAINT=DATE")?;
        Ok(Self {
            constraint: constraint.to_owned(),
            date: date.to_owned(),
        })
    }
}

impl fmt::Display for ConstraintArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.constraint, self.date)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConstraintRepr {
    constraint: String,
    date: String,
}

impl TryFrom<ConstraintRepr> for ConstraintArg {
    type Error = String;
    fn try_from(r: ConstraintRepr) -> std::result::Result<Self, String> {
        Ok(Self {
            constraint: r.constraint,
            date: r.date,
        })
    }
}

string_or_object!(
    ConstraintArg,
    ConstraintRepr,
    "CONSTRAINT=DATE or {constraint, date}"
);
inline_schema!(ConstraintArg, "ConstraintArg", {"anyOf": [
    {"type": "string", "description": "CONSTRAINT=DATE, e.g. startNoEarlierThan=2024-01-31"},
    {"type": "object", "properties": {"constraint": {"type": "string"}, "date": {"type": "string"}},
     "required": ["constraint", "date"], "additionalProperties": false}
]});

/// A source repository revision: `REVISION` (default repository), `REPOSITORY=REVISION`, or
/// `{repository?, revision}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionArg {
    pub repository: Option<String>,
    pub revision: String,
}

impl FromStr for RevisionArg {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let s = s.trim();
        if !s.contains('=') {
            if s.is_empty() {
                return Err("expected REVISION or REPOSITORY=REVISION".into());
            }
            return Ok(Self {
                repository: None,
                revision: s.to_owned(),
            });
        }
        let (repository, revision) = split_assignment(s, "REVISION or REPOSITORY=REVISION")?;
        Ok(Self {
            repository: Some(repository.to_owned()),
            revision: revision.to_owned(),
        })
    }
}

impl fmt::Display for RevisionArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repository {
            Some(repository) => write!(f, "{repository}={}", self.revision),
            None => f.write_str(&self.revision),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevisionRepr {
    #[serde(default)]
    repository: Option<String>,
    revision: String,
}

impl TryFrom<RevisionRepr> for RevisionArg {
    type Error = String;
    fn try_from(r: RevisionRepr) -> std::result::Result<Self, String> {
        Ok(Self {
            repository: r.repository,
            revision: r.revision,
        })
    }
}

string_or_object!(
    RevisionArg,
    RevisionRepr,
    "REVISION, REPOSITORY=REVISION or {repository?, revision}"
);
inline_schema!(RevisionArg, "RevisionArg", {"anyOf": [
    {"type": "string", "description": "REVISION (default repository) or REPOSITORY=REVISION"},
    {"type": "object", "properties": {"repository": {"type": "string"}, "revision": {"type": "string"}},
     "required": ["revision"], "additionalProperties": false}
]});

/// A plan: its URI, `ID` (in the default project) or `PROJECT/ID`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanArg(pub String);

impl PlanArg {
    /// The plan's URI; an id is looked up.
    pub async fn resolve(&self, cx: &Context<'_>) -> Result<String> {
        let s = self.0.trim();
        if s.starts_with("subterra:") {
            return Ok(s.to_owned());
        }
        let (project, id) = match s.split_once('/') {
            Some((project, id)) => (project.to_owned(), id),
            None => (cx.project(None)?, s),
        };
        let plan = cx.polarion.plans().get(&project, id).await?;
        plan.uri.ok_or_else(|| {
            crate::CommandError::from(polarion::Error::Invalid(format!(
                "the plan `{project}/{id}` has no URI"
            )))
        })
    }
}

impl FromStr for PlanArg {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        if s.trim().is_empty() || s.split('/').count() > 2 && !s.trim().starts_with("subterra:") {
            return Err(format!("expected a plan URI, ID or PROJECT/ID, got `{s}`"));
        }
        Ok(Self(s.trim().to_owned()))
    }
}

impl fmt::Display for PlanArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PlanArg {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

impl Serialize for PlanArg {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

inline_schema!(PlanArg, "PlanArg", {"type": "string", "description": "a plan URI, ID or PROJECT/ID"});

// ---- documents

/// A document: `SPACE/NAME` (in the default project), `PROJECT/SPACE/NAME`, a URI, or
/// `{project, space, name}`/`{uri}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentArg {
    /// A document in the default project.
    Short {
        space: String,
        name: String,
    },
    Qualified {
        project: String,
        space: String,
        name: String,
    },
    Uri(String),
}

impl DocumentArg {
    pub fn resolve(&self, cx: &Context<'_>) -> Result<DocumentRef> {
        Ok(match self {
            DocumentArg::Short { space, name } => {
                DocumentRef::new(cx.default_project_for(&self.to_string())?, space, name)
            }
            DocumentArg::Qualified {
                project,
                space,
                name,
            } => DocumentRef::new(project, space, name),
            DocumentArg::Uri(uri) => DocumentRef::from_uri(uri.clone()),
        })
    }
}

impl FromStr for DocumentArg {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let s = s.trim();
        if s.starts_with("subterra:") {
            return Ok(DocumentArg::Uri(s.to_owned()));
        }
        match s.split('/').collect::<Vec<_>>()[..] {
            [project, space, name] if ![project, space, name].contains(&"") => {
                Ok(DocumentArg::Qualified {
                    project: project.to_owned(),
                    space: space.to_owned(),
                    name: name.to_owned(),
                })
            }
            [space, name] if !space.is_empty() && !name.is_empty() => Ok(DocumentArg::Short {
                space: space.to_owned(),
                name: name.to_owned(),
            }),
            _ => Err(format!(
                "expected SPACE/NAME, PROJECT/SPACE/NAME or a URI, got `{s}`"
            )),
        }
    }
}

impl fmt::Display for DocumentArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentArg::Short { space, name } => write!(f, "{space}/{name}"),
            DocumentArg::Qualified {
                project,
                space,
                name,
            } => write!(f, "{project}/{space}/{name}"),
            DocumentArg::Uri(uri) => f.write_str(uri),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DocumentRepr {
    Ref {
        project: String,
        space: String,
        name: String,
    },
    Uri {
        uri: String,
    },
}

impl TryFrom<DocumentRepr> for DocumentArg {
    type Error = String;
    fn try_from(repr: DocumentRepr) -> std::result::Result<Self, String> {
        Ok(match repr {
            DocumentRepr::Ref {
                project,
                space,
                name,
            } => DocumentArg::Qualified {
                project,
                space,
                name,
            },
            DocumentRepr::Uri { uri } => DocumentArg::Uri(uri),
        })
    }
}

string_or_object!(
    DocumentArg,
    DocumentRepr,
    "SPACE/NAME, PROJECT/SPACE/NAME, a URI, {project, space, name} or {uri}"
);
inline_schema!(DocumentArg, "DocumentArg", {"type": "string", "format": "document"});

// ---- rich text

/// Rich text: a string (plain text) or `{"type": "text/html", "content": "..."}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichText {
    pub content_type: String,
    pub content: String,
}

impl RichText {
    pub fn plain(content: impl Into<String>) -> Self {
        Self {
            content_type: "text/plain".into(),
            content: content.into(),
        }
    }
}

impl From<RichText> for Text {
    fn from(t: RichText) -> Self {
        Text {
            content_type: t.content_type,
            content: t.content,
        }
    }
}

impl FromStr for RichText {
    type Err = String;
    /// Plain text, or a JSON object `{type, content}` (on the command line).
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        if s.trim_start().starts_with('{')
            && let Ok(repr) = serde_json::from_str::<TextRepr>(s)
        {
            return Ok(repr.into());
        }
        Ok(RichText::plain(s))
    }
}

impl fmt::Display for RichText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.content)
    }
}

#[derive(Deserialize, Serialize)]
struct TextRepr {
    #[serde(rename = "type", default = "text_plain")]
    content_type: String,
    content: String,
}

fn text_plain() -> String {
    "text/plain".into()
}

impl From<TextRepr> for RichText {
    fn from(r: TextRepr) -> Self {
        RichText {
            content_type: r.content_type,
            content: r.content,
        }
    }
}

impl<'de> Deserialize<'de> for RichText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = RichText;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("text or {type, content}")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> std::result::Result<RichText, E> {
                Ok(RichText::plain(s))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                map: A,
            ) -> std::result::Result<RichText, A::Error> {
                TextRepr::deserialize(de::value::MapAccessDeserializer::new(map)).map(Into::into)
            }
        }
        deserializer.deserialize_any(V)
    }
}

impl Serialize for RichText {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        if self.content_type == "text/plain" {
            serializer.serialize_str(&self.content)
        } else {
            TextRepr {
                content_type: self.content_type.clone(),
                content: self.content.clone(),
            }
            .serialize(serializer)
        }
    }
}

inline_schema!(RichText, "RichText", {
    "anyOf": [
        {"type": "string"},
        {"type": "object", "properties": {"type": {"type": "string"}, "content": {"type": "string"}}, "required": ["content"]}
    ]
});

// ---- text

/// Text such as Markdown. In JSON (and so over MCP) the string is the text itself: no file is
/// ever read. On the command line also `@FILE` (the file's content), `@-` (standard input) or
/// `@@...` (text that starts with `@`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TextArg(pub String);

impl FromStr for TextArg {
    type Err = String;
    /// The text, `@FILE`, `@-` or `@@...` (on the command line). A file or standard input is
    /// UTF-8, or UTF-16 with a byte order mark (what `>` writes in Windows PowerShell 5.1); a
    /// byte order mark is dropped.
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        let Some(rest) = s.strip_prefix('@') else {
            return Ok(TextArg(s.to_owned()));
        };
        if rest.starts_with('@') {
            return Ok(TextArg(rest.to_owned()));
        }
        let bytes = match rest {
            "" => {
                return Err(
                    "`@` needs a file name: `@FILE`, `@-` for standard input, `@@...` for text \
                     that starts with `@`"
                        .into(),
                );
            }
            "-" => {
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut std::io::stdin(), &mut bytes)
                    .map_err(|e| format!("cannot read standard input: {e}"))?;
                return decode(bytes)
                    .map(TextArg)
                    .map_err(|e| format!("cannot read standard input: it {e}"));
            }
            path => std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?,
        };
        decode(bytes)
            .map(TextArg)
            .map_err(|e| format!("cannot read {rest}: the file {e}"))
    }
}

/// Text from a file: UTF-8 (a byte order mark dropped) or UTF-16 with a byte order mark.
fn decode(bytes: Vec<u8>) -> std::result::Result<String, &'static str> {
    fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> std::result::Result<String, &'static str> {
        if !bytes.len().is_multiple_of(2) {
            return Err("is not valid UTF-16");
        }
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| unit(pair))
            .collect();
        String::from_utf16(&units).map_err(|_| "is not valid UTF-16")
    }
    match bytes.as_slice() {
        [0xff, 0xfe, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xfe, 0xff, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xef, 0xbb, 0xbf, rest @ ..] => {
            String::from_utf8(rest.to_vec()).map_err(|_| "is not valid UTF-8")
        }
        _ => String::from_utf8(bytes).map_err(|_| "is not valid UTF-8"),
    }
}

inline_schema!(TextArg, "TextArg", {"type": "string"});

// ---- field selections

/// Which fields to fetch: `standard`, `all`, or a list of field keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldSelection {
    Standard,
    All,
    Keys(Vec<String>),
}

impl FieldSelection {
    /// The selection plus extra custom field keys; `None` when neither is given.
    pub fn fields(selection: Option<&FieldSelection>, custom: &[String]) -> Option<Fields> {
        let base = selection.map(|s| match s {
            FieldSelection::Standard => Fields::standard(),
            FieldSelection::All => Fields::all(),
            FieldSelection::Keys(keys) => Fields::new(keys.clone()),
        });
        if custom.is_empty() {
            return base;
        }
        Some(
            custom
                .iter()
                .fold(base.unwrap_or_default(), |f, key| f.with_custom(key)),
        )
    }
}

impl FromStr for FieldSelection {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        Ok(match s.trim() {
            "standard" => FieldSelection::Standard,
            "all" => FieldSelection::All,
            other => FieldSelection::Keys(crate::config::split_list(other)),
        })
    }
}

impl fmt::Display for FieldSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldSelection::Standard => f.write_str("standard"),
            FieldSelection::All => f.write_str("all"),
            FieldSelection::Keys(keys) => f.write_str(&keys.join(",")),
        }
    }
}

impl<'de> Deserialize<'de> for FieldSelection {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = FieldSelection;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("`standard`, `all`, or a list of field keys")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> std::result::Result<FieldSelection, E> {
                s.parse().map_err(E::custom)
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<FieldSelection, A::Error> {
                let mut keys = Vec::new();
                while let Some(key) = seq.next_element::<String>()? {
                    keys.push(key);
                }
                Ok(FieldSelection::Keys(keys))
            }
        }
        deserializer.deserialize_any(V)
    }
}

impl Serialize for FieldSelection {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            FieldSelection::Keys(keys) => keys.serialize(serializer),
            other => serializer.collect_str(other),
        }
    }
}

inline_schema!(FieldSelection, "FieldSelection", {
    "anyOf": [
        {"type": "string", "description": "`standard`, `all`, or comma-separated field keys"},
        {"type": "array", "items": {"type": "string"}}
    ]
});

// ---- binary data

/// Binary content: base64 in JSON; on the command line base64 or `@FILE`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Data(pub Vec<u8>);

impl FromStr for Data {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        if let Some(path) = s.strip_prefix('@') {
            return std::fs::read(path)
                .map(Data)
                .map_err(|e| format!("cannot read {path}: {e}"));
        }
        let compact: String = s.split_whitespace().collect();
        BASE64
            .decode(compact)
            .map(Data)
            .map_err(|e| format!("invalid base64: {e}"))
    }
}

impl<'de> Deserialize<'de> for Data {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        let compact: String = s.split_whitespace().collect();
        BASE64
            .decode(compact)
            .map(Data)
            .map_err(|e| de::Error::custom(format!("invalid base64: {e}")))
    }
}

impl Serialize for Data {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&BASE64.encode(&self.0))
    }
}

inline_schema!(Data, "Data", {"type": "string", "contentEncoding": "base64"});

// ---- dynamic JSON

/// A JSON object whose shape is only known at run time (custom field values, test parameters),
/// typed later against Polarion's field definitions. On the command line: JSON text.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(transparent)]
pub struct JsonObject(pub Map<String, Value>);

impl FromStr for JsonObject {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match serde_json::from_str(s) {
            Ok(Value::Object(map)) => Ok(JsonObject(map)),
            _ => Err(format!("expected a JSON object, got `{s}`")),
        }
    }
}

impl<'de> Deserialize<'de> for JsonObject {
    /// An object, or a string holding one.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::Object(map) => Ok(JsonObject(map)),
            Value::String(s) => s.parse().map_err(de::Error::custom),
            other => Err(de::Error::custom(format!(
                "expected a JSON object, got {other}"
            ))),
        }
    }
}

inline_schema!(JsonObject, "JsonObject", {"type": "object"});

/// Any JSON value (a custom field value; `null` clears). On the command line: JSON text, or a
/// plain string.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonValue(pub Value);

impl FromStr for JsonValue {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        Ok(JsonValue(
            serde_json::from_str(s).unwrap_or_else(|_| Value::from(s)),
        ))
    }
}

inline_schema!(JsonValue, "JsonValue", {});

// ---- queries

/// Search options shared by the query commands.
#[cmd_args]
pub struct QueryArgs {
    /// Lucene query, e.g. `type:requirement AND status:open`; empty matches everything
    #[positional]
    pub query: Option<String>,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Sort order, e.g. `id`, `~updated` (descending)
    pub sort: Option<String>,
    /// Maximum number of results (default 100)
    pub limit: Option<u32>,
    /// Return every match (no limit)
    pub all: bool,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
    /// Custom field keys to fetch in addition
    pub custom: Vec<String>,
    /// Search as of this baseline revision
    pub baseline: Option<String>,
    /// Return work items as they were in this revision
    pub revision: Option<String>,
}

impl QueryArgs {
    pub fn query(&self, cx: &Context<'_>) -> Query {
        let mut q = Query::new(self.query.clone().unwrap_or_default());
        if let Some(p) = cx.opt_project(self.project.as_deref()) {
            q = q.project(p);
        }
        if let Some(sort) = &self.sort {
            q = q.sort(sort.clone());
        }
        if self.all {
            q = q.unlimited();
        } else if let Some(limit) = self.limit {
            q = q.limit(limit);
        }
        if let Some(fields) = FieldSelection::fields(self.fields.as_ref(), &self.custom) {
            q = q.fields(fields);
        }
        if let Some(b) = &self.baseline {
            q = q.baseline(b.clone());
        }
        if let Some(r) = &self.revision {
            q = q.revision(r.clone());
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn references_parse_from_strings_and_objects() {
        assert_eq!("P-1".parse(), Ok(ItemArg::Id("P-1".into())));
        assert_eq!(
            "Q/Q-2".parse(),
            Ok(ItemArg::Qualified {
                project: "Q".into(),
                id: "Q-2".into()
            })
        );
        assert!(matches!("subterra:x".parse(), Ok(ItemArg::Uri(_))));
        assert!("a/b/c".parse::<ItemArg>().is_err());
        let obj: ItemArg = serde_json::from_value(json!({"project": "S", "id": "S-4"})).unwrap();
        assert_eq!(obj.to_string(), "S/S-4");
        let doc: DocumentArg = serde_json::from_value(json!("Specs/SRS")).unwrap();
        assert_eq!(
            doc,
            DocumentArg::Short {
                space: "Specs".into(),
                name: "SRS".into()
            }
        );
        assert!("Q/Specs/SRS".parse::<DocumentArg>().is_ok());
        assert_eq!(serde_json::to_value(&doc).unwrap(), json!("Specs/SRS"));
    }

    #[test]
    fn text_fields_and_data() {
        let html: RichText =
            serde_json::from_value(json!({"type": "text/html", "content": "<b>x</b>"})).unwrap();
        assert_eq!(html.content_type, "text/html");
        assert_eq!(
            "plain".parse::<RichText>().unwrap(),
            RichText::plain("plain")
        );
        assert_eq!(
            serde_json::from_value::<FieldSelection>(json!("all")).unwrap(),
            FieldSelection::All
        );
        assert_eq!(
            serde_json::from_value::<FieldSelection>(json!(["id", "title"])).unwrap(),
            FieldSelection::Keys(vec!["id".into(), "title".into()])
        );
        assert_eq!(
            "id, title".parse::<FieldSelection>().unwrap(),
            FieldSelection::Keys(vec!["id".into(), "title".into()])
        );
        assert_eq!(
            "aGVs\nbG8=".parse::<Data>().unwrap(),
            Data(b"hello".to_vec())
        );
        assert_eq!(
            serde_json::to_value(Data(b"hello".to_vec())).unwrap(),
            json!("aGVsbG8=")
        );
        let obj: JsonObject = serde_json::from_value(json!(r#"{"asil": "D"}"#)).unwrap();
        assert_eq!(obj.0["asil"], "D");
    }

    #[test]
    fn text_is_the_argument_or_a_file_on_the_command_line() {
        assert_eq!("# Title".parse(), Ok(TextArg("# Title".into())));
        assert_eq!("".parse(), Ok(TextArg(String::new())));
        assert_eq!("a @b".parse(), Ok(TextArg("a @b".into())));
        // `@@` stands for a literal `@`.
        assert_eq!("@@team".parse(), Ok(TextArg("@team".into())));
        assert_eq!("@@".parse(), Ok(TextArg("@".into())));
        let e = "@".parse::<TextArg>().unwrap_err();
        assert!(e.contains("file name"), "{e}");

        let file =
            std::env::temp_dir().join(format!("polarion-commands-text-{}.md", std::process::id()));
        let arg = format!("@{}", file.display());
        // The byte order mark some editors write is dropped; line endings stay.
        std::fs::write(&file, "\u{feff}# Title\r\n\r\nBody @x\n").unwrap();
        let read = arg.parse::<TextArg>();
        // UTF-16 with a byte order mark, as `>` writes it in Windows PowerShell 5.1.
        let utf16 = |bom: [u8; 2], unit: fn(u16) -> [u8; 2]| {
            let mut bytes = bom.to_vec();
            bytes.extend("- Grüße\r\n".encode_utf16().flat_map(unit));
            std::fs::write(&file, bytes).unwrap();
            arg.parse::<TextArg>()
        };
        let little_endian = utf16([0xff, 0xfe], u16::to_le_bytes);
        let big_endian = utf16([0xfe, 0xff], u16::to_be_bytes);
        std::fs::write(&file, b"\xff\xfe#").unwrap();
        let odd_utf16 = arg.parse::<TextArg>();
        std::fs::write(&file, b"#\xff\0").unwrap();
        let not_utf8 = arg.parse::<TextArg>();
        std::fs::remove_file(&file).unwrap();
        let missing = arg.parse::<TextArg>();
        assert_eq!(read, Ok(TextArg("# Title\r\n\r\nBody @x\n".into())));
        for decoded in [little_endian, big_endian] {
            assert_eq!(decoded, Ok(TextArg("- Grüße\r\n".into())));
        }
        assert_eq!(
            odd_utf16.unwrap_err(),
            format!(
                "cannot read {}: the file is not valid UTF-16",
                file.display()
            )
        );
        assert_eq!(
            not_utf8.unwrap_err(),
            format!(
                "cannot read {}: the file is not valid UTF-8",
                file.display()
            )
        );
        let e = missing.unwrap_err();
        assert!(
            e.starts_with(&format!("cannot read {}: ", file.display())),
            "{e}"
        );
    }

    #[test]
    fn text_in_json_is_the_text_itself() {
        for text in ["@desc.md", "@-", "@@x", "", "line 1\nline 2"] {
            let arg: TextArg = serde_json::from_value(json!(text)).unwrap();
            assert_eq!(arg, TextArg(text.into()));
            assert_eq!(serde_json::to_value(&arg).unwrap(), json!(text));
        }
        assert!(serde_json::from_value::<TextArg>(json!({"content": "x"})).is_err());
        let schema = TextArg::json_schema(&mut SchemaGenerator::default());
        assert_eq!(
            serde_json::to_value(schema).unwrap(),
            json!({"type": "string"})
        );
    }

    #[test]
    fn follow_ups_parse_from_strings_and_objects() {
        // Links: the role up to the first `=`, a revision after the last `@`.
        let link: LinkArg = "parent=Sandbox/SB-1".parse().unwrap();
        assert_eq!(
            link,
            LinkArg {
                role: "parent".into(),
                item: ItemArg::Qualified {
                    project: "Sandbox".into(),
                    id: "SB-1".into()
                },
                revision: None,
                suspect: false,
            }
        );
        let uri = "subterra:data-service:objects:/default/P${WorkItem}P-1";
        let pinned: LinkArg = format!("relates_to={uri}@42").parse().unwrap();
        assert_eq!(pinned.item, ItemArg::Uri(uri.into()));
        assert_eq!(pinned.revision.as_deref(), Some("42"));
        let suspect: LinkArg =
            serde_json::from_value(json!({"role": "parent", "item": "P-1", "suspect": true}))
                .unwrap();
        assert!(suspect.suspect);
        // `suspect` has no string form: it serializes as an object, which reads back the same.
        let value = serde_json::to_value(&suspect).unwrap();
        assert_eq!(
            value,
            json!({"role": "parent", "item": "P-1", "suspect": true})
        );
        assert_eq!(serde_json::from_value::<LinkArg>(value).unwrap(), suspect);
        assert_eq!(
            serde_json::to_value(&pinned).unwrap(),
            json!(format!("relates_to={uri}@42"))
        );
        for bad in ["P-1", "=P-1", "parent=", "https://x/y=P-1"] {
            assert!(bad.parse::<LinkArg>().is_err(), "{bad}");
        }
        assert!(
            serde_json::from_value::<LinkArg>(json!({"role": "r", "item": "P-1", "x": 1})).is_err()
        );

        // Hyperlinks: the URL may contain `=`; a bare URL is not a role.
        let link: HyperlinkArg = "ref_ext=https://x.example/?a=b".parse().unwrap();
        assert_eq!(
            (link.role.as_str(), link.url.as_str()),
            ("ref_ext", "https://x.example/?a=b")
        );
        assert_eq!(link.to_string(), "ref_ext=https://x.example/?a=b");
        assert!("https://x.example/?a=b".parse::<HyperlinkArg>().is_err());
        let object: HyperlinkArg =
            serde_json::from_value(json!({"role": "ref_ext", "url": "https://x"})).unwrap();
        assert_eq!(object.url, "https://x");

        // OSLC links: the label before the URL.
        let oslc: OslcLinkArg = "relates_to=R1=https://o.example/r?id=1".parse().unwrap();
        assert_eq!(
            (oslc.role.as_str(), oslc.label.as_str(), oslc.url.as_str()),
            ("relates_to", "R1", "https://o.example/r?id=1")
        );
        assert_eq!(oslc.to_string().parse::<OslcLinkArg>().unwrap(), oslc);
        assert!("relates_to=https://o".parse::<OslcLinkArg>().is_err());
        assert!(
            serde_json::from_value::<OslcLinkArg>(json!({"role": "r", "label": "a=b", "url": "u"}))
                .is_err()
        );

        // Planning constraints and revisions.
        let c: ConstraintArg = "startNoEarlierThan=2024-01-31".parse().unwrap();
        assert_eq!(
            (c.constraint.as_str(), c.date.as_str()),
            ("startNoEarlierThan", "2024-01-31")
        );
        assert!("2024-01-31".parse::<ConstraintArg>().is_err());
        let plain: RevisionArg = "1234".parse().unwrap();
        assert_eq!((plain.repository, plain.revision.as_str()), (None, "1234"));
        let other: RevisionArg = "ext=abc".parse().unwrap();
        assert_eq!(
            (other.repository.as_deref(), other.revision.as_str()),
            (Some("ext"), "abc")
        );
        assert_eq!(other.to_string(), "ext=abc");
        assert!("".parse::<RevisionArg>().is_err());

        // Plans: a URI, an id or PROJECT/ID.
        for good in [
            "rel1",
            "P/rel1",
            "subterra:data-service:objects:/default/P${Plan}rel1",
        ] {
            assert_eq!(good.parse::<PlanArg>().unwrap().to_string(), good);
        }
        assert!("a/b/c".parse::<PlanArg>().is_err());
        assert!("".parse::<PlanArg>().is_err());
    }
}
