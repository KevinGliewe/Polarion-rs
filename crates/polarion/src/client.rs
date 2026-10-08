use std::collections::{BTreeSet, VecDeque};

use polarion_soap::Auth;

use crate::convert;
use crate::error::{Error, Result};
use crate::model::{Document, Link, Project, User, WorkItem};
use crate::query::{Fields, WorkItemQuery, term};

/// Module fields fetched when listing documents (everything but the body).
const DOCUMENT_LIST_FIELDS: &[&str] = &[
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

/// Convenient access to a Polarion server, built on the SOAP web services.
///
/// Cheap to clone; clones share the session. The session is opened on the
/// first call; see [`polarion_soap::Client`] for the details.
#[derive(Debug, Clone)]
pub struct Polarion {
    soap: polarion_soap::Client,
}

impl Polarion {
    /// Connect to `base_url` (e.g. `https://polarion.example.com/polarion`) with a **read-only** client:
    /// operations that may modify data are refused before anything is sent.
    ///
    /// For other settings (writes, TLS, timeouts) build a [`polarion_soap::Client`] and use
    /// [`Polarion::from_client`].
    pub fn new(base_url: &str, auth: Auth) -> Result<Self> {
        let soap = polarion_soap::Client::builder(base_url, auth)
            .read_only(true)
            .build()?;
        Ok(Self { soap })
    }

    pub fn from_client(soap: polarion_soap::Client) -> Self {
        Self { soap }
    }

    /// The underlying SOAP client, for everything this library does not cover.
    pub fn soap(&self) -> &polarion_soap::Client {
        &self.soap
    }

    /// End the session on the server.
    pub async fn logout(&self) -> Result<()> {
        Ok(self.soap.logout().await?)
    }

    // ---- projects and users ----

    pub async fn project(&self, id: &str) -> Result<Project> {
        let p = self.soap.project().get_project(id).await?;
        if p.unresolvable == Some(true) || (p.id.is_none() && p.uri.is_none()) {
            return Err(not_found("project", id));
        }
        Ok(p.into())
    }

    /// All projects the user can see, sorted by id. Walks the project group tree
    /// (two requests per group).
    pub async fn projects(&self) -> Result<Vec<Project>> {
        let service = self.soap.project();
        let root = service.get_root_project_group().await?;
        let mut queue: VecDeque<String> = root.uri.into_iter().collect();
        let mut seen = BTreeSet::new();
        let mut projects: Vec<Project> = Vec::new();
        while let Some(group) = queue.pop_front() {
            if !seen.insert(group.clone()) {
                continue;
            }
            projects.extend(
                service
                    .get_contained_projects(&group)
                    .await?
                    .into_iter()
                    .filter(|p| p.unresolvable != Some(true))
                    .map(Project::from),
            );
            queue.extend(
                service
                    .get_contained_groups(&group)
                    .await?
                    .into_iter()
                    .filter_map(|g| g.uri),
            );
        }
        projects.sort_by(|a, b| a.id.cmp(&b.id));
        projects.dedup_by(|a, b| a.id == b.id);
        Ok(projects)
    }

    pub async fn user(&self, id: &str) -> Result<User> {
        let u = self.soap.project().get_user(id).await?;
        if u.unresolvable == Some(true) || (u.id.is_none() && u.uri.is_none()) {
            return Err(not_found("user", id));
        }
        Ok(u.into())
    }

    // ---- work items ----

    /// One work item with all fields.
    pub async fn work_item(&self, project: &str, id: &str) -> Result<WorkItem> {
        let w = self.soap.tracker().get_work_item_by_id(project, id).await?;
        if w.unresolvable == Some(true) || (w.id.is_none() && w.uri.is_none()) {
            return Err(not_found("work item", &format!("{project}/{id}")));
        }
        Ok(w.into())
    }

    pub async fn query_work_items(&self, query: &WorkItemQuery) -> Result<Vec<WorkItem>> {
        let items = self
            .soap
            .tracker()
            .query_work_items_limited(
                &query.lucene(),
                query.sort_key(),
                query.field_keys(),
                query.results_limit(),
            )
            .await?;
        Ok(items.into_iter().map(WorkItem::from).collect())
    }

    /// Number of work items matching the query (sort, limit and fields are ignored).
    pub async fn count_work_items(&self, query: &WorkItemQuery) -> Result<u32> {
        let count = self
            .soap
            .tracker()
            .get_work_items_count(&query.lucene())
            .await?;
        Ok(u32::try_from(count).unwrap_or(0))
    }

    /// Links from other work items to this one ([`WorkItem::uri`]); each link's `item` is the source.
    pub async fn back_links(&self, work_item_uri: &str) -> Result<Vec<Link>> {
        let links = self
            .soap
            .tracker()
            .get_back_linked_workitems(work_item_uri)
            .await?;
        Ok(links.into_iter().filter_map(convert::link).collect())
    }

    // ---- documents ----

    /// Document space ids of a project (`_default` is the default space).
    pub async fn spaces(&self, project: &str) -> Result<Vec<String>> {
        Ok(self.soap.tracker().get_document_spaces(project).await?)
    }

    /// Documents of a project, or of one space, sorted by space and name. Bodies are not fetched.
    pub async fn documents(&self, project: &str, space: Option<&str>) -> Result<Vec<Document>> {
        let fields = document_list_fields();
        let tracker = self.soap.tracker();
        let modules = match space {
            Some(space) => {
                tracker
                    .get_modules_with_fields(project, space, &fields)
                    .await?
            }
            None => {
                let query = format!("project.id:{}", term(project));
                tracker.query_modules(&query, "title", &fields, -1).await?
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
    /// otherwise the same fields as [`Polarion::documents`].
    pub async fn document(
        &self,
        project: &str,
        space: &str,
        name: &str,
        with_body: bool,
    ) -> Result<Document> {
        let location = format!("{space}/{name}");
        let tracker = self.soap.tracker();
        let m = if with_body {
            tracker.get_module_by_location(project, &location).await?
        } else {
            tracker
                .get_module_by_location_with_fields(project, &location, &document_list_fields())
                .await?
        };
        if m.unresolvable == Some(true) || (m.uri.is_none() && m.title.is_none()) {
            return Err(not_found("document", &format!("{project}/{location}")));
        }
        Ok(m.into())
    }

    /// The work items of a document ([`Document::uri`]), in document order.
    pub async fn document_work_items(
        &self,
        document_uri: &str,
        fields: &Fields,
    ) -> Result<Vec<WorkItem>> {
        let items = self
            .soap
            .tracker()
            .get_module_work_items(document_uri, None, true, fields.keys())
            .await?;
        Ok(items.into_iter().map(WorkItem::from).collect())
    }
}

fn document_list_fields() -> Vec<String> {
    DOCUMENT_LIST_FIELDS.iter().map(|&f| f.into()).collect()
}

fn not_found(kind: &'static str, id: &str) -> Error {
    Error::NotFound {
        kind,
        id: id.to_owned(),
    }
}
