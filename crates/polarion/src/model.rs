//! Domain types: flat, serializable views of the SOAP types.
//!
//! Enum options, users and work items are reduced to their ids, custom fields
//! become a `key → value` map, and empty fields are left out of the JSON form.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::uri::{DocumentRef, ItemRef};

/// Rich text (`text/html` or `text/plain`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Text {
    /// MIME type; `type` in JSON.
    #[serde(rename = "type")]
    pub content_type: String,
    pub content: String,
}

/// A custom field value.
///
/// Serializes untagged to plain JSON: enum options become their id, lists
/// become arrays, rich text an object with `type` and `content`, and dates and
/// times ISO 8601 strings.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum FieldValue {
    String(String),
    Bool(bool),
    Int(i64),
    Float(f64),
    /// `xsd:decimal`, kept as text to avoid rounding (currency fields).
    Decimal(String),
    Date(String),
    DateTime(String),
    Time(String),
    /// The id of an enum option (also used for user references).
    Enum(String),
    Text(Text),
    /// Multi-valued fields, e.g. a multi-enum.
    List(Vec<FieldValue>),
    /// A value this library does not map; `xml` holds the original element when available.
    #[serde(rename_all = "camelCase")]
    Other {
        xsi_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        xml: Option<String>,
    },
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for FieldValue {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "FieldValue".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "A custom field value: a string (also enum option ids, dates and decimals), number, boolean, a list of values, rich text {type, content}, or {xsiType, xml} for unmapped types"
        })
    }
}

impl FieldValue {
    /// The value as text, for enum ids, strings, dates and numbers kept as text.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            FieldValue::String(s)
            | FieldValue::Decimal(s)
            | FieldValue::Date(s)
            | FieldValue::DateTime(s)
            | FieldValue::Time(s)
            | FieldValue::Enum(s) => Some(s),
            FieldValue::Text(t) => Some(&t.content),
            _ => None,
        }
    }
}

/// A link between two work items.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Link {
    /// Link role id, e.g. `parent` or `verifies`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// The other work item: the target of an outgoing link, the source of a back link.
    pub item: ItemRef,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub suspect: bool,
    /// Revision the link is pinned to, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

/// A link from a work item to a web resource.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Hyperlink {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub url: String,
}

/// A work item. Fields not requested from the server are `None` or empty.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct WorkItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Work item type id; `type` in JSON.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Text>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// User ids.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assignees: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_end: Option<String>,
    /// The document containing the work item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<DocumentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_number: Option<String>,
    /// Category ids.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// Outgoing links.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hyperlinks: Vec<Hyperlink>,
    /// Links to work items on other servers (OSLC).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub external_links: Vec<Hyperlink>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_revisions: Vec<LinkedRevision>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub approvals: Vec<Approval>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub work_records: Vec<WorkRecord>,
    /// Time point id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_point: Option<String>,
    /// URIs of the plans the work item is planned in.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub planned_in: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_estimate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_estimate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_spent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Custom fields with a value; empty fields are left out.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub custom_fields: BTreeMap<String, FieldValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

impl WorkItem {
    /// A reference to this work item for further calls (from its URI).
    pub fn reference(&self) -> Option<ItemRef> {
        self.uri.as_deref().map(ItemRef::from_uri)
    }
}

/// A revision in a source repository linked to a work item.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LinkedRevision {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    pub revision: String,
}

/// A comment on a work item.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Text>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub resolved: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// URI of the comment this one replies to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// URIs of the replies.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<String>,
    /// For document comments: the work item the comment refers to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_item: Option<ItemRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// Attachment metadata; the content is fetched with [`crate::WorkItems::attachment`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<i64>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// An approval of a work item.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Approval {
    /// User id of the approver.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Approval status id, e.g. `waiting`, `approved`, `disapproved`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Time booked on a work item.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct WorkRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_spent: Option<String>,
    /// Work record type id; `type` in JSON.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// A workflow action (status transition) of a work item.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAction {
    /// The id to pass to [`crate::WorkItems::perform_action`].
    pub id: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The action id in the workflow configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_id: Option<String>,
    /// The status the work item gets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_status: Option<String>,
    /// Fields that must have a value before the action can be performed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub required_fields: Vec<String>,
    /// Why the action is not available (for [`crate::WorkItems::unavailable_actions`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// One revision in the history of a work item.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Change {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// The revision that created the work item.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub creation: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<FieldChange>,
}

/// The change of one field in a [`Change`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct FieldChange {
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<FieldValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<FieldValue>,
    /// For multi-valued fields: values added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<FieldValue>,
    /// For multi-valued fields: values removed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<FieldValue>,
}

/// The definition of a custom field.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct FieldDefinition {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Value type: the local name of the schema type, e.g. `string`, `boolean`, `date`,
    /// `EnumOptionId`, `Text`; `type` in JSON.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub multi: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    /// For enum fields: the enumeration id (see [`crate::Schema::enum_options`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<FieldValue>,
}

/// An option of an enumeration (types, statuses, priorities, enum custom fields, ...).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct EnumOption {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    pub sequence: i32,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
}

/// A work item category.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Category {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Text>,
}

/// A time point (milestone) of a project.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TimePoint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Text>,
}

/// A document (called "module" in the SOAP API).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Document {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Space (folder) id, `_default` for the default space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Document type id; `type` in JSON.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<String>,
    /// Link role that structures the work items in the document, e.g. `parent`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure_link_role: Option<String>,
    /// Work item types allowed in the document.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub allowed_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uses_outline_numbering: Option<bool>,
    /// For derived (reused) documents: the source document's URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub custom_fields: BTreeMap<String, FieldValue>,
    /// The document body (Polarion HTML with embedded work item placeholders).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Text>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

impl Document {
    /// A reference to this document for further calls (from its URI).
    pub fn reference(&self) -> Option<DocumentRef> {
        self.uri.as_deref().map(DocumentRef::from_uri)
    }
}

/// A project.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Project {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Text>,
    /// User id of the project lead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracker_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
}

/// A user.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct User {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// A project group (folder of projects).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct ProjectGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// URI of the parent group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// URIs of the subgroups.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    /// Ids of the projects directly in this group.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// A wiki page.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct WikiPage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Page type; `type` in JSON.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<String>,
    /// URIs of linked pages.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_pages: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Text>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// A baseline: a named repository revision.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Baseline {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The repository revision the baseline marks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// For baselines of a single object (e.g. a document): its URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_object: Option<String>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// A repository revision (commit).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Revision {
    /// The revision id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Commits made by Polarion itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub internal: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_work_items: Vec<ItemRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// A test run (or test run template).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct TestRun {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Test run type id; `type` in JSON.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub template: bool,
    /// URI of the template the run was created from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_uri: Option<String>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    /// How test cases are selected (e.g. `manualSelection`, `staticQueryResult`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub select_test_cases_by: Option<String>,
    /// The query selecting the test cases.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// The document the test cases come from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<DocumentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_defect: Option<ItemRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_in_history: Option<bool>,
    /// URIs of the projects in the run's project span.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub project_span: Vec<String>,
    /// Test parameter values by name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, FieldValue>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<TestRecord>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub custom_fields: BTreeMap<String, FieldValue>,
    /// The run's report page content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Text>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// The result of executing one test case in a test run.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct TestRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_case: Option<ItemRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_case_revision: Option<String>,
    /// Result id, e.g. `passed`, `failed`, `blocked`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<Text>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executed: Option<String>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executed_by: Option<String>,
    /// Seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defect: Option<ItemRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed: Option<bool>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, FieldValue>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub step_results: Vec<TestStepResult>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

/// The result of one test step.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TestStepResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<Text>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

/// The test steps of a test case: a table whose columns are `columns`
/// (e.g. `step`, `description`, `expectedResult`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TestSteps {
    pub columns: Vec<String>,
    /// One row per step, one cell per column.
    pub steps: Vec<Vec<Text>>,
}

/// A plan (release, iteration) or plan template.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub template: bool,
    /// URI of the parent plan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Text>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_estimate: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimation_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prioritization_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculation_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<i32>,
    /// Work item types that may be planned.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub allowed_types: Vec<String>,
    /// URIs of the projects in the plan's project span.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub project_span: Vec<String>,
    /// The planned work items.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ItemRef>,
    /// User id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub custom_fields: BTreeMap<String, FieldValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Text>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}
