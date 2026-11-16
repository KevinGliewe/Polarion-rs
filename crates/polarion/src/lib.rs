//! Convenient access to Siemens Polarion ALM.
//!
//! Built on the SOAP web services ([`polarion_soap`]), which work with
//! ordinary user permissions (the REST API needs extra role permissions).
//! Results are flat domain types that serialize to compact JSON: enum
//! options, users and linked work items are reduced to their ids, and custom
//! fields become a `key → value` map.
//!
//! Operations are grouped by area and cover the whole SOAP API:
//! [`Polarion::items`] (work items), [`Polarion::documents`],
//! [`Polarion::schema`] (field definitions, enumerations, configuration),
//! [`Polarion::tests`] (test runs), [`Polarion::plans`], [`Polarion::wiki`],
//! [`Polarion::baselines`], [`Polarion::revisions`], [`Polarion::activities`],
//! [`Polarion::folders`], [`Polarion::projects`], [`Polarion::users`],
//! [`Polarion::security`] and [`Polarion::builds`]; sessions and transactions
//! are on [`Polarion`] itself.
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
//! Writes not tied to a project (users, global folders, activity comments, the account
//! vault) also need [`PolarionBuilder::allow_global_writes`]. The raw SOAP client is
//! available through [`Polarion::soap`] (not checked against the allowlist).
//!
//! [`PolarionBuilder::restrict_writes`] restricts a client to a list of objects instead (and
//! [`PolarionBuilder::allow_creates`] to a budget of creates): writes that change anything
//! else, or that may change objects the client cannot see (workflow actions, deletes, ...),
//! are refused, and a SOAP write filter checks every request, those made through
//! [`Polarion::soap`] included.
//!
//! # Markdown
//!
//! The feature `markdown` reads and writes work item descriptions as Markdown
//! (`description_markdown`, `set_description_markdown` and `append_description_markdown` on
//! [`WorkItems`]), converted by the `polarion-markdown` crate, which is re-exported as
//! `polarion_markdown`. With `mermaid`, ```` ```mermaid ```` blocks are rendered to SVG and
//! attached to the work item as `mermaid-<hash>.svg` (an unchanged diagram is neither rendered
//! nor uploaded again), and the description shows them as images; reading the description back
//! restores the blocks. `highlight` highlights fenced code that names its language.

// The mock helpers of the integration tests (`tests/common`), shared by the unit tests, name
// this crate `polarion`.
#[cfg(test)]
extern crate self as polarion;

mod activities;
mod atomic;
mod baselines;
mod builds;
mod client;
mod convert;
mod documents;
mod error;
mod folders;
mod items;
#[cfg(feature = "markdown")]
mod markdown;
mod model;
mod plans;
mod projects;
mod query;
mod restricted;
mod revisions;
mod schema;
mod security;
mod tests_mgmt;
pub mod uri;
mod wiki;
mod write;

pub use activities::{Activities, ActivityScope};
pub use baselines::Baselines;
pub use builds::Builds;
pub use client::{CreateBudget, Polarion, PolarionBuilder};
pub use documents::Documents;
pub use error::{Error, Result};
pub use folders::Folders;
pub use items::WorkItems;
#[cfg(feature = "markdown")]
pub use markdown::{
    DiagramAttachment, ExportedImage, ImageAttachment, LocalImage, MarkdownExport,
    MarkdownReadOptions, MarkdownUpdate, MarkdownWriteOptions,
};
pub use model::{
    Approval, Attachment, Baseline, Category, Change, Comment, Document, EnumOption, FieldChange,
    FieldDefinition, FieldValue, Hyperlink, Link, LinkedRevision, Plan, Project, ProjectGroup,
    Revision, TestRecord, TestRun, TestStepResult, TestSteps, Text, TimePoint, User, WikiPage,
    WorkItem, WorkRecord, WorkflowAction,
};
pub use plans::Plans;
#[cfg(feature = "markdown")]
pub use polarion_markdown;
pub use polarion_soap as soap;
pub use polarion_soap::Auth;
pub use projects::{Projects, Users};
pub use query::{DEFAULT_LIMIT, Fields, Query, WorkItemQuery};
pub use revisions::Revisions;
pub use schema::Schema;
pub use security::Security;
pub use tests_mgmt::TestRuns;
pub use uri::{DocumentRef, ItemRef};
pub use wiki::Wiki;
pub use write::{
    DocumentUpdate, NewDocument, NewExternalLink, NewHyperlink, NewLink, NewLinkedRevision,
    NewOslcLink, NewPlan, NewPlanningConstraint, NewTestRun, NewWorkItem, NewWorkRecord,
    PlanUpdate, ReuseDocument, TestRunUpdate, UserUpdate, WorkItemUpdate,
};
