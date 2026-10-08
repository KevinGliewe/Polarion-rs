//! Documents (called modules in the SOAP API).

use crate::client::{Polarion, not_found};
use crate::error::Result;
use crate::model::{Document, WorkItem};
use crate::query::{Fields, term};
use crate::uri::DocumentRef;

/// Module fields fetched when listing documents (everything but the body).
const LIST_FIELDS: &[&str] = &[
    "id",
    "title",
    "moduleFolder",
    "moduleName",
    "type",
    "status",
    "author",
    "created",
    "updated",
    "project",
];

fn list_fields() -> Vec<String> {
    LIST_FIELDS.iter().map(|&f| f.into()).collect()
}

/// Documents: [`Polarion::documents`].
#[derive(Debug, Clone, Copy)]
pub struct Documents<'a> {
    polarion: &'a Polarion,
}

impl<'a> Documents<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    /// Document space ids of a project (`_default` is the default space).
    pub async fn spaces(&self, project: &str) -> Result<Vec<String>> {
        Ok(self
            .polarion
            .soap()
            .tracker()
            .get_document_spaces(project)
            .await?)
    }

    /// Documents of a project, or of one space, sorted by space and name. Bodies are not fetched.
    pub async fn list(&self, project: &str, space: Option<&str>) -> Result<Vec<Document>> {
        let fields = list_fields();
        let tracker = self.polarion.soap().tracker();
        let modules = match space {
            Some(space) => {
                tracker
                    .get_modules_with_fields(project, space, &fields)
                    .await?
            }
            None => {
                let query = format!("project.id:{}", term(project));
                tracker
                    .query_modules(Some(&query), "title", &fields, -1)
                    .await?
            }
        };
        let mut docs: Vec<Document> = modules
            .into_iter()
            .filter(|m| m.unresolvable != Some(true))
            .map(Document::from)
            .collect();
        docs.sort_by(|a, b| (&a.space, &a.name).cmp(&(&b.space, &b.name)));
        Ok(docs)
    }

    /// One document. With `with_body`, all fields including the body (which can be large);
    /// otherwise the same fields as [`Documents::list`].
    pub async fn get(
        &self,
        project: &str,
        space: &str,
        name: &str,
        with_body: bool,
    ) -> Result<Document> {
        let location = format!("{space}/{name}");
        let tracker = self.polarion.soap().tracker();
        let m = if with_body {
            tracker.get_module_by_location(project, &location).await?
        } else {
            tracker
                .get_module_by_location_with_fields(project, &location, &list_fields())
                .await?
        };
        if m.unresolvable == Some(true) || (m.uri.is_none() && m.title.is_none()) {
            return Err(not_found("document", format!("{project}/{location}")));
        }
        Ok(m.into())
    }

    /// The work items of a document, in document order.
    pub async fn work_items(
        &self,
        document: &DocumentRef,
        fields: &Fields,
    ) -> Result<Vec<WorkItem>> {
        let items = self
            .polarion
            .soap()
            .tracker()
            .get_module_work_items(&document.uri, None, true, fields.keys())
            .await?;
        Ok(items.into_iter().map(WorkItem::from).collect())
    }
}
