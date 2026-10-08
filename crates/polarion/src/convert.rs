//! Mappings from the generated SOAP types to the domain types.

use std::collections::BTreeMap;

use polarion_soap::types::{self as soap, AnyValue};

use crate::model::{Document, FieldValue, Hyperlink, Link, Project, Text, User, WorkItem};
use crate::uri::{self, DocumentRef, ItemRef};

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
            custom_fields: custom_fields(w.custom_fields),
            uri: w.uri,
        }
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
                "subterra:data-service:objects:/default/P${Module}{moduleFolder}Specs/SRS".into(),
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
                "subterra:data-service:objects:/default/P${Module}{moduleFolder}_default/Spec"
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
