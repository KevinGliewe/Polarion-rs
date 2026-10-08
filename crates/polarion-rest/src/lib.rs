//! Low-level async client for the Siemens Polarion ALM REST API (v1).
//!
//! Every operation of the API is available as a method on [`Client`],
//! generated from the OpenAPI definition in `spec/polarion-openapi.json`.
//! Resource attributes are typed per resource in [`models`]; custom fields
//! end up in each attributes struct's `extra` map.
//!
//! ```no_run
//! use polarion_rest::{Client, Query};
//!
//! # async fn run() -> polarion_rest::Result<()> {
//! let client = Client::new("https://polarion.example.com/polarion", "<personal access token>")?;
//! let items = client
//!     .get_work_items("MyProject", &Query::new().query("type:requirement").all_fields("workitems"))
//!     .await?;
//! for item in &items.data {
//!     let attrs = item.attributes.as_ref();
//!     println!("{:?}: {:?}", item.id, attrs.and_then(|a| a.title.as_deref()));
//! }
//! # Ok(()) }
//! ```

mod client;
mod error;
mod generated;
pub mod jsonapi;
mod query;

pub use client::{Client, ClientBuilder, FilePart};
pub use error::{Error, Result};
pub use generated::models;
pub use jsonapi::{
    Envelope, ListEnvelope, Relationship, RelationshipData, Resource, ResourceKind, ResourceRef,
    SingleEnvelope, TextContent,
};
pub use query::Query;
pub use reqwest::{Method, StatusCode};
