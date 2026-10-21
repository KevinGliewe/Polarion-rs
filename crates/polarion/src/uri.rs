//! References parsed from Polarion object URIs such as
//! `subterra:data-service:objects:/default/PROJ${WorkItem}PROJ-1`.
//!
//! Parsing is best effort: the raw URI is always kept, and a URI in an
//! unexpected format leaves the parsed parts empty instead of failing.

use serde::ser::{Serialize, SerializeStruct, Serializer};

/// Prefix of object URIs in the default repository.
const OBJECTS: &str = "subterra:data-service:objects:/default/";
pub(crate) const WORK_ITEM: &str = "${WorkItem}";
pub(crate) const MODULE: &str = "${Module}";
const MODULE_FOLDER: &str = "{moduleFolder}";
const USER: &str = "${User}";
pub(crate) const TEST_RUN: &str = "${TestRun}";
const PROJECT: &str = "${Project}";

/// A work item, as referenced from links and other objects.
///
/// Serializes as `{"project": .., "id": ..}`, or as `{"uri": ..}` if the URI could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRef {
    pub project: Option<String>,
    pub id: Option<String>,
    pub uri: String,
}

impl ItemRef {
    /// A work item by project and id.
    ///
    /// Builds the URI `subterra:data-service:objects:/default/PROJECT${WorkItem}ID`, the format
    /// Polarion returns for work items of the default repository.
    pub fn new(project: impl Into<String>, id: impl Into<String>) -> Self {
        let (project, id) = (project.into(), id.into());
        Self {
            uri: format!("{OBJECTS}{project}{WORK_ITEM}{id}"),
            project: Some(project),
            id: Some(id),
        }
    }

    pub fn from_uri(uri: impl Into<String>) -> Self {
        let uri = uri.into();
        let (project, id) = match split(&uri, WORK_ITEM) {
            Some((p, id)) if !p.is_empty() && !id.is_empty() => {
                (Some(p.to_owned()), Some(id.to_owned()))
            }
            _ => (None, None),
        };
        Self { project, id, uri }
    }
}

/// `PROJECT/ID`, or the URI if it could not be parsed.
impl std::fmt::Display for ItemRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.project, &self.id) {
            (Some(project), Some(id)) => write!(f, "{project}/{id}"),
            _ => f.write_str(&self.uri),
        }
    }
}

impl Serialize for ItemRef {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match (&self.project, &self.id) {
            (Some(project), Some(id)) => {
                let mut st = s.serialize_struct("ItemRef", 2)?;
                st.serialize_field("project", project)?;
                st.serialize_field("id", id)?;
                st.end()
            }
            _ => {
                let mut st = s.serialize_struct("ItemRef", 1)?;
                st.serialize_field("uri", &self.uri)?;
                st.end()
            }
        }
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for ItemRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ItemRef".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "description": "A work item: `project` and `id`, or its `uri`",
            "properties": {
                "project": {"type": "string"},
                "id": {"type": "string"},
                "uri": {"type": "string"}
            }
        })
    }
}

/// A document (called "module" in the SOAP API), identified by project, space and name.
///
/// Serializes as `{"project": .., "space": .., "name": ..}`, or as `{"uri": ..}` if the URI could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRef {
    pub project: Option<String>,
    pub space: Option<String>,
    pub name: Option<String>,
    pub uri: String,
}

impl DocumentRef {
    /// A document by project, space and name.
    ///
    /// Builds the URI `subterra:data-service:objects:/default/PROJECT${Module}{moduleFolder}SPACE#NAME`,
    /// the format Polarion returns for documents (space and name are separated by `#`; the server
    /// cannot resolve `SPACE/NAME`).
    pub fn new(
        project: impl Into<String>,
        space: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        let (project, space, name) = (project.into(), space.into(), name.into());
        Self {
            uri: format!("{OBJECTS}{project}{MODULE}{MODULE_FOLDER}{space}#{name}"),
            project: Some(project),
            space: Some(space),
            name: Some(name),
        }
    }

    /// Parses `…/default/PROJECT${Module}{moduleFolder}SPACE#NAME` (also with `/` between space
    /// and name).
    pub fn from_uri(uri: impl Into<String>) -> Self {
        let uri = uri.into();
        let parsed = split(&uri, MODULE).and_then(|(project, rest)| {
            let rest = rest.strip_prefix(MODULE_FOLDER)?;
            let (space, name) = rest.split_once('#').or_else(|| rest.split_once('/'))?;
            (!project.is_empty() && !space.is_empty() && !name.is_empty())
                .then(|| (project.to_owned(), space.to_owned(), name.to_owned()))
        });
        match parsed {
            Some((project, space, name)) => Self {
                project: Some(project),
                space: Some(space),
                name: Some(name),
                uri,
            },
            None => Self {
                project: None,
                space: None,
                name: None,
                uri,
            },
        }
    }

    /// `space/name`, the location `getModuleByLocation` expects.
    pub fn location(&self) -> Option<String> {
        Some(format!("{}/{}", self.space.as_ref()?, self.name.as_ref()?))
    }
}

/// `PROJECT/SPACE/NAME`, or the URI if it could not be parsed.
impl std::fmt::Display for DocumentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.project, &self.space, &self.name) {
            (Some(project), Some(space), Some(name)) => write!(f, "{project}/{space}/{name}"),
            _ => f.write_str(&self.uri),
        }
    }
}

impl Serialize for DocumentRef {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match (&self.project, &self.space, &self.name) {
            (Some(project), Some(space), Some(name)) => {
                let mut st = s.serialize_struct("DocumentRef", 3)?;
                st.serialize_field("project", project)?;
                st.serialize_field("space", space)?;
                st.serialize_field("name", name)?;
                st.end()
            }
            _ => {
                let mut st = s.serialize_struct("DocumentRef", 1)?;
                st.serialize_field("uri", &self.uri)?;
                st.end()
            }
        }
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for DocumentRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DocumentRef".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "description": "A document: `project`, `space` and `name`, or its `uri`",
            "properties": {
                "project": {"type": "string"},
                "space": {"type": "string"},
                "name": {"type": "string"},
                "uri": {"type": "string"}
            }
        })
    }
}

/// The project of an object URI (`…/default/PROJ${TestRun}ID` → `PROJ`); `None` for
/// global objects (`…/default/${User}jdoe`) and URIs in other formats.
pub fn project_of(uri: &str) -> Option<&str> {
    let rest = &uri[uri.find(OBJECTS)? + OBJECTS.len()..];
    let project = &rest[..rest.find("${")?];
    (!project.is_empty() && !project.contains('/')).then_some(project)
}

/// The URI of a test run: `subterra:data-service:objects:/default/PROJECT${TestRun}ID`, the format
/// Polarion returns for test runs.
pub fn test_run_uri(project: &str, id: &str) -> String {
    format!("{OBJECTS}{project}{TEST_RUN}{id}")
}

/// The object that owns a comment or a work record, from the URI of that comment or work
/// record, in the formats Polarion uses (checked against a real server by the live read tier):
///
/// - a work item comment `…/default/PROJ${Comment}{WorkItem}PROJ-1#KEY` and a work record
///   `…/default/PROJ${WorkRecord}{WorkItem}PROJ-1#KEY` → `…/default/PROJ${WorkItem}PROJ-1`;
/// - a document comment `…/default/PROJ${ModuleComment}{Module}{moduleFolder}SPACE#NAME#KEY`
///   → `…/default/PROJ${Module}{moduleFolder}SPACE#NAME`.
///
/// A `%revision` suffix is dropped first. Strict: the work item id is ASCII letters, digits,
/// `_`, `-` and `.` and ends at the `#`; space and name contain no `#`, `$`, `{`, `}` or `%`;
/// the key is ASCII letters, digits, `_`, `-` and `.` (no `..`), so it can name no other object.
/// Every other format gives `None`.
pub fn owner_of(uri: &str) -> Option<String> {
    let uri = strip_revision(uri);
    let rest = uri.strip_prefix(OBJECTS)?;
    let marker = rest.find("${")?;
    let project = &rest[..marker];
    if !is_id(project) {
        return None;
    }
    let kind = &rest[marker..];
    if let Some(rest) = kind
        .strip_prefix("${Comment}{WorkItem}")
        .or_else(|| kind.strip_prefix("${WorkRecord}{WorkItem}"))
    {
        let (id, key) = rest.split_once('#')?;
        return (is_id(id) && is_key(key)).then(|| format!("{OBJECTS}{project}{WORK_ITEM}{id}"));
    }
    let rest = kind.strip_prefix("${ModuleComment}{Module}{moduleFolder}")?;
    let (space, rest) = rest.split_once('#')?;
    let (name, key) = rest.split_once('#')?;
    let plain = |s: &str| !s.is_empty() && !s.contains(['#', '$', '{', '}', '%']);
    (plain(space) && plain(name) && is_key(key))
        .then(|| format!("{OBJECTS}{project}{MODULE}{MODULE_FOLDER}{space}#{name}"))
}

/// The key of a comment or work record after its owner: ASCII letters, digits, `_`, `-` and
/// `.`, without `..`; never a second object marker (`${`), a path or a `%revision`.
fn is_key(key: &str) -> bool {
    is_id(key) && !key.contains("..")
}

/// Whether `uri` is the URI of `project` (`…/default/PROJ${Project}PROJ`).
pub(crate) fn is_project_uri(uri: &str, project: &str) -> bool {
    uri.strip_prefix(OBJECTS)
        .and_then(|rest| rest.strip_prefix(project))
        .and_then(|rest| rest.strip_prefix(PROJECT))
        .is_some_and(|id| id == project)
}

/// Whether `uri` is `…/default/PROJECT<marker>REST` for this project and object marker
/// (`${WorkItem}`, `${TestRun}`, `${Module}`), with a non-empty rest that holds no further
/// marker (`$`).
pub(crate) fn is_object_uri(uri: &str, project: &str, marker: &str) -> bool {
    uri.strip_prefix(OBJECTS)
        .and_then(|rest| rest.strip_prefix(project))
        .and_then(|rest| rest.strip_prefix(marker))
        .is_some_and(|rest| !rest.is_empty() && !rest.contains('$'))
}

/// A project id or a work item id: ASCII letters, digits, `_`, `-` and `.`.
fn is_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_id_char)
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// The URI of a user (`subterra:data-service:objects:/default/${User}ID`; format not yet
/// verified against a real server for writes).
pub fn user_uri(id: &str) -> String {
    format!("{OBJECTS}{USER}{id}")
}

/// The user id in a user URI (`…/default/${User}jdoe`).
pub fn user_id(uri: &str) -> Option<&str> {
    split(uri, USER)
        .map(|(_, id)| id)
        .filter(|id| !id.is_empty())
}

/// Split `…/default/PROJ${Kind}rest` into `("PROJ", "rest")`, dropping a `%revision` suffix.
fn split<'a>(uri: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
    let (left, right) = uri.split_once(marker)?;
    let project = left.rsplit('/').next().unwrap_or_default();
    Some((project, strip_revision(right)))
}

/// `s` without a `%revision` suffix.
pub(crate) fn strip_revision(s: &str) -> &str {
    match s.rsplit_once('%') {
        Some((head, rev)) if !rev.is_empty() && rev.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "subterra:data-service:objects:/default/";

    #[test]
    fn work_item_uris() {
        let r = ItemRef::from_uri(format!("{BASE}PROJ${{WorkItem}}PROJ-12"));
        assert_eq!(r.project.as_deref(), Some("PROJ"));
        assert_eq!(r.id.as_deref(), Some("PROJ-12"));

        let r = ItemRef::from_uri(format!("{BASE}PROJ${{WorkItem}}PROJ-12%4711"));
        assert_eq!(r.id.as_deref(), Some("PROJ-12"));

        let r = ItemRef::from_uri("something else");
        assert_eq!((r.project, r.id), (None, None));
        assert_eq!(r.uri, "something else");
    }

    #[test]
    fn document_uris() {
        // The form the server returns.
        let r = DocumentRef::from_uri(format!(
            "{BASE}PROJ${{Module}}{{moduleFolder}}Specs#System Requirements"
        ));
        assert_eq!(r.project.as_deref(), Some("PROJ"));
        assert_eq!(r.space.as_deref(), Some("Specs"));
        assert_eq!(r.name.as_deref(), Some("System Requirements"));
        assert_eq!(r.location().as_deref(), Some("Specs/System Requirements"));

        let r = DocumentRef::from_uri(format!("{BASE}PROJ${{Module}}{{moduleFolder}}Specs/SRS"));
        assert_eq!(
            (r.space.as_deref(), r.name.as_deref()),
            (Some("Specs"), Some("SRS"))
        );

        let r = DocumentRef::from_uri(format!("{BASE}PROJ${{Module}}Specs"));
        assert_eq!((r.project, r.space, r.name), (None, None, None));
    }

    #[test]
    fn built_refs_parse_back() {
        let r = ItemRef::new("P", "P-1");
        assert_eq!(r.uri, format!("{BASE}P${{WorkItem}}P-1"));
        assert_eq!(ItemRef::from_uri(r.uri.clone()), r);
        assert_eq!(r.to_string(), "P/P-1");

        let d = DocumentRef::new("P", "Specs", "SRS");
        assert_eq!(
            d.uri,
            format!("{BASE}P${{Module}}{{moduleFolder}}Specs#SRS")
        );
        assert_eq!(DocumentRef::from_uri(d.uri.clone()), d);
        assert_eq!(d.to_string(), "P/Specs/SRS");
    }

    #[test]
    fn projects_of_uris() {
        assert_eq!(project_of(&format!("{BASE}P${{TestRun}}R1")), Some("P"));
        assert_eq!(
            project_of(&format!("{BASE}P${{Module}}{{moduleFolder}}a/b")),
            Some("P")
        );
        assert_eq!(project_of(&format!("{BASE}${{User}}jdoe")), None);
        assert_eq!(project_of("junk"), None);
    }

    #[test]
    fn test_run_and_project_uris() {
        assert_eq!(test_run_uri("P", "R1"), format!("{BASE}P${{TestRun}}R1"));
        assert_eq!(project_of(&test_run_uri("P", "R1")), Some("P"));
        assert!(is_project_uri(&format!("{BASE}P${{Project}}P"), "P"));
        for other in [
            format!("{BASE}P${{Project}}Q"),
            format!("{BASE}Q${{Project}}P"),
            format!("{BASE}P${{WorkItem}}P"),
            format!("{BASE}P${{Project}}P/x"),
        ] {
            assert!(!is_project_uri(&other, "P"), "{other}");
        }
    }

    #[test]
    fn owners_of_comments_and_work_records() {
        let item = format!("{BASE}P${{WorkItem}}P-1");
        let item12 = format!("{BASE}P${{WorkItem}}P-12");
        let doc = format!("{BASE}P${{Module}}{{moduleFolder}}Specs - Old#System Requirements");
        let comment = |id: &str, key: &str| format!("{BASE}P${{Comment}}{{WorkItem}}{id}#{key}");
        let record = |id: &str, key: &str| format!("{BASE}P${{WorkRecord}}{{WorkItem}}{id}#{key}");
        let doc_comment = |space: &str, name: &str, key: &str| {
            format!("{BASE}P${{ModuleComment}}{{Module}}{{moduleFolder}}{space}#{name}#{key}")
        };
        for (owned, owner) in [
            // The formats the server returns.
            (comment("P-1", "1"), Some(item.clone())),
            (record("P-1", "2"), Some(item.clone())),
            (format!("{}%1234", comment("P-1", "1")), Some(item.clone())),
            (
                doc_comment("Specs - Old", "System Requirements", "1"),
                Some(doc.clone()),
            ),
            // The id ends at the `#`: `P-12` is never taken for `P-1`.
            (comment("P-12", "3"), Some(item12.clone())),
            // Objects themselves, missing keys, other kinds and repositories.
            (item.clone(), None),
            (doc.clone(), None),
            (comment("P-1", ""), None),
            (comment("", "1"), None),
            (doc_comment("Specs", "SRS", ""), None),
            (doc_comment("", "SRS", "1"), None),
            (doc_comment("Specs", "", "1"), None),
            (format!("{item}/5"), None),
            (format!("{item}#5"), None),
            (format!("{doc}#5"), None),
            (format!("{BASE}P${{Comment}}{{TestRun}}R1#1"), None),
            (format!("{BASE}${{Comment}}{{WorkItem}}P-1#1"), None),
            (
                "subterra:data-service:objects:/other/P${Comment}{WorkItem}P-1#1".to_owned(),
                None,
            ),
            // The key names nothing else: no second object, path, reference or revision.
            (comment("P-1", "1#2"), None),
            (comment("P-1", "1/../P-3"), None),
            (comment("P-1", "a..b"), None),
            (comment("P-1", "x${WorkItem}P-3"), None),
            (comment("P-1", "{x}"), None),
            (comment("P-1", "5 6"), None),
            (comment("P-1 x", "1"), None),
            (doc_comment("Specs", "SRS", "1#2"), None),
            (doc_comment("Specs", "SRS${WorkItem}P-3", "1"), None),
            (doc_comment("Sp{e}cs", "SRS", "1"), None),
            (format!("{BASE}P x${{Comment}}{{WorkItem}}P-1#1"), None),
            ("junk".to_owned(), None),
        ] {
            assert_eq!(owner_of(&owned), owner, "{owned}");
        }
    }

    #[test]
    fn user_uris() {
        assert_eq!(user_id(&format!("{BASE}${{User}}jdoe")), Some("jdoe"));
        assert_eq!(user_id("jdoe"), None);
    }

    #[test]
    fn serialization_falls_back_to_the_uri() {
        let ok = ItemRef::from_uri(format!("{BASE}P${{WorkItem}}P-1"));
        let bad = ItemRef::from_uri("x");
        assert_eq!(
            serde_json::to_string(&[ok, bad]).unwrap(),
            r#"[{"project":"P","id":"P-1"},{"uri":"x"}]"#
        );
    }
}
