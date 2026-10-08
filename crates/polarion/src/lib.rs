//! Convenient access to Siemens Polarion ALM.
//!
//! Built on the SOAP web services ([`polarion_soap`]), which work with
//! ordinary user permissions (the REST API needs extra role permissions).
//! Results are flat domain types that serialize to compact JSON: enum
//! options, users and linked work items are reduced to their ids, and custom
//! fields become a `key → value` map.
//!
//! ```no_run
//! use polarion::{Auth, Polarion, WorkItemQuery};
//!
//! # async fn run() -> polarion::Result<()> {
//! // Read-only: operations that may modify data are refused before they are sent.
//! let polarion = Polarion::new("https://polarion.example.com/polarion", Auth::Token("<token>".into()))?;
//!
//! let open = polarion
//!     .query_work_items(&WorkItemQuery::new("type:requirement AND status:open").project("MyProject"))
//!     .await?;
//! for item in &open {
//!     println!("{:?} {:?}", item.id, item.title);
//! }
//!
//! let item = polarion.work_item("MyProject", "MP-42").await?;
//! for (key, value) in &item.custom_fields {
//!     println!("{key} = {:?}", value.as_str());
//! }
//! let back_links = polarion.back_links(item.uri.as_deref().unwrap_or_default()).await?;
//!
//! for doc in polarion.documents("MyProject", None).await? {
//!     println!("{:?}/{:?}: {:?}", doc.space, doc.name, doc.title);
//! }
//! # Ok(()) }
//! ```
//!
//! Everything else is available through [`Polarion::soap`].

mod client;
mod convert;
mod error;
mod model;
mod query;
mod uri;

pub use client::Polarion;
pub use error::{Error, Result};
pub use model::{Document, FieldValue, Hyperlink, Link, Project, Text, User, WorkItem};
pub use polarion_soap as soap;
pub use polarion_soap::Auth;
pub use query::{DEFAULT_LIMIT, Fields, WorkItemQuery};
pub use uri::{DocumentRef, ItemRef};
