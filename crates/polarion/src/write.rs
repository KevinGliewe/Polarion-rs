//! Inputs for writes: new work items, updates, and typing JSON values for custom fields.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::model::{FieldDefinition, FieldValue, Text};

/// A work item to create with [`crate::WorkItems::create`].
///
/// Assignees, categories, links, hyperlinks, comments and attachments are not
/// accepted by `createWorkItem`; add them afterwards with the dedicated methods.
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
