//! Convenient access to Siemens Polarion ALM.
//!
//! Built on the SOAP web services ([`polarion_soap`]), which work with
//! ordinary user permissions (the REST API needs extra role permissions).
//! Results are flat domain types that serialize to compact JSON: enum
//! options, users and linked work items are reduced to their ids, and custom
//! fields become a `key → value` map.
//!
//! Operations are grouped by area: [`Polarion::items`] (work items),
//! [`Polarion::documents`], [`Polarion::schema`] (field definitions,
//! enumerations), [`Polarion::projects`] and [`Polarion::users`].
//!
//! ```no_run
//! use polarion::{Auth, ItemRef, Polarion, WorkItemQuery};
//!
//! # async fn run() -> polarion::Result<()> {
//! // Read-only: operations that may modify data are refused before they are sent.
//! let polarion = Polarion::new("https://polarion.example.com/polarion", Auth::Token("<token>".into()))?;
//!
//! let open = polarion
//!     .items()
//!     .query(&WorkItemQuery::new("type:requirement AND status:open").project("MyProject"))
//!     .await?;
//! for item in &open {
//!     println!("{:?} {:?}", item.id, item.title);
//! }
//!
//! let item = polarion.items().get(&ItemRef::new("MyProject", "MP-42")).await?;
//! for (key, value) in &item.custom_fields {
//!     println!("{key} = {:?}", value.as_str());
//! }
//! let back_links = polarion.items().back_links(&ItemRef::new("MyProject", "MP-42")).await?;
//!
//! for doc in polarion.documents().list("MyProject", None).await? {
//!     println!("{:?}/{:?}: {:?}", doc.space, doc.name, doc.title);
//! }
//! # Ok(()) }
//! ```
//!
//! # Writes
//!
//! Writes are enabled per project; every write to another project is refused
//! with [`Error::WriteNotAllowed`] before anything is sent:
//!
//! ```no_run
//! use polarion::{Auth, FieldValue, ItemRef, NewWorkItem, Polarion, WorkItemUpdate};
//!
//! # async fn run() -> polarion::Result<()> {
//! let polarion = Polarion::builder("https://polarion.example.com/polarion", Auth::Token("<token>".into()))
//!     .allow_writes(["Sandbox"])
//!     .build()?;
//! let items = polarion.items();
//!
//! let mut new = NewWorkItem::new("Sandbox", "requirement", "Brakes must work");
//! new.custom_fields.insert("asil".into(), FieldValue::Enum("D".into()));
//! let item = items.create(&new).await?;
//!
//! let update = WorkItemUpdate { title: Some("Brakes must always work".into()), ..Default::default() };
//! items.update(&item, &update).await?;
//! items.add_link(&item, "relates_to", &ItemRef::new("Sandbox", "SB-1")).await?;
//! items.transition(&item, "reviewed").await?;
//! # Ok(()) }
//! ```
//!
//! Everything else is available through [`Polarion::soap`] (not checked against the allowlist).

mod client;
mod convert;
mod documents;
mod error;
mod items;
mod model;
mod projects;
mod query;
mod schema;
mod uri;
mod write;

pub use client::{Polarion, PolarionBuilder};
pub use documents::Documents;
pub use error::{Error, Result};
pub use items::WorkItems;
pub use model::{
    Approval, Attachment, Category, Change, Comment, Document, EnumOption, FieldChange,
    FieldDefinition, FieldValue, Hyperlink, Link, LinkedRevision, Project, Text, TimePoint, User,
    WorkItem, WorkRecord, WorkflowAction,
};
pub use polarion_soap as soap;
pub use polarion_soap::Auth;
pub use projects::{Projects, Users};
pub use query::{DEFAULT_LIMIT, Fields, WorkItemQuery};
pub use schema::Schema;
pub use uri::{DocumentRef, ItemRef};
pub use write::{NewWorkItem, NewWorkRecord, WorkItemUpdate};
