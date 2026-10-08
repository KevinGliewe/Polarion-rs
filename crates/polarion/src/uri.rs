//! References parsed from Polarion object URIs such as
//! `subterra:data-service:objects:/default/PROJ${WorkItem}PROJ-1`.
//!
//! Parsing is best effort: the raw URI is always kept, and a URI in an
//! unexpected format leaves the parsed parts empty instead of failing.

use serde::ser::{Serialize, SerializeStruct, Serializer};

const WORK_ITEM: &str = "${WorkItem}";
const MODULE: &str = "${Module}";
const MODULE_FOLDER: &str = "{moduleFolder}";
const USER: &str = "${User}";

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
    pub fn from_uri(uri: impl Into<String>) -> Self {
        let uri = uri.into();
        let parsed = split(&uri, MODULE).and_then(|(project, rest)| {
            let (space, name) = rest.strip_prefix(MODULE_FOLDER)?.split_once('/')?;
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

fn strip_revision(s: &str) -> &str {
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
        let r = DocumentRef::from_uri(format!(
            "{BASE}PROJ${{Module}}{{moduleFolder}}Specs/System Requirements"
        ));
        assert_eq!(r.project.as_deref(), Some("PROJ"));
        assert_eq!(r.space.as_deref(), Some("Specs"));
        assert_eq!(r.name.as_deref(), Some("System Requirements"));
        assert_eq!(r.location().as_deref(), Some("Specs/System Requirements"));

        let r = DocumentRef::from_uri(format!("{BASE}PROJ${{Module}}Specs"));
        assert_eq!((r.project, r.space, r.name), (None, None, None));
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
