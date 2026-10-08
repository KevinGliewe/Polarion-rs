//! JSON:API envelope types as used by the Polarion REST API.
//!
//! Polarion follows JSON:API with a few deviations (bulk create/update/delete,
//! `application/json` instead of `application/vnd.api+json`, `revision` on
//! resource identifiers). The types here are deliberately lenient: every
//! optional member is `Option`, and unknown members are ignored.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

/// Top-level JSON:API document (`{"data": ..., "included": ..., ...}`).
///
/// Called `Envelope` rather than `Document` to avoid clashing with Polarion
/// Documents. `D` is the primary data: `Resource<A>` for single-resource
/// endpoints, `Vec<Resource<A>>` for collections.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope<D> {
    pub data: D,
    /// Resources pulled in via the `include` query parameter.
    ///
    /// Kept as raw JSON because one document can include resources of many
    /// types. Use [`Envelope::find_included`] to look up and decode one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub included: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,
}

/// An envelope whose primary data is a single resource.
pub type SingleEnvelope<A = Value> = Envelope<Resource<A>>;
/// An envelope whose primary data is a list of resources.
pub type ListEnvelope<A = Value> = Envelope<Vec<Resource<A>>>;

impl<D> Envelope<D> {
    /// Wrap primary data in an envelope.
    pub fn new(data: D) -> Self {
        Self {
            data,
            included: Vec::new(),
            meta: None,
            links: None,
        }
    }

    /// Find a resource in `included` by type and id and decode its attributes as `A`.
    ///
    /// Returns `None` if no such resource was included or it does not decode as `A`.
    pub fn find_included<A: DeserializeOwned>(
        &self,
        resource_type: &str,
        id: &str,
    ) -> Option<Resource<A>> {
        self.included
            .iter()
            .find(|v| {
                v.get("type").and_then(Value::as_str) == Some(resource_type)
                    && v.get("id").and_then(Value::as_str) == Some(id)
            })
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }
}

impl<A> ListEnvelope<A> {
    /// Total number of matching resources across all pages, if reported by the server.
    pub fn total_count(&self) -> Option<u64> {
        self.meta.as_ref().and_then(|m| m.total_count)
    }

    /// `true` if the server reported a next page.
    pub fn has_next_page(&self) -> bool {
        self.links.as_ref().is_some_and(|l| l.next.is_some())
    }
}

/// A JSON:API resource object.
///
/// The same type is used for responses and request bodies: for creates, leave
/// `id` empty; for updates, only set the attributes that should change (all
/// attribute structs in this crate skip `None` fields on serialization).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resource<A = Value> {
    #[serde(rename = "type")]
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Local id of a resource being created, e.g. to match an attachment
    /// resource to its file part in a multipart upload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<A>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub relationships: BTreeMap<String, Relationship>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,
}

/// Implemented by every generated attributes struct; ties it to its JSON:API `type`.
pub trait ResourceKind {
    /// Resource type name, e.g. `"workitems"`.
    const TYPE: &'static str;
}

impl<A: ResourceKind> Resource<A> {
    /// A new resource of `A`'s type with the given attributes, e.g. for a create request.
    pub fn from_attributes(attributes: A) -> Self {
        Self::new(A::TYPE).with_attributes(attributes)
    }

    /// A reference to a resource of `A`'s type.
    pub fn reference(id: impl Into<String>) -> ResourceRef {
        ResourceRef::new(A::TYPE, id)
    }
}

impl<A> Resource<A> {
    /// A new, empty resource of the given type (e.g. `"workitems"`).
    pub fn new(resource_type: impl Into<String>) -> Self {
        Self {
            resource_type: resource_type.into(),
            id: None,
            revision: None,
            lid: None,
            attributes: None,
            relationships: BTreeMap::new(),
            meta: None,
            links: None,
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_lid(mut self, lid: impl Into<String>) -> Self {
        self.lid = Some(lid.into());
        self
    }

    pub fn with_attributes(mut self, attributes: A) -> Self {
        self.attributes = Some(attributes);
        self
    }

    /// Set a to-one relationship.
    pub fn with_relationship(mut self, name: impl Into<String>, target: ResourceRef) -> Self {
        self.relationships
            .insert(name.into(), Relationship::to_one(Some(target)));
        self
    }

    /// Set a to-many relationship.
    pub fn with_relationships(
        mut self,
        name: impl Into<String>,
        targets: impl IntoIterator<Item = ResourceRef>,
    ) -> Self {
        self.relationships
            .insert(name.into(), Relationship::to_many(targets));
        self
    }

    /// Identifier of this resource, if it has an id.
    pub fn to_ref(&self) -> Option<ResourceRef> {
        self.id.as_ref().map(|id| ResourceRef {
            resource_type: self.resource_type.clone(),
            id: id.clone(),
            revision: self.revision.clone(),
        })
    }

    /// Look up a relationship by name.
    pub fn relationship(&self, name: &str) -> Option<&Relationship> {
        self.relationships.get(name)
    }
}

/// A resource identifier object (`{"type": ..., "id": ...}`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceRef {
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

impl ResourceRef {
    pub fn new(resource_type: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            resource_type: resource_type.into(),
            id: id.into(),
            revision: None,
        }
    }
}

/// A relationship object.
///
/// `data` is always serialized (as `null` when `None`) so that a PATCH can
/// clear a to-one relationship.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    #[serde(default)]
    pub data: Option<RelationshipData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,
}

impl Relationship {
    pub fn to_one(target: Option<ResourceRef>) -> Self {
        Self {
            data: target.map(RelationshipData::ToOne),
            meta: None,
            links: None,
        }
    }

    pub fn to_many(targets: impl IntoIterator<Item = ResourceRef>) -> Self {
        Self {
            data: Some(RelationshipData::ToMany(targets.into_iter().collect())),
            meta: None,
            links: None,
        }
    }

    /// All referenced resources, regardless of cardinality.
    pub fn refs(&self) -> &[ResourceRef] {
        match &self.data {
            Some(RelationshipData::ToOne(r)) => std::slice::from_ref(r),
            Some(RelationshipData::ToMany(rs)) => rs,
            None => &[],
        }
    }
}

/// Linkage data of a relationship.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RelationshipData {
    ToOne(ResourceRef),
    ToMany(Vec<ResourceRef>),
}

/// Document- or resource-level links.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Links {
    #[serde(rename = "self", default, skip_serializing_if = "Option::is_none")]
    pub self_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related: Option<String>,
    /// Link to the resource in the Polarion web UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal: Option<String>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

/// Document-level meta information.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(
        rename = "totalCount",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub total_count: Option<u64>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

/// Serializes as `{"data": <data>}` without cloning; used for request bodies.
#[derive(Debug, Serialize)]
pub(crate) struct DataRef<'a, T: ?Sized> {
    pub data: &'a T,
}

/// Error response body (`{"errors": [...]}`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ErrorDocument {
    #[serde(default)]
    pub errors: Vec<ErrorObject>,
}

/// A single JSON:API error object.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ErrorObject {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ErrorSource>,
}

/// Location of the problem that caused an error.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ErrorSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<Value>,
}

/// Rich text value, as used by `description`, comment `text`, etc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextContent {
    /// Either `text/html` or `text/plain`.
    #[serde(rename = "type")]
    pub content_type: String,
    pub value: String,
}

impl TextContent {
    pub fn html(value: impl Into<String>) -> Self {
        Self {
            content_type: "text/html".to_owned(),
            value: value.into(),
        }
    }

    pub fn plain(value: impl Into<String>) -> Self {
        Self {
            content_type: "text/plain".to_owned(),
            value: value.into(),
        }
    }
}
