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
    /// Custom fields with a value; empty fields are left out.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub custom_fields: BTreeMap<String, FieldValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
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
