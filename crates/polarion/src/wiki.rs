//! Wiki pages.

use polarion_soap::services::TrackerService;
use polarion_soap::types as soap;

use crate::client::{Polarion, not_found};
use crate::error::Result;
use crate::items::baseline_and_revision;
use crate::model::WikiPage;
use crate::query::{Fields, Query};

/// Fields fetched by queries without [`Query::fields`]: everything but the page content.
const DEFAULT_FIELDS: &[&str] = &[
    "attachments",
    "author",
    "created",
    "id",
    "linkedPageURIs",
    "location",
    "pageLocation",
    "pageName",
    "project",
    "spaceId",
    "title",
    "type",
    "updated",
    "updatedBy",
];

fn default_fields() -> Vec<String> {
    DEFAULT_FIELDS.iter().map(|&f| f.into()).collect()
}

fn pages(list: Vec<soap::WikiPage>) -> Vec<WikiPage> {
    list.into_iter()
        .filter(|p| p.unresolvable != Some(true))
        .map(WikiPage::from)
        .collect()
}

/// Wiki pages: [`Polarion::wiki`]. Pages are addressed by URI ([`WikiPage::uri`]).
#[derive(Debug, Clone, Copy)]
pub struct Wiki<'a> {
    polarion: &'a Polarion,
}

impl<'a> Wiki<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    fn tracker(&self) -> TrackerService<'a> {
        self.polarion.soap().tracker()
    }

    pub async fn spaces(&self, project: &str) -> Result<Vec<String>> {
        Ok(self.tracker().get_wiki_spaces(project).await?)
    }

    /// The pages of a space: all fields (`fields` `None`) or only `fields`.
    pub async fn pages(
        &self,
        project: &str,
        space: &str,
        fields: Option<&Fields>,
    ) -> Result<Vec<WikiPage>> {
        let t = self.tracker();
        Ok(pages(match fields {
            None => t.get_wiki_pages(project, space).await?,
            Some(f) => {
                t.get_wiki_pages_with_fields(project, space, f.keys())
                    .await?
            }
        }))
    }

    /// URIs of the pages of a space.
    pub async fn page_uris(&self, project: &str, space: &str) -> Result<Vec<String>> {
        Ok(self.tracker().get_wiki_page_uris(project, space).await?)
    }

    /// One page by URI: all fields (`fields` `None`) or only `fields`.
    pub async fn get(&self, uri: &str, fields: Option<&Fields>) -> Result<WikiPage> {
        let t = self.tracker();
        let page = match fields {
            None => t.get_wiki_page_by_uri(uri).await?,
            Some(f) => t.get_wiki_page_by_uri_with_fields(uri, f.keys()).await?,
        };
        if page.unresolvable == Some(true) || (page.uri.is_none() && page.page_name.is_none()) {
            return Err(not_found("wiki page", uri));
        }
        Ok(page.into())
    }

    /// Pages matching a Lucene query, optionally in a baseline. Without [`Query::fields`]
    /// every field except the page content is fetched.
    pub async fn query(&self, query: &Query) -> Result<Vec<WikiPage>> {
        if query.revision_value().is_some() {
            return Err(baseline_and_revision());
        }
        let default = default_fields();
        let fields = query.field_list().map_or(default.as_slice(), Fields::keys);
        let (q, sort, limit) = (query.lucene_opt(), query.sort_key(), query.results_limit());
        let t = self.tracker();
        Ok(pages(match query.baseline_revision() {
            None => {
                t.query_wiki_pages(q.as_deref(), sort, fields, limit)
                    .await?
            }
            Some(b) => {
                t.query_wiki_pages_in_baseline(q.as_deref(), sort, b, fields, limit)
                    .await?
            }
        }))
    }

    /// URIs of the pages matching a Lucene query, optionally in a baseline.
    pub async fn query_uris(&self, query: &Query) -> Result<Vec<String>> {
        let (q, sort, limit) = (query.lucene_opt(), query.sort_key(), query.results_limit());
        let t = self.tracker();
        Ok(match query.baseline_revision() {
            None => t.query_wiki_page_uris(q.as_deref(), sort, limit).await?,
            Some(b) => {
                t.query_wiki_page_uris_in_baseline(q.as_deref(), sort, b, limit)
                    .await?
            }
        })
    }

    /// Pages selected by SQL, optionally in a baseline (fields as in [`Wiki::query`]).
    pub async fn query_sql(
        &self,
        sql: &str,
        fields: Option<&Fields>,
        baseline: Option<&str>,
    ) -> Result<Vec<WikiPage>> {
        let default = default_fields();
        let fields = fields.map_or(default.as_slice(), Fields::keys);
        let t = self.tracker();
        Ok(pages(match baseline {
            None => t.query_wiki_pages_by_sql(sql, fields).await?,
            Some(b) => {
                t.query_wiki_pages_in_baseline_by_sql(sql, b, fields)
                    .await?
            }
        }))
    }

    /// URIs of the pages selected by SQL, optionally in a baseline.
    pub async fn query_uris_sql(&self, sql: &str, baseline: Option<&str>) -> Result<Vec<String>> {
        let t = self.tracker();
        Ok(match baseline {
            None => t.query_wiki_page_uris_by_sql(sql).await?,
            Some(b) => t.query_wiki_page_uris_in_baseline_by_sql(sql, b).await?,
        })
    }
}
