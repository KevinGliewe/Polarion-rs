//! Query parameters shared by most Polarion REST endpoints.

/// Query string builder.
///
/// Holds raw `(key, value)` pairs in insertion order. Typed setters cover the
/// parameters the Polarion API defines; [`Query::param`] covers anything else.
/// Endpoints ignore parameters they do not support.
///
/// ```
/// use polarion_rest::Query;
///
/// let q = Query::new()
///     .query("type:requirement AND status:open")
///     .sort("-updated")
///     .all_fields("workitems")
///     .include("author")
///     .page_size(50);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    params: Vec<(String, String)>,
}

impl Query {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a parameter, replacing any previous value for the same key.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.set(key.into(), value.into());
        self
    }

    /// `page[size]`: number of resources per page (Polarion's default maximum is 100).
    pub fn page_size(self, size: u32) -> Self {
        self.param("page[size]", size.to_string())
    }

    /// `page[number]`: 1-based page index.
    pub fn page_number(self, number: u32) -> Self {
        self.param("page[number]", number.to_string())
    }

    /// `query`: Lucene query, e.g. `type:task AND status:open`.
    pub fn query(self, query: impl Into<String>) -> Self {
        self.param("query", query)
    }

    /// `sort`: comma-separated field list, prefix with `-` for descending.
    pub fn sort(self, sort: impl Into<String>) -> Self {
        self.param("sort", sort)
    }

    /// `include`: comma-separated relationship paths to include in the response.
    pub fn include(self, include: impl Into<String>) -> Self {
        self.param("include", include)
    }

    /// `revision`: read the resource(s) at a specific revision.
    pub fn revision(self, revision: impl Into<String>) -> Self {
        self.param("revision", revision)
    }

    /// `fields[<resource_type>]`: sparse fieldset, comma-separated field names or `@all`/`@basic`.
    pub fn fields(self, resource_type: &str, fields: impl Into<String>) -> Self {
        self.param(format!("fields[{resource_type}]"), fields)
    }

    /// Shorthand for `fields[<resource_type>]=@all`.
    ///
    /// By default Polarion only returns a basic set of attributes; this
    /// requests all of them, including custom fields.
    pub fn all_fields(self, resource_type: &str) -> Self {
        self.fields(resource_type, "@all")
    }

    /// `workflowAction`: execute a workflow action as part of an update.
    pub fn workflow_action(self, action: impl Into<String>) -> Self {
        self.param("workflowAction", action)
    }

    /// `changeTypeTo`: change the work item type as part of an update.
    pub fn change_type_to(self, work_item_type: impl Into<String>) -> Self {
        self.param("changeTypeTo", work_item_type)
    }

    /// Value of a parameter, if set.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// All parameters in insertion order.
    pub fn pairs(&self) -> &[(String, String)] {
        &self.params
    }

    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }

    fn set(&mut self, key: String, value: String) {
        match self.params.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => self.params.push((key, value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_replaces_existing_key() {
        let q = Query::new().page_size(10).query("a").page_size(20);
        assert_eq!(
            q.pairs(),
            &[
                ("page[size]".to_owned(), "20".to_owned()),
                ("query".to_owned(), "a".to_owned())
            ]
        );
    }

    #[test]
    fn sparse_fields_key() {
        let q = Query::new().all_fields("workitems");
        assert_eq!(q.get("fields[workitems]"), Some("@all"));
    }
}
