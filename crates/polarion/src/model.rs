//! Domain types: flat, serializable views of the SOAP types.
//!
//! Enum options, users and work items are reduced to their ids, custom fields
//! become a `key → value` map, and empty fields are left out of the JSON form.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::uri::{DocumentRef, ItemRef};

/// Rich text (`text/html` or `text/plain`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
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
pub struct Hyperlink {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub url: String,
}

/// A work item. Fields not requested from the server are `None` or empty.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
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
pub struct LinkedRevision {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    pub revision: String,
}

/// A comment on a work item.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// Attachment metadata; the content is fetched with [`crate::WorkItems::attachment`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
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
pub struct User {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}
