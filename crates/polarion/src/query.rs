//! Work item queries and field selection.

/// Default number of results of a [`WorkItemQuery`].
pub const DEFAULT_LIMIT: u32 = 100;

/// Work item fields to fetch. Fields not listed are left empty in the results.
///
/// Keys are Polarion's field ids (`title`, `assignee`, `linkedWorkItems`, `module`, `plannedIn`, ...),
/// which differ from the SOAP element names in a few places: the server refuses `moduleURI` and
/// `plannedInURIs`. Custom fields are added with [`Fields::with_custom`] (`customFields.<key>`):
/// the key `customFields` alone returns none of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fields(Vec<String>);

/// The light set used by default: identity, state, people, dates and document position.
const STANDARD: &[&str] = &[
    "id",
    "title",
    "type",
    "status",
    "priority",
    "severity",
    "resolution",
    "author",
    "assignee",
    "created",
    "updated",
    "dueDate",
    "outlineNumber",
    "project",
    "module",
];

/// Every work item field the SOAP API knows, including all custom fields.
const ALL: &[&str] = &[
    "approvals",
    "assignee",
    "attachments",
    "author",
    "categories",
    "comments",
    "created",
    "description",
    "dueDate",
    "externallyLinkedWorkItems",
    "hyperlinks",
    "id",
    "initialEstimate",
    "linkedOslcResources",
    "linkedRevisions",
    "linkedRevisionsDerived",
    "linkedWorkItems",
    "linkedWorkItemsDerived",
    "location",
    "module",
    "outlineNumber",
    "plannedEnd",
    "plannedIn",
    "plannedStart",
    "planningConstraints",
    "previousStatus",
    "priority",
    "project",
    "remainingEstimate",
    "resolution",
    "resolvedOn",
    "severity",
    "status",
    "timePoint",
    "timeSpent",
    "title",
    "type",
    "updated",
    "workRecords",
    "customFields",
];

impl Fields {
    /// Enough to list and identify work items: id, title, type, status, priority, severity,
    /// resolution, author, assignee, created, updated, dueDate, outlineNumber, project and module
    /// (the document).
    pub fn standard() -> Self {
        Self::new(STANDARD.iter().copied())
    }

    /// Every built-in field, including description and links. Custom fields need
    /// [`Fields::with_custom`] (Polarion returns none for the key `customFields` alone).
    pub fn all() -> Self {
        Self::new(ALL.iter().copied())
    }

    /// Exactly these fields.
    pub fn new<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(keys.into_iter().map(Into::into).collect())
    }

    /// Add a field.
    pub fn with(mut self, key: impl Into<String>) -> Self {
        let key = key.into();
        if !self.0.contains(&key) {
            self.0.push(key);
        }
        self
    }

    /// Add a custom field by its key.
    pub fn with_custom(self, key: &str) -> Self {
        self.with(format!("customFields.{key}"))
    }

    pub fn keys(&self) -> &[String] {
        &self.0
    }
}

impl Default for Fields {
    fn default() -> Self {
        Self::standard()
    }
}

/// A Lucene query for work items, documents, wiki pages, test runs, plans or baselines.
///
/// ```
/// use polarion::{Fields, WorkItemQuery};
///
/// let query = WorkItemQuery::new("type:requirement AND status:open")
///     .project("MyProject")
///     .sort("id")
///     .limit(20)
///     .fields(Fields::standard().with("description").with_custom("asil"));
/// assert_eq!(query.lucene(), "project.id:MyProject AND (type:requirement AND status:open)");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    query: String,
    project: Option<String>,
    sort: String,
    limit: Option<u32>,
    fields: Option<Fields>,
    baseline: Option<String>,
    revision: Option<String>,
}

/// The query type used for work items; the same as [`Query`].
pub type WorkItemQuery = Query;

impl Query {
    /// A query in Polarion's Lucene syntax; empty matches everything (of the project, if set).
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            project: None,
            sort: "id".to_owned(),
            limit: Some(DEFAULT_LIMIT),
            fields: None,
            baseline: None,
            revision: None,
        }
    }

    /// Restrict the query to one project.
    pub fn project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    /// Sort order, e.g. `id`, `~updated` (descending) or `status id`. Default: `id`.
    pub fn sort(mut self, sort: impl Into<String>) -> Self {
        self.sort = sort.into();
        self
    }

    /// Maximum number of results. Default: [`DEFAULT_LIMIT`].
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Return every match. Mind the server load on large projects.
    pub fn unlimited(mut self) -> Self {
        self.limit = None;
        self
    }

    /// Fields to fetch. Default: [`Fields::standard`] for work items, all fields otherwise.
    pub fn fields(mut self, fields: Fields) -> Self {
        self.fields = Some(fields);
        self
    }

    /// Search the index as it was in a baseline (repository revision).
    /// Supported for work items, documents and wiki pages; ignored by counting.
    pub fn baseline(mut self, revision: impl Into<String>) -> Self {
        self.baseline = Some(revision.into());
        self
    }

    /// Work items only: search their current state but return them as they were in
    /// `revision`.
    pub fn revision(mut self, revision: impl Into<String>) -> Self {
        self.revision = Some(revision.into());
        self
    }

    /// The Lucene query sent to the server, including the project restriction.
    pub fn lucene(&self) -> String {
        let query = self.query.trim();
        match &self.project {
            None => query.to_owned(),
            Some(p) if query.is_empty() => format!("project.id:{}", term(p)),
            Some(p) => format!("project.id:{} AND ({query})", term(p)),
        }
    }

    /// `None` when the query is empty (for operations that take a nullable query).
    pub(crate) fn lucene_opt(&self) -> Option<String> {
        Some(self.lucene()).filter(|q| !q.is_empty())
    }

    pub(crate) fn sort_key(&self) -> &str {
        &self.sort
    }

    pub(crate) fn limit_value(&self) -> Option<u32> {
        self.limit
    }

    /// The SOAP `resultsLimit` (-1 for no limit).
    pub(crate) fn results_limit(&self) -> i32 {
        self.limit
            .map_or(-1, |l| i32::try_from(l).unwrap_or(i32::MAX))
    }

    pub(crate) fn field_list(&self) -> Option<&Fields> {
        self.fields.as_ref()
    }

    pub(crate) fn baseline_revision(&self) -> Option<&str> {
        self.baseline.as_deref()
    }

    pub(crate) fn revision_value(&self) -> Option<&str> {
        self.revision.as_deref()
    }
}

/// A Lucene term: bare if it is a simple token, quoted otherwise.
pub(crate) fn term(value: &str) -> String {
    let simple = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if simple {
        value.to_owned()
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_restriction() {
        assert_eq!(WorkItemQuery::new("").project("P").lucene(), "project.id:P");
        assert_eq!(
            WorkItemQuery::new("a OR b").project("My Project").lucene(),
            "project.id:\"My Project\" AND (a OR b)"
        );
        assert_eq!(WorkItemQuery::new("a OR b").lucene(), "a OR b");
    }

    #[test]
    fn limits() {
        assert_eq!(WorkItemQuery::new("").results_limit(), 100);
        assert_eq!(WorkItemQuery::new("").limit(5).results_limit(), 5);
        assert_eq!(WorkItemQuery::new("").unlimited().results_limit(), -1);
    }

    #[test]
    fn fields_do_not_repeat() {
        let f = Fields::new(["id"]).with("id").with_custom("asil");
        assert_eq!(f.keys(), ["id", "customFields.asil"]);
    }
}
