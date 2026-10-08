//! Async client for the Siemens Polarion ALM SOAP web services.
//!
//! All operations of the seven services (Session, Project, Tracker,
//! TestManagement, Planning, Security, Builder) are generated from the WSDLs in
//! `spec/soap/`. Each service is an accessor on [`Client`]; operations are
//! methods in snake case.
//!
//! ```no_run
//! use polarion_soap::{Auth, Client};
//!
//! # async fn run() -> polarion_soap::Result<()> {
//! let client = Client::builder("https://polarion.example.com/polarion", Auth::Token("<token>".into()))
//!     .read_only(true)
//!     .build()?;
//! let items = client
//!     .tracker()
//!     .query_work_items_limited("project.id:MyProject AND type:requirement", "id", &["id".into(), "title".into()], 10)
//!     .await?;
//! for item in &items {
//!     println!("{:?}: {:?}", item.id, item.title);
//! }
//! # Ok(()) }
//! ```
//!
//! Custom field values come as [`types::AnyValue`], decoded from the
//! `xsi:type` the server sends (enum options, rich text, dates, ...).

mod client;
mod error;
mod generated;
pub mod xml;

pub use client::{Auth, Client, ClientBuilder};
pub use error::{Error, Result};
pub use generated::{NAMESPACES, services, types};
