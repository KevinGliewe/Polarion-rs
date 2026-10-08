//! Typed arguments: references, rich text, field selections, binary data and the deliberately
//! dynamic JSON values. Each deserializes from the forms a person or an agent would write, parses
//! from a command-line string, and describes itself as JSON Schema.

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
/// `Repr`, then converted).
macro_rules! string_or_object {
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
}
