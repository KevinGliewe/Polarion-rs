//! Mappings between the generated SOAP types and the domain types.

use std::collections::BTreeMap;

use polarion_soap::types::{self as soap, AnyValue};

use crate::error::{Error, Result};
use crate::model::{
    Approval, Attachment, Baseline, Category, Change, Comment, Document, EnumOption, FieldChange,
    FieldDefinition, FieldValue, Hyperlink, Link, LinkedRevision, Plan, Project, ProjectGroup,
    Revision, TestRecord, TestRun, TestStepResult, TestSteps, Text, TimePoint, User, WikiPage,
    WorkItem, WorkRecord, WorkflowAction,
};
use crate::uri::{self, DocumentRef, ItemRef};
use crate::write::{NewWorkItem, WorkItemUpdate};

fn enum_id(e: Option<soap::EnumOptionId>) -> Option<String> {
    e.and_then(|e| e.id)
}

fn user_id(u: Option<soap::User>) -> Option<String> {
    let u = u?;
    u.id.or_else(|| u.uri.as_deref().and_then(uri::user_id).map(str::to_owned))
}

fn text(t: Option<soap::Text>) -> Option<Text> {
    t.map(Text::from)
}

impl From<soap::Text> for Text {
    fn from(t: soap::Text) -> Self {
        Self {
            content_type: t.type_,
            content: t.content,
        }
    }
}

/// Custom fields with a value, by key.
pub(crate) fn custom_fields(fields: Option<Vec<soap::Custom>>) -> BTreeMap<String, FieldValue> {
    fields
        .into_iter()
        .flatten()
        .filter_map(|c| Some((c.key, field_value(c.value?)?)))
        .collect()
}

/// `None` for nil and for enum options without an id.
pub(crate) fn field_value(value: AnyValue) -> Option<FieldValue> {
    Some(match value {
        AnyValue::Nil => return None,
        AnyValue::String(s) | AnyValue::QName(s) => FieldValue::String(s),
        AnyValue::Boolean(b) => FieldValue::Bool(b),
        AnyValue::Int(i) => FieldValue::Int(i.into()),
        AnyValue::Long(i) => FieldValue::Int(i),
        // Through the shortest decimal form, so 0.1f32 stays 0.1 instead of 0.10000000149.
        AnyValue::Float(f) => FieldValue::Float(f.to_string().parse().unwrap_or(f.into())),
        AnyValue::Double(f) => FieldValue::Float(f),
        AnyValue::Decimal(s) => FieldValue::Decimal(s),
        AnyValue::Date(s) => FieldValue::Date(s),
        AnyValue::DateTime(s) => FieldValue::DateTime(s),
        AnyValue::Time(s) => FieldValue::Time(s),
        AnyValue::EnumOptionId(e) => FieldValue::Enum(e.id?),
        AnyValue::PriorityOptionId(e) => FieldValue::Enum(e.id?),
        AnyValue::User(u) => FieldValue::Enum(user_id(Some(*u))?),
        AnyValue::Text(t) => FieldValue::Text((*t).into()),
        AnyValue::ArrayOfEnumOptionId(v) => FieldValue::List(
            v.into_iter()
                .filter_map(|e| e.id)
                .map(FieldValue::Enum)
                .collect(),
        ),
        AnyValue::ArrayOfPriorityOptionId(v) => FieldValue::List(
            v.into_iter()
                .filter_map(|e| e.id)
                .map(FieldValue::Enum)
                .collect(),
        ),
        AnyValue::ArrayOfXsdString(v) | AnyValue::ArrayOfstring(v) => {
            FieldValue::List(v.into_iter().map(FieldValue::String).collect())
        }
        AnyValue::ArrayOfText(v) => {
            FieldValue::List(v.into_iter().map(|t| FieldValue::Text(t.into())).collect())
        }
        AnyValue::ArrayOfXsdAnyType(v) => {
            FieldValue::List(v.into_iter().filter_map(field_value).collect())
        }
        AnyValue::Unknown { xsi_type, xml } => FieldValue::Other {
            xsi_type,
            xml: Some(xml),
        },
        other => FieldValue::Other {
            xsi_type: Some(variant_name(&other)),
            xml: None,
        },
    })
}

/// Name of a generated `AnyValue` variant, i.e. its schema type name.
fn variant_name(value: &AnyValue) -> String {
    let debug = format!("{value:?}");
    debug
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or_default()
        .to_owned()
}

pub(crate) fn link(l: soap::LinkedWorkItem) -> Option<Link> {
    Some(Link {
        role: enum_id(l.role),
        item: ItemRef::from_uri(l.work_item_uri?),
        suspect: l.suspect.unwrap_or(false),
        revision: l.revision.filter(|r| !r.is_empty()),
    })
}

impl From<soap::WorkItem> for WorkItem {
    fn from(w: soap::WorkItem) -> Self {
        let parsed = w.uri.as_deref().map(ItemRef::from_uri);
        Self {
            id: w.id.or_else(|| parsed.as_ref().and_then(|r| r.id.clone())),
            project: w
                .project
                .and_then(|p| p.id)
                .or_else(|| parsed.as_ref().and_then(|r| r.project.clone())),
            kind: enum_id(w.type_),
            title: w.title,
            status: enum_id(w.status),
            priority: w.priority.and_then(|p| p.id),
            severity: enum_id(w.severity),
            resolution: enum_id(w.resolution),
            description: text(w.description),
            author: user_id(w.author),
            assignees: w
                .assignee
                .into_iter()
                .flatten()
                .filter_map(|u| user_id(Some(u)))
                .collect(),
            created: w.created,
            updated: w.updated,
            due_date: w.due_date,
            resolved_on: w.resolved_on,
            planned_start: w.planned_start,
            planned_end: w.planned_end,
            document: w.module_uri.map(DocumentRef::from_uri),
            outline_number: w.outline_number,
            categories: w
                .categories
                .into_iter()
                .flatten()
                .filter_map(|c| c.id)
                .collect(),
            links: w
                .linked_work_items
                .into_iter()
                .flatten()
                .filter_map(link)
                .collect(),
            hyperlinks: w
                .hyperlinks
                .into_iter()
                .flatten()
                .filter_map(|h| {
                    Some(Hyperlink {
                        role: enum_id(h.role),
                        url: h.uri?,
                    })
                })
                .collect(),
            external_links: w
                .externally_linked_work_items
                .into_iter()
                .flatten()
                .filter_map(|h| {
                    Some(Hyperlink {
                        role: enum_id(h.role),
                        url: h.work_item_uri?,
                    })
                })
                .collect(),
            linked_revisions: w
                .linked_revisions
                .into_iter()
                .flatten()
                .filter_map(|r| {
                    Some(LinkedRevision {
                        repository: r.repository_name,
                        revision: r.name?,
                    })
                })
                .collect(),
            comments: w
                .comments
                .into_iter()
                .flatten()
                .map(Comment::from)
                .collect(),
            attachments: w
                .attachments
                .into_iter()
                .flatten()
                .map(Attachment::from)
                .collect(),
            approvals: w
                .approvals
                .into_iter()
                .flatten()
                .map(|a| Approval {
                    user: user_id(a.user),
                    status: enum_id(a.status),
                })
                .collect(),
            work_records: w
                .work_records
                .into_iter()
                .flatten()
                .map(WorkRecord::from)
                .collect(),
            time_point: w.time_point.and_then(|t| t.id),
            planned_in: w.planned_in_ur_is.unwrap_or_default(),
            initial_estimate: w.initial_estimate,
            remaining_estimate: w.remaining_estimate,
            time_spent: w.time_spent,
            previous_status: enum_id(w.previous_status),
            location: w.location,
            custom_fields: custom_fields(w.custom_fields),
            uri: w.uri,
        }
    }
}

impl From<soap::Comment> for Comment {
    fn from(c: soap::Comment) -> Self {
        Self {
            id: c.id,
            author: user_id(c.author),
            created: c.created,
            title: c.title,
            text: text(c.text),
            resolved: c.resolved.unwrap_or(false),
            tags: c.tags.into_iter().flatten().filter_map(|t| t.id).collect(),
            parent: c.parent_comment_uri,
            replies: c.child_comment_ur_is.unwrap_or_default(),
            work_item: None,
            uri: c.uri,
        }
    }
}

impl From<soap::ModuleComment> for Comment {
    fn from(c: soap::ModuleComment) -> Self {
        Self {
            id: c.id,
            author: user_id(c.author),
            created: c.created,
            title: None,
            text: text(c.text),
            resolved: c.resolved.unwrap_or(false),
            tags: c.tags.into_iter().flatten().filter_map(|t| t.id).collect(),
            parent: c.parent_comment_uri,
            replies: c.child_comment_ur_is.unwrap_or_default(),
            work_item: c.referred_work_item_uri.map(ItemRef::from_uri),
            uri: c.uri,
        }
    }
}

impl From<soap::TestRunComment> for Comment {
    fn from(c: soap::TestRunComment) -> Self {
        Self {
            id: c.id,
            author: user_id(c.author),
            created: c.created,
            title: c.title,
            text: text(c.text),
            resolved: c.resolved.unwrap_or(false),
            tags: c.tags.into_iter().flatten().filter_map(|t| t.id).collect(),
            parent: c.parent_comment_uri,
            replies: c.child_comment_ur_is.unwrap_or_default(),
            work_item: None,
            uri: c.uri,
        }
    }
}

impl From<soap::Attachment> for Attachment {
    fn from(a: soap::Attachment) -> Self {
        Self {
            id: a.id,
            file_name: a.file_name,
            title: a.title,
            length: a.length,
            author: user_id(a.author),
            updated: a.updated,
            url: a.url,
        }
    }
}

impl From<soap::TestRunAttachment> for Attachment {
    fn from(a: soap::TestRunAttachment) -> Self {
        Self {
            id: a.id,
            file_name: a.file_name,
            title: a.title,
            length: a.length,
            author: user_id(a.author),
            updated: a.updated,
            url: a.url,
        }
    }
}

impl From<soap::WikiPageAttachment> for Attachment {
    fn from(a: soap::WikiPageAttachment) -> Self {
        Self {
            id: a.id,
            file_name: a.file_name,
            title: a.title,
            length: a.length,
            author: user_id(a.author),
            updated: a.updated,
            url: a.url,
        }
    }
}

impl From<soap::WorkRecord> for WorkRecord {
    fn from(r: soap::WorkRecord) -> Self {
        Self {
            id: r.id,
            user: user_id(r.user),
            date: r.date,
            time_spent: r.time_spent,
            kind: enum_id(r.type_),
            comment: r.comment,
            uri: r.uri,
        }
    }
}

impl From<soap::WorkflowAction> for WorkflowAction {
    fn from(a: soap::WorkflowAction) -> Self {
        Self {
            id: a.action_id,
            name: a.action_name,
            native_id: a.native_action_id,
            target_status: enum_id(a.target_status),
            required_fields: a.required_features.unwrap_or_default(),
            message: a.unavailability_message.filter(|m| !m.is_empty()),
        }
    }
}

impl From<soap::Change> for Change {
    fn from(c: soap::Change) -> Self {
        Self {
            revision: c.revision,
            date: c.date,
            user: c.user,
            creation: c.creation,
            changes: c
                .diffs
                .into_iter()
                .flatten()
                .map(|d| FieldChange {
                    field: d.field_name.unwrap_or_default(),
                    before: d.before.and_then(field_value),
                    after: d.after.and_then(field_value),
                    added: d
                        .added
                        .into_iter()
                        .flatten()
                        .filter_map(field_value)
                        .collect(),
                    removed: d
                        .removed
                        .into_iter()
                        .flatten()
                        .filter_map(field_value)
                        .collect(),
                })
                .collect(),
        }
    }
}

impl From<soap::CustomFieldType> for FieldDefinition {
    fn from(f: soap::CustomFieldType) -> Self {
        Self {
            id: f.id.unwrap_or_default(),
            name: f.name,
            // An `xsd:QName` such as `ns2:EnumOptionId`: the prefix is meaningless here.
            kind: f
                .type_
                .map(|t| t.rsplit([':', '}']).next().unwrap_or(&t).to_owned()),
            multi: f.multi,
            required: f.required,
            enum_id: f.enum_id,
            description: f.description,
            depends_on: f.depends_on,
            default_value: f.default_value.and_then(field_value),
        }
    }
}

impl From<soap::EnumOption> for EnumOption {
    fn from(o: soap::EnumOption) -> Self {
        Self {
            id: o.id.unwrap_or_default(),
            name: o.name,
            default: o.default_,
            hidden: o.hidden,
            sequence: o.sequence_number,
            properties: o
                .properties
                .into_iter()
                .flatten()
                .filter_map(|p| Some((p.key, p.value?)))
                .collect(),
        }
    }
}

impl From<soap::Category> for Category {
    fn from(c: soap::Category) -> Self {
        Self {
            id: c.id,
            name: c.name,
            description: text(c.description),
        }
    }
}

impl From<soap::TimePoint> for TimePoint {
    fn from(t: soap::TimePoint) -> Self {
        Self {
            id: t.id,
            name: t.name,
            time: t.time,
            closed: t.closed.unwrap_or(false),
            description: text(t.description),
        }
    }
}

// ---- domain → SOAP (writes) ----

pub(crate) fn soap_text(t: &Text) -> soap::Text {
    soap::Text {
        type_: t.content_type.clone(),
        content: t.content.clone(),
        content_lossy: false,
    }
}

pub(crate) fn option_id(id: &str) -> soap::EnumOptionId {
    soap::EnumOptionId {
        id: Some(id.to_owned()),
    }
}

/// The SOAP form of a custom field value.
pub(crate) fn any_value(value: &FieldValue) -> Result<AnyValue> {
    Ok(match value {
        FieldValue::String(s) => AnyValue::String(s.clone()),
        FieldValue::Bool(b) => AnyValue::Boolean(*b),
        FieldValue::Int(i) => match i32::try_from(*i) {
            Ok(i) => AnyValue::Int(i),
            Err(_) => AnyValue::Long(*i),
        },
        // Polarion's float custom fields are `xsd:float`.
        FieldValue::Float(f) => AnyValue::Float(*f as f32),
        FieldValue::Decimal(s) => AnyValue::Decimal(s.clone()),
        FieldValue::Date(s) => AnyValue::Date(s.clone()),
        FieldValue::DateTime(s) => AnyValue::DateTime(s.clone()),
        FieldValue::Time(s) => AnyValue::Time(s.clone()),
        FieldValue::Enum(id) => AnyValue::EnumOptionId(Box::new(option_id(id))),
        FieldValue::Text(t) => AnyValue::Text(Box::new(soap_text(t))),
        FieldValue::List(items) => {
            if items.iter().all(|v| matches!(v, FieldValue::Enum(_))) {
                AnyValue::ArrayOfEnumOptionId(
                    items
                        .iter()
                        .filter_map(|v| match v {
                            FieldValue::Enum(id) => Some(option_id(id)),
                            _ => None,
                        })
                        .collect(),
                )
            } else if items.iter().all(|v| matches!(v, FieldValue::String(_))) {
                AnyValue::ArrayOfXsdString(
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect(),
                )
            } else {
                AnyValue::ArrayOfXsdAnyType(items.iter().map(any_value).collect::<Result<_>>()?)
            }
        }
        FieldValue::Other { xsi_type, .. } => {
            return Err(Error::Invalid(format!(
                "values of type {} cannot be written",
                xsi_type.as_deref().unwrap_or("unknown")
            )));
        }
    })
}

/// The `createWorkItem` content. `project` is the project as returned by the server.
pub(crate) fn new_work_item(n: &NewWorkItem, project: soap::Project) -> Result<soap::WorkItem> {
    Ok(soap::WorkItem {
        project: Some(soap::Project {
            id: project.id,
            uri: project.uri,
            ..soap::Project::default()
        }),
        type_: Some(option_id(&n.kind)),
        title: n.title.clone(),
        description: n.description.as_ref().map(soap_text),
        priority: n.priority.as_ref().map(|p| soap::PriorityOptionId {
            id: Some(p.clone()),
        }),
        severity: n.severity.as_deref().map(option_id),
        due_date: n.due_date.clone(),
        initial_estimate: n.initial_estimate.clone(),
        remaining_estimate: n.remaining_estimate.clone(),
        custom_fields: if n.custom_fields.is_empty() {
            None
        } else {
            Some(
                n.custom_fields
                    .iter()
                    .map(|(key, value)| {
                        Ok(soap::Custom {
                            key: key.clone(),
                            value: Some(any_value(value)?),
                        })
                    })
                    .collect::<Result<_>>()?,
            )
        },
        ..soap::WorkItem::default()
    })
}

/// The `updateWorkItem` content: the URI plus the standard fields that are set.
pub(crate) fn work_item_update(uri: &str, u: &WorkItemUpdate) -> soap::WorkItem {
    soap::WorkItem {
        uri: Some(uri.to_owned()),
        title: u.title.clone(),
        description: u.description.as_ref().map(soap_text),
        priority: u.priority.as_ref().map(|p| soap::PriorityOptionId {
            id: Some(p.clone()),
        }),
        severity: u.severity.as_deref().map(option_id),
        resolution: u.resolution.as_deref().map(option_id),
        due_date: u.due_date.clone(),
        initial_estimate: u.initial_estimate.clone(),
        remaining_estimate: u.remaining_estimate.clone(),
        time_spent: u.time_spent.clone(),
        ..soap::WorkItem::default()
    }
}

impl From<soap::Module> for Document {
    fn from(m: soap::Module) -> Self {
        let parsed = m.uri.as_deref().map(DocumentRef::from_uri);
        let parsed = parsed.as_ref();
        Self {
            project: m
                .project
                .and_then(|p| p.id)
                .or_else(|| parsed.and_then(|r| r.project.clone())),
            space: m
                .module_folder
                .or_else(|| parsed.and_then(|r| r.space.clone())),
            name: m
                .module_name
                .or_else(|| parsed.and_then(|r| r.name.clone())),
            title: m.title,
            kind: enum_id(m.type_),
            status: enum_id(m.status),
            author: user_id(m.author),
            created: m.created,
            updated: m.updated,
            updated_by: user_id(m.updated_by),
            structure_link_role: enum_id(m.structure_link_role),
            allowed_types: m
                .allowed_wi_types
                .into_iter()
                .flatten()
                .filter_map(|t| t.id)
                .collect(),
            uses_outline_numbering: m.uses_outline_numbering,
            derived_from: m.derived_from_uri,
            comments: m
                .comments
                .into_iter()
                .flatten()
                .map(Comment::from)
                .collect(),
            custom_fields: custom_fields(m.custom_fields),
            body: text(m.home_page_content),
            uri: m.uri,
        }
    }
}

impl From<soap::Project> for Project {
    fn from(p: soap::Project) -> Self {
        Self {
            id: p.id,
            name: p.name,
            description: text(p.description),
            lead: user_id(p.lead),
            location: p.location,
            tracker_prefix: p.tracker_prefix,
            active: p.active,
            start: p.start,
            finish: p.finish,
        }
    }
}

impl From<soap::User> for User {
    fn from(u: soap::User) -> Self {
        Self {
            id: u
                .id
                .or_else(|| u.uri.as_deref().and_then(uri::user_id).map(str::to_owned)),
            name: u.name,
            email: u.email,
        }
    }
}

fn parameters(p: Option<Vec<soap::Parameter>>) -> BTreeMap<String, FieldValue> {
    p.into_iter()
        .flatten()
        .filter_map(|p| Some((p.name?, field_value(p.value?)?)))
        .collect()
}

fn user_from_uri(uri: Option<String>) -> Option<String> {
    let uri = uri?;
    Some(uri::user_id(&uri).map(str::to_owned).unwrap_or(uri))
}

fn project_from_uri(uri: Option<&str>) -> Option<String> {
    uri.and_then(uri::project_of).map(str::to_owned)
}

impl From<soap::ProjectGroup> for ProjectGroup {
    fn from(g: soap::ProjectGroup) -> Self {
        Self {
            name: g.name,
            location: g.location,
            parent: g.parent_uri,
            groups: g.group_ur_is.unwrap_or_default(),
            projects: g.project_i_ds.unwrap_or_default(),
            uri: g.uri,
        }
    }
}

impl From<soap::WikiPage> for WikiPage {
    fn from(w: soap::WikiPage) -> Self {
        Self {
            project: w
                .project
                .and_then(|p| p.id)
                .or_else(|| project_from_uri(w.uri.as_deref())),
            space: w.space_id,
            name: w.page_name.or(w.id),
            title: w.title,
            kind: w.type_,
            author: user_id(w.author),
            created: w.created,
            updated: w.updated,
            updated_by: user_id(w.updated_by),
            linked_pages: w.linked_page_ur_is.unwrap_or_default(),
            attachments: w
                .attachments
                .into_iter()
                .flatten()
                .map(Attachment::from)
                .collect(),
            body: text(w.home_page_content),
            uri: w.uri,
        }
    }
}

impl From<soap::Baseline> for Baseline {
    fn from(b: soap::Baseline) -> Self {
        Self {
            id: b.id,
            project: b
                .project
                .and_then(|p| p.id)
                .or_else(|| project_from_uri(b.uri.as_deref())),
            name: b.name,
            description: b.description,
            revision: b.base_revision,
            base_object: b.base_object_uri,
            author: user_id(b.author),
            uri: b.uri,
        }
    }
}

impl From<soap::Revision> for Revision {
    fn from(r: soap::Revision) -> Self {
        Self {
            name: r.name,
            repository: r.repository_name,
            author: r.author,
            created: r.created,
            message: r.message,
            branch: r.branch,
            internal: r.internal_commit,
            linked_work_items: r
                .linked_work_item_ur_is
                .into_iter()
                .flatten()
                .map(ItemRef::from_uri)
                .collect(),
            uri: r.uri,
        }
    }
}

impl From<soap::TestRecord> for TestRecord {
    fn from(r: soap::TestRecord) -> Self {
        Self {
            test_case: r.test_case_uri.map(ItemRef::from_uri),
            test_case_revision: r.test_case_revision,
            result: enum_id(r.result),
            comment: text(r.comment),
            executed: r.executed,
            executed_by: user_from_uri(r.executed_by_uri),
            duration: r.duration,
            iteration: r.iteration,
            defect: r.defect_uri.map(ItemRef::from_uri),
            signed: r.signed,
            parameters: parameters(r.test_parameters),
            step_results: r
                .test_step_results
                .into_iter()
                .flatten()
                .map(|s| TestStepResult {
                    result: enum_id(s.result),
                    comment: text(s.comment),
                    attachments: s
                        .attachments
                        .into_iter()
                        .flatten()
                        .map(Attachment::from)
                        .collect(),
                })
                .collect(),
            attachments: r
                .attachments
                .into_iter()
                .flatten()
                .map(Attachment::from)
                .collect(),
        }
    }
}

impl From<soap::TestRun> for TestRun {
    fn from(r: soap::TestRun) -> Self {
        Self {
            id: r.id,
            project: project_from_uri(r.project_uri.as_deref())
                .or_else(|| project_from_uri(r.uri.as_deref())),
            title: r.title,
            kind: enum_id(r.type_),
            status: enum_id(r.status),
            template: r.is_template.unwrap_or(false),
            template_uri: r.template_uri,
            author: user_from_uri(r.author_uri),
            created: r.created,
            updated: r.updated,
            finished_on: r.finished_on,
            group_id: r.group_id,
            select_test_cases_by: enum_id(r.select_test_cases_by),
            query: r.query,
            document: r.document.and_then(|d| d.uri).map(DocumentRef::from_uri),
            summary_defect: r.summary_defect_uri.map(ItemRef::from_uri),
            keep_in_history: r.keep_in_history,
            project_span: r.project_span_ur_is.unwrap_or_default(),
            parameters: parameters(r.test_parameters.and_then(|p| p.parameters)),
            records: r
                .records
                .into_iter()
                .flatten()
                .map(TestRecord::from)
                .collect(),
            attachments: r
                .attachments
                .into_iter()
                .flatten()
                .map(Attachment::from)
                .collect(),
            comments: r
                .comments
                .into_iter()
                .flatten()
                .map(Comment::from)
                .collect(),
            custom_fields: custom_fields(r.custom_fields),
            body: text(r.home_page_content),
            uri: r.uri,
        }
    }
}

impl From<soap::TestSteps> for TestSteps {
    fn from(t: soap::TestSteps) -> Self {
        Self {
            columns: t.keys.into_iter().flatten().filter_map(|k| k.id).collect(),
            steps: t
                .steps
                .into_iter()
                .flatten()
                .map(|s| s.values.into_iter().flatten().map(Text::from).collect())
                .collect(),
        }
    }
}

impl From<soap::Plan> for Plan {
    fn from(p: soap::Plan) -> Self {
        Self {
            id: p.id,
            project: project_from_uri(p.project_uri.as_deref())
                .or_else(|| project_from_uri(p.uri.as_deref())),
            name: p.name,
            status: enum_id(p.status),
            template: p.is_template.unwrap_or(false),
            parent: p.parent.and_then(|parent| parent.uri),
            template_uri: p.template_uri,
            description: text(p.description),
            start_date: p.start_date,
            due_date: p.due_date,
            started_on: p.started_on,
            finished_on: p.finished_on,
            capacity: p.capacity,
            default_estimate: p.default_estimate,
            estimation_field: p.estimation_field,
            prioritization_field: p.prioritization_field,
            calculation_type: enum_id(p.calculation_type),
            color: p.color,
            sort_order: p.sort_order,
            allowed_types: p
                .allowed_types
                .into_iter()
                .flatten()
                .filter_map(|t| t.id)
                .collect(),
            project_span: p.project_span_ur_is.unwrap_or_default(),
            items: p
                .records
                .into_iter()
                .flatten()
                .filter_map(|r| r.item?.uri)
                .map(ItemRef::from_uri)
                .collect(),
            author: user_from_uri(p.author_uri),
            created: p.created,
            updated: p.updated,
            custom_fields: custom_fields(p.custom_fields),
            body: text(p.home_page_content),
            uri: p.uri,
        }
    }
}

// ---- domain → SOAP for the other areas

pub(crate) fn soap_custom_fields(
    fields: &BTreeMap<String, FieldValue>,
) -> Result<Option<Vec<soap::Custom>>> {
    if fields.is_empty() {
        return Ok(None);
    }
    fields
        .iter()
        .map(|(key, value)| {
            Ok(soap::Custom {
                key: key.clone(),
                value: Some(any_value(value)?),
            })
        })
        .collect::<Result<_>>()
        .map(Some)
}

/// A test record to send. `executed_by` is a user id; attachments are ignored
/// (add them with the attachment methods).
pub(crate) fn soap_test_record(r: &TestRecord) -> Result<soap::TestRecord> {
    Ok(soap::TestRecord {
        test_case_uri: r.test_case.as_ref().map(|t| t.uri.clone()),
        test_case_revision: r.test_case_revision.clone(),
        result: r.result.as_deref().map(option_id),
        comment: r.comment.as_ref().map(soap_text),
        executed: r.executed.clone(),
        executed_by_uri: r.executed_by.as_deref().map(uri::user_uri),
        duration: r.duration,
        iteration: r.iteration,
        defect_uri: r.defect.as_ref().map(|d| d.uri.clone()),
        signed: r.signed,
        test_parameters: if r.parameters.is_empty() {
            None
        } else {
            Some(
                r.parameters
                    .iter()
                    .map(|(name, value)| {
                        Ok(soap::Parameter {
                            name: Some(name.clone()),
                            value: Some(any_value(value)?),
                        })
                    })
                    .collect::<Result<_>>()?,
            )
        },
        test_step_results: if r.step_results.is_empty() {
            None
        } else {
            Some(
                r.step_results
                    .iter()
                    .map(|s| soap::TestStepResult {
                        result: s.result.as_deref().map(option_id),
                        comment: s.comment.as_ref().map(soap_text),
                        attachments: None,
                    })
                    .collect(),
            )
        },
        attachments: None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn opt(id: &str) -> Option<soap::EnumOptionId> {
        Some(soap::EnumOptionId {
            id: Some(id.into()),
        })
    }

    fn custom(key: &str, value: AnyValue) -> soap::Custom {
        soap::Custom {
            key: key.into(),
            value: Some(value),
        }
    }

    #[test]
    fn work_item_json_is_flat() {
        let item = soap::WorkItem {
            id: Some("P-1".into()),
            project: Some(soap::Project {
                id: Some("P".into()),
                ..Default::default()
            }),
            type_: opt("requirement"),
            title: Some("Brakes".into()),
            status: opt("open"),
            priority: Some(soap::PriorityOptionId {
                id: Some("90.0".into()),
            }),
            description: Some(soap::Text {
                type_: "text/html".into(),
                content: "<p>x</p>".into(),
                content_lossy: false,
            }),
            author: Some(soap::User {
                uri: Some("subterra:data-service:objects:/default/${User}jdoe".into()),
                ..Default::default()
            }),
            assignee: Some(vec![soap::User {
                id: Some("amy".into()),
                ..Default::default()
            }]),
            module_uri: Some(
                "subterra:data-service:objects:/default/P${Module}{moduleFolder}Specs#SRS".into(),
            ),
            linked_work_items: Some(vec![
                soap::LinkedWorkItem {
                    role: opt("parent"),
                    suspect: Some(true),
                    work_item_uri: Some(
                        "subterra:data-service:objects:/default/P${WorkItem}P-0".into(),
                    ),
                    revision: None,
                },
                soap::LinkedWorkItem::default(),
            ]),
            hyperlinks: Some(vec![soap::Hyperlink {
                role: opt("ref"),
                uri: Some("https://example.com".into()),
            }]),
            custom_fields: Some(vec![
                custom(
                    "asil",
                    AnyValue::EnumOptionId(Box::new(soap::EnumOptionId {
                        id: Some("D".into()),
                    })),
                ),
                custom(
                    "platforms",
                    AnyValue::ArrayOfEnumOptionId(vec![
                        soap::EnumOptionId {
                            id: Some("a".into()),
                        },
                        soap::EnumOptionId {
                            id: Some("b".into()),
                        },
                    ]),
                ),
                custom("effort", AnyValue::Float(0.1)),
                custom("count", AnyValue::Int(3)),
                custom("reviewed", AnyValue::Date("2024-01-31".into())),
                custom("safe", AnyValue::Boolean(false)),
                custom("empty", AnyValue::Nil),
                custom(
                    "exotic",
                    AnyValue::Unknown {
                        xsi_type: Some("{urn:x}Mystery".into()),
                        xml: "<v/>".into(),
                    },
                ),
                custom("table", AnyValue::Table(Box::default())),
            ]),
            uri: Some("subterra:data-service:objects:/default/P${WorkItem}P-1".into()),
            ..Default::default()
        };

        let json = serde_json::to_value(WorkItem::from(item)).unwrap();
        assert_eq!(
            json,
            json!({
                "id": "P-1",
                "project": "P",
                "type": "requirement",
                "title": "Brakes",
                "status": "open",
                "priority": "90.0",
                "description": {"type": "text/html", "content": "<p>x</p>"},
                "author": "jdoe",
                "assignees": ["amy"],
                "document": {"project": "P", "space": "Specs", "name": "SRS"},
                "links": [{"role": "parent", "item": {"project": "P", "id": "P-0"}, "suspect": true}],
                "hyperlinks": [{"role": "ref", "url": "https://example.com"}],
                "customFields": {
                    "asil": "D",
                    "count": 3,
                    "effort": 0.1,
                    "exotic": {"xsiType": "{urn:x}Mystery", "xml": "<v/>"},
                    "platforms": ["a", "b"],
                    "reviewed": "2024-01-31",
                    "safe": false,
                    "table": {"xsiType": "Table"},
                },
                "uri": "subterra:data-service:objects:/default/P${WorkItem}P-1",
            })
        );
    }

    #[test]
    fn ids_fall_back_to_the_uri() {
        let item = WorkItem::from(soap::WorkItem {
            uri: Some("subterra:data-service:objects:/default/P${WorkItem}P-7".into()),
            ..Default::default()
        });
        assert_eq!(item.id.as_deref(), Some("P-7"));
        assert_eq!(item.project.as_deref(), Some("P"));

        let doc = Document::from(soap::Module {
            uri: Some(
                "subterra:data-service:objects:/default/P${Module}{moduleFolder}_default#Spec"
                    .into(),
            ),
            ..Default::default()
        });
        assert_eq!(
            (
                doc.project.as_deref(),
                doc.space.as_deref(),
                doc.name.as_deref()
            ),
            (Some("P"), Some("_default"), Some("Spec"))
        );
    }
}
