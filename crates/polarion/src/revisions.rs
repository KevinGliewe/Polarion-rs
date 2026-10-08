//! Source repository revisions.

use crate::client::{Polarion, not_found};
use crate::error::Result;
use crate::model::{Revision, WorkItem};
use crate::query::{Fields, Query};

/// Fields fetched by queries without [`Query::fields`]: all of them.
const DEFAULT_FIELDS: &[&str] = &[
    "author",
    "branch",
    "created",
    "internalCommit",
    "linkedWorkItemURIs",
    "message",
    "name",
    "repositoryName",
];

/// Revisions: [`Polarion::revisions`].
#[derive(Debug, Clone, Copy)]
pub struct Revisions<'a> {
    polarion: &'a Polarion,
}

impl<'a> Revisions<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    /// A revision of a repository (`default` for the default repository).
    pub async fn get(&self, repository: &str, revision: &str) -> Result<Revision> {
        let r = self
            .polarion
            .soap()
            .tracker()
            .get_revision(repository, revision)
            .await?;
        if r.unresolvable == Some(true) || (r.uri.is_none() && r.name.is_none()) {
            return Err(not_found("revision", format!("{repository}/{revision}")));
        }
        Ok(r.into())
    }

    pub async fn get_by_uri(&self, uri: &str) -> Result<Revision> {
        let r = self
            .polarion
            .soap()
            .tracker()
            .get_revision_by_uri(uri)
            .await?;
        if r.unresolvable == Some(true) || (r.uri.is_none() && r.name.is_none()) {
            return Err(not_found("revision", uri));
        }
        Ok(r.into())
    }

    /// Revisions matching a Lucene query (limit and baseline are ignored; fields default to all).
    pub async fn query(&self, query: &Query) -> Result<Vec<Revision>> {
        let default: Vec<String> = DEFAULT_FIELDS.iter().map(|&f| f.into()).collect();
        let fields = query.field_list().map_or(default.as_slice(), Fields::keys);
        let list = self
            .polarion
            .soap()
            .tracker()
            .query_revisions(&query.lucene(), query.sort_key(), fields)
            .await?;
        Ok(list.into_iter().map(Revision::from).collect())
    }

    /// URIs of the revisions matching a Lucene query, with or without Polarion's own commits.
    pub async fn query_uris(&self, query: &Query, include_internal: bool) -> Result<Vec<String>> {
        Ok(self
            .polarion
            .soap()
            .tracker()
            .query_revision_uris(&query.lucene(), query.sort_key(), include_internal)
            .await?)
    }

    /// Work items linked to a revision: all fields (`fields` `None`) or only `fields`.
    pub async fn work_items(
        &self,
        revision_uri: &str,
        fields: Option<&Fields>,
    ) -> Result<Vec<WorkItem>> {
        let t = self.polarion.soap().tracker();
        let list = match fields {
            None => t.get_work_items_linked_to_revision(revision_uri).await?,
            Some(f) => {
                t.get_work_items_linked_to_revision_with_fields(revision_uri, f.keys())
                    .await?
            }
        };
        Ok(list.into_iter().map(WorkItem::from).collect())
    }
}
