//! Custom field values: JSON whose type is only known from Polarion's field definitions at run
//! time, so it stays JSON in the DTOs and is typed here.

use std::collections::BTreeMap;

use polarion::{FieldDefinition, FieldValue, Text};
use serde_json::{Map, Value};

use crate::error::{CommandError, Result};

/// Custom field values from a JSON object, typed by the given definitions (or inferred from the
/// JSON where a key has no definition). `null` clears a field.
pub(crate) fn custom_values(
    object: &Map<String, Value>,
    definitions: &[FieldDefinition],
) -> Result<BTreeMap<String, Option<FieldValue>>> {
    object
        .iter()
        .map(|(key, value)| {
            let typed = match definitions.iter().find(|d| &d.id == key) {
                Some(def) => FieldValue::coerce(value, def)?,
                None => infer(key, value)?,
            };
            Ok((key.clone(), typed))
        })
        .collect()
}

/// Type a JSON value without a field definition. Strings stay strings; use
/// `{"enum": "id"}`, `{"enums": [..]}`, `{"date": ".."}`, `{"dateTime": ".."}`,
/// `{"time": ".."}`, `{"decimal": ".."}` or `{"type": .., "content": ..}` to be explicit.
pub fn infer(key: &str, value: &Value) -> Result<Option<FieldValue>> {
    let bad = || CommandError::invalid(format!("custom field `{key}`: cannot use {value}"));
    Ok(Some(match value {
        Value::Null => return Ok(None),
        Value::Bool(b) => FieldValue::Bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => FieldValue::Int(i),
            None => FieldValue::Float(n.as_f64().ok_or_else(bad)?),
        },
        Value::String(s) => FieldValue::String(s.clone()),
        Value::Array(items) => FieldValue::List(
            items
                .iter()
                .map(|v| infer(key, v)?.ok_or_else(bad))
                .collect::<Result<_>>()?,
        ),
        Value::Object(map) => {
            let s = |k: &str| map.get(k).and_then(Value::as_str).map(str::to_owned);
            if let Some(id) = s("enum") {
                FieldValue::Enum(id)
            } else if let Some(Value::Array(ids)) = map.get("enums") {
                FieldValue::List(
                    ids.iter()
                        .map(|v| v.as_str().map(|s| FieldValue::Enum(s.to_owned())))
                        .collect::<Option<_>>()
                        .ok_or_else(bad)?,
                )
            } else if let Some(d) = s("date") {
                FieldValue::Date(d)
            } else if let Some(d) = s("dateTime") {
                FieldValue::DateTime(d)
            } else if let Some(t) = s("time") {
                FieldValue::Time(t)
            } else if let Some(d) = s("decimal") {
                FieldValue::Decimal(d)
            } else if let Some(content) = s("content") {
                FieldValue::Text(Text {
                    content_type: s("type").unwrap_or_else(|| "text/plain".into()),
                    content,
                })
            } else {
                return Err(bad());
            }
        }
    }))
}
