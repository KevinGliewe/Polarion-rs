//! Inputs for writes: new work items, updates, and typing JSON values for custom fields.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::model::{FieldDefinition, FieldValue, Text};
use crate::uri::ItemRef;

/// A work item to create with [`crate::WorkItems::create`] (or
/// [`crate::WorkItems::create_in_document`]).
///
/// `createWorkItem` carries the fields up to `custom_fields`. The others are sent by follow-up
/// requests right after the create, in the order of the fields: as one transaction, in a Polarion
/// session of its own ([`NewWorkItem::follow_ups`]), so a workflow that requires one of them
/// (e.g. a category) sees it, and a failure leaves nothing behind. A follow-up the server
/// answers with "nothing added" (e.g. a category given twice) fails the whole create.
///
/// Not set on create: the status and resolution (workflow actions, [`crate::WorkItems::transition`]),
/// comments, attachments, work records and approval states; add them afterwards with the
/// dedicated methods.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewWorkItem {
    pub project: String,
    /// Work item type id, e.g. `requirement`.
    pub kind: String,
    pub title: Option<String>,
    pub description: Option<Text>,
    /// Priority id, e.g. `50.0`.
    pub priority: Option<String>,
    pub severity: Option<String>,
    pub due_date: Option<String>,
    pub initial_estimate: Option<String>,
    pub remaining_estimate: Option<String>,
    pub custom_fields: BTreeMap<String, FieldValue>,
    /// Category ids (`addCategory`).
    pub categories: Vec<String>,
    /// Assignee user ids (`addAssignee`).
    pub assignees: Vec<String>,
    /// User ids to request approvals from (`addApprovee`).
    pub approvees: Vec<String>,
    /// Hyperlinks (`addHyperlink`).
    pub hyperlinks: Vec<NewHyperlink>,
    /// Links to other work items (`addLinkedItem`, or `addLinkedItemWithRev` with a revision or
    /// `suspect`).
    pub links: Vec<NewLink>,
    /// Planning constraints (`addPlaningContraint`).
    pub planning_constraints: Vec<NewPlanningConstraint>,
    /// URIs of plans to add the work item to (`addPlanItems`, one request per plan).
    pub planned_in: Vec<String>,
    /// Test steps, one cell per column of [`crate::TestRuns::steps_configuration`]
    /// (`setTestSteps`, one request for all).
    pub test_steps: Vec<Vec<Text>>,
    /// Links to work items on other servers (`addExternallyLinkedItem`).
    pub external_links: Vec<NewExternalLink>,
    /// Links to OSLC resources (`addLinkedOslcItem`).
    pub oslc_links: Vec<NewOslcLink>,
    /// Source repository revisions (`addLinkedRevision`, `addExternalLinkedRevision`).
    pub linked_revisions: Vec<NewLinkedRevision>,
}

impl NewWorkItem {
    pub fn new(
        project: impl Into<String>,
        kind: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        Self {
            project: project.into(),
            kind: kind.into(),
            title: Some(title.into()),
            ..Self::default()
        }
    }

    /// The number of follow-up requests after the create.
    pub fn follow_ups(&self) -> usize {
        self.categories.len()
            + self.assignees.len()
            + self.approvees.len()
            + self.hyperlinks.len()
            + self.links.len()
            + self.planning_constraints.len()
            + self.planned_in.len()
            + usize::from(!self.test_steps.is_empty())
            + self.external_links.len()
            + self.oslc_links.len()
            + self.linked_revisions.len()
    }
}

/// A hyperlink of a [`NewWorkItem`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewHyperlink {
    /// Hyperlink role id, e.g. `ref_ext`.
    pub role: String,
    pub url: String,
}

/// A link of a [`NewWorkItem`] to another work item.
#[derive(Debug, Clone, PartialEq)]
pub struct NewLink {
    /// Link role id, e.g. `parent`.
    pub role: String,
    pub target: ItemRef,
    /// Pin the link to this revision of the target (`None`: HEAD).
    pub revision: Option<String>,
    /// Mark the link suspect.
    pub suspect: bool,
}

impl NewLink {
    /// A plain link (HEAD, not suspect).
    pub fn new(role: impl Into<String>, target: ItemRef) -> Self {
        Self {
            role: role.into(),
            target,
            revision: None,
            suspect: false,
        }
    }
}

/// A planning constraint of a [`NewWorkItem`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewPlanningConstraint {
    /// Constraint id, e.g. `startNoEarlierThan`.
    pub constraint: String,
    /// `YYYY-MM-DD`.
    pub date: String,
}

/// A link of a [`NewWorkItem`] to a work item on another server.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewExternalLink {
    /// Link role id.
    pub role: String,
    pub url: String,
}

/// A link of a [`NewWorkItem`] to an OSLC resource.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewOslcLink {
    /// Link role id.
    pub role: String,
    pub url: String,
    pub label: String,
}

/// A source repository revision of a [`NewWorkItem`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewLinkedRevision {
    /// Repository id; `None` for the default repository.
    pub repository: Option<String>,
    pub revision: String,
}

/// Changes for [`crate::WorkItems::update`]. Fields left `None` are not touched.
///
/// The status is changed through workflow actions ([`crate::WorkItems::transition`]),
/// so the workflow's conditions and functions apply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkItemUpdate {
    pub title: Option<String>,
    pub description: Option<Text>,
    pub priority: Option<String>,
    pub severity: Option<String>,
    pub resolution: Option<String>,
    pub due_date: Option<String>,
    pub initial_estimate: Option<String>,
    pub remaining_estimate: Option<String>,
    pub time_spent: Option<String>,
    /// Custom fields to set (`Some`) or clear (`None`).
    pub custom_fields: BTreeMap<String, Option<FieldValue>>,
    /// Standard fields to clear, by field key (e.g. `dueDate`, `severity`).
    pub clear: Vec<String>,
}

impl WorkItemUpdate {
    /// Whether any standard field (sent with `updateWorkItem`) is set.
    pub(crate) fn has_standard_fields(&self) -> bool {
        self.title.is_some()
            || self.description.is_some()
            || self.priority.is_some()
            || self.severity.is_some()
            || self.resolution.is_some()
            || self.due_date.is_some()
            || self.initial_estimate.is_some()
            || self.remaining_estimate.is_some()
            || self.time_spent.is_some()
    }
}

/// A time record for [`crate::WorkItems::add_work_record`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewWorkRecord {
    /// User id.
    pub user: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    /// Duration in Polarion's format, e.g. `1d 2h` or `30m`.
    pub time_spent: String,
    /// Work record type id; required when there is a comment.
    pub kind: Option<String>,
    pub comment: Option<String>,
}

/// A document to create with [`crate::Documents::create`].
///
/// `createDocument` carries the fields up to `body`. The type, status and custom fields are
/// written by an `updateModule` right after the create: as one transaction, in a Polarion
/// session of its own, so a workflow that requires one of them sees it, and a failure leaves
/// nothing behind.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewDocument {
    pub project: String,
    /// Space id; `None` for the default space.
    pub space: Option<String>,
    pub name: String,
    pub title: Option<String>,
    /// Allowed work item type ids (Polarion accepts one).
    pub allowed_types: Vec<String>,
    /// Link role that structures the document's work items, e.g. `parent`.
    pub structure_link_role: String,
    /// Initial content (HTML).
    pub body: Option<String>,
    /// Document type id.
    pub kind: Option<String>,
    pub status: Option<String>,
    pub custom_fields: BTreeMap<String, FieldValue>,
}

impl NewDocument {
    /// The fields written after the create, if any.
    pub(crate) fn follow_up(&self) -> Option<DocumentUpdate> {
        let update = DocumentUpdate {
            kind: self.kind.clone(),
            status: self.status.clone(),
            custom_fields: self.custom_fields.clone(),
            ..DocumentUpdate::default()
        };
        (update != DocumentUpdate::default()).then_some(update)
    }
}

/// Changes for [`crate::Documents::update`]. Fields left `None` are not touched.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocumentUpdate {
    pub title: Option<String>,
    /// Document type id.
    pub kind: Option<String>,
    pub status: Option<String>,
    /// The whole document body (Polarion HTML).
    pub body: Option<Text>,
    pub structure_link_role: Option<String>,
    pub uses_outline_numbering: Option<bool>,
    pub custom_fields: BTreeMap<String, FieldValue>,
}

/// Target of [`crate::Documents::reuse`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReuseDocument {
    pub project: String,
    /// Space id; `None` for the default space.
    pub space: Option<String>,
    pub name: String,
    pub title: Option<String>,
    /// Also change the first title heading in the content to `title`.
    pub update_title_in_document: bool,
    /// Role of the links from reused work items to their sources; `None`: no links.
    pub link_role: Option<String>,
    /// Fields kept in sync with the source document.
    pub derived_fields: Vec<String>,
}

/// Changes for [`crate::TestRuns::update`]. Fields left `None` are not touched.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TestRunUpdate {
    pub title: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub group_id: Option<String>,
    pub query: Option<String>,
    pub select_test_cases_by: Option<String>,
    pub keep_in_history: Option<bool>,
    pub finished_on: Option<String>,
    pub summary_defect: Option<ItemRef>,
    pub custom_fields: BTreeMap<String, FieldValue>,
}

impl TestRunUpdate {
    /// Whether nothing is changed.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A test run to create with [`crate::TestRuns::create`].
///
/// `createTestRun` (`createTestRunWithTitle` with a title) carries the project, id, template and
/// title. The other `fields` are written by an `updateTestRun` right after the create: as one
/// transaction, in a Polarion session of its own, so a workflow that requires one of them sees
/// it, and a failure leaves nothing behind. `fields.title` must be `None` (use `title`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewTestRun {
    pub project: String,
    pub id: String,
    /// Test run template id.
    pub template: String,
    /// `None`: the template's.
    pub title: Option<String>,
    pub fields: TestRunUpdate,
}

impl NewTestRun {
    pub fn new(
        project: impl Into<String>,
        id: impl Into<String>,
        template: impl Into<String>,
    ) -> Self {
        Self {
            project: project.into(),
            id: id.into(),
            template: template.into(),
            ..Self::default()
        }
    }
}

/// A plan to create with [`crate::Plans::create`].
///
/// `createPlan` carries the project, id, name, parent and template. The other `fields` are
/// written by an `updatePlan` right after the create, as one transaction in a Polarion session of
/// its own. `fields.name` must be `None` (use `name`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewPlan {
    pub project: String,
    pub id: String,
    pub name: String,
    /// Parent plan id.
    pub parent: Option<String>,
    /// Plan template id.
    pub template: String,
    pub fields: PlanUpdate,
}

impl NewPlan {
    pub fn new(
        project: impl Into<String>,
        id: impl Into<String>,
        name: impl Into<String>,
        template: impl Into<String>,
    ) -> Self {
        Self {
            project: project.into(),
            id: id.into(),
            name: name.into(),
            template: template.into(),
            ..Self::default()
        }
    }
}

impl PlanUpdate {
    /// Whether nothing is changed.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Changes for [`crate::Plans::update`]. Fields left `None` are not touched.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlanUpdate {
    pub name: Option<String>,
    pub status: Option<String>,
    pub description: Option<Text>,
    pub start_date: Option<String>,
    pub due_date: Option<String>,
    pub capacity: Option<f32>,
    pub default_estimate: Option<f32>,
    pub estimation_field: Option<String>,
    pub prioritization_field: Option<String>,
    pub color: Option<String>,
    pub sort_order: Option<i32>,
    pub custom_fields: BTreeMap<String, FieldValue>,
}

/// Changes for [`crate::Users::update`]. Fields left `None` are not touched.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserUpdate {
    pub name: Option<String>,
    pub email: Option<String>,
    pub description: Option<Text>,
    pub disabled_notifications: Option<bool>,
}

impl FieldValue {
    /// Type a JSON value for a custom field according to its definition.
    ///
    /// `null` means "clear the field" and gives `None`. Enum fields take option ids
    /// (an array for multi-valued fields); rich text takes a string (sent as
    /// `text/plain`) or `{"type": "text/html", "content": ".."}`.
    pub fn coerce(value: &Value, field: &FieldDefinition) -> Result<Option<FieldValue>> {
        if value.is_null() {
            return Ok(None);
        }
        let kind = field.kind.as_deref().unwrap_or("string");
        let kind = kind.rsplit([':', '}']).next().unwrap_or(kind);
        let invalid = |expected: &str| {
            Error::Invalid(format!(
                "custom field `{}` ({kind}) expects {expected}, got {value}",
                field.id
            ))
        };
        let text = |v: &Value| match v {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        };
        let one = |v: &Value| -> Result<FieldValue> {
            Ok(match kind.to_ascii_lowercase().as_str() {
                "string" => FieldValue::String(text(v).ok_or_else(|| invalid("a string"))?),
                "enumoptionid" | "priorityoptionid" => {
                    FieldValue::Enum(text(v).ok_or_else(|| invalid("an option id"))?)
                }
                "text" => match v {
                    Value::String(s) => FieldValue::Text(Text {
                        content_type: "text/plain".into(),
                        content: s.clone(),
                    }),
                    Value::Object(map) => FieldValue::Text(Text {
                        content_type: map
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("text/plain")
                            .to_owned(),
                        content: map
                            .get("content")
                            .and_then(Value::as_str)
                            .ok_or_else(|| invalid("a string or {type, content}"))?
                            .to_owned(),
                    }),
                    _ => return Err(invalid("a string or {type, content}")),
                },
                "boolean" => match v {
                    Value::Bool(b) => FieldValue::Bool(*b),
                    Value::String(s) if s == "true" || s == "false" => {
                        FieldValue::Bool(s == "true")
                    }
                    _ => return Err(invalid("true or false")),
                },
                "int" | "integer" | "long" | "short" => FieldValue::Int(
                    match v {
                        Value::Number(n) => n.as_i64(),
                        Value::String(s) => s.trim().parse().ok(),
                        _ => None,
                    }
                    .ok_or_else(|| invalid("an integer"))?,
                ),
                "float" | "double" => FieldValue::Float(
                    match v {
                        Value::Number(n) => n.as_f64(),
                        Value::String(s) => s.trim().parse().ok(),
                        _ => None,
                    }
                    .ok_or_else(|| invalid("a number"))?,
                ),
                "decimal" | "currency" => {
                    FieldValue::Decimal(text(v).ok_or_else(|| invalid("a number"))?)
                }
                "date" => FieldValue::Date(text(v).ok_or_else(|| invalid("a date (YYYY-MM-DD)"))?),
                "datetime" => FieldValue::DateTime(
                    text(v).ok_or_else(|| invalid("a date and time (ISO 8601)"))?,
                ),
                "time" => FieldValue::Time(text(v).ok_or_else(|| invalid("a time (HH:MM:SS)"))?),
                other => {
                    return Err(Error::Invalid(format!(
                        "custom field `{}` has type `{other}`, which cannot be written yet",
                        field.id
                    )));
                }
            })
        };
        if field.multi {
            let items: Vec<&Value> = match value {
                Value::Array(items) => items.iter().collect(),
                single => vec![single],
            };
            Ok(Some(FieldValue::List(
                items.into_iter().map(one).collect::<Result<_>>()?,
            )))
        } else {
            one(value).map(Some)
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn field(kind: &str, multi: bool) -> FieldDefinition {
        FieldDefinition {
            id: "f".into(),
            kind: Some(kind.into()),
            multi,
            ..FieldDefinition::default()
        }
    }

    #[test]
    fn values_are_typed_by_the_definition() {
        let c = |v: Value, kind: &str, multi: bool| FieldValue::coerce(&v, &field(kind, multi));
        assert_eq!(
            c(json!("D"), "EnumOptionId", false).unwrap(),
            Some(FieldValue::Enum("D".into()))
        );
        assert_eq!(
            c(json!(["a", "b"]), "EnumOptionId", true).unwrap(),
            Some(FieldValue::List(vec![
                FieldValue::Enum("a".into()),
                FieldValue::Enum("b".into())
            ]))
        );
        assert_eq!(
            c(json!("a"), "EnumOptionId", true).unwrap(),
            Some(FieldValue::List(vec![FieldValue::Enum("a".into())]))
        );
        assert_eq!(
            c(json!("x < y"), "Text", false).unwrap(),
            Some(FieldValue::Text(Text {
                content_type: "text/plain".into(),
                content: "x < y".into()
            }))
        );
        assert_eq!(
            c(
                json!({"type": "text/html", "content": "<b>x</b>"}),
                "Text",
                false
            )
            .unwrap(),
            Some(FieldValue::Text(Text {
                content_type: "text/html".into(),
                content: "<b>x</b>".into()
            }))
        );
        assert_eq!(
            c(json!("3"), "int", false).unwrap(),
            Some(FieldValue::Int(3))
        );
        assert_eq!(
            c(json!(1.5), "float", false).unwrap(),
            Some(FieldValue::Float(1.5))
        );
        assert_eq!(
            c(json!(true), "boolean", false).unwrap(),
            Some(FieldValue::Bool(true))
        );
        assert_eq!(
            c(json!("2024-01-31"), "xsd:date", false).unwrap(),
            Some(FieldValue::Date("2024-01-31".into()))
        );
        assert_eq!(c(json!(null), "date", false).unwrap(), None);
    }

    #[test]
    fn mismatches_are_rejected() {
        let err = FieldValue::coerce(&json!("x"), &field("boolean", false)).unwrap_err();
        assert!(err.to_string().contains("expects true or false"), "{err}");
        assert!(FieldValue::coerce(&json!(1), &field("Table", false)).is_err());
        assert!(FieldValue::coerce(&json!({}), &field("string", false)).is_err());
    }
}
