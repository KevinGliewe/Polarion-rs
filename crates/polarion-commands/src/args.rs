//! Command arguments: a JSON object with typed accessors.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use polarion::{DocumentRef, FieldDefinition, FieldValue, Fields, ItemRef, Query, Text};
use serde_json::{Map, Value};

use crate::error::{CommandError, Result};

/// The arguments of one command call, plus the default project for short ids.
#[derive(Debug, Clone, Default)]
pub struct Args {
    values: Map<String, Value>,
    default_project: Option<String>,
}

fn invalid(name: &str, expected: &str, got: &Value) -> CommandError {
    CommandError::invalid(format!("`{name}`: expected {expected}, got {got}"))
}

impl Args {
    pub fn new(values: Map<String, Value>, default_project: Option<String>) -> Self {
        Self {
            values,
            default_project,
        }
    }

    /// From a JSON value, which must be an object (or null for no arguments).
    pub fn from_value(value: Value, default_project: Option<String>) -> Result<Self> {
        match value {
            Value::Object(values) => Ok(Self::new(values, default_project)),
            Value::Null => Ok(Self::new(Map::new(), default_project)),
            other => Err(CommandError::invalid(format!(
                "arguments must be an object, got {other}"
            ))),
        }
    }

    pub fn values(&self) -> &Map<String, Value> {
        &self.values
    }

    /// The raw value; `null` counts as absent.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.get(name).filter(|v| !v.is_null())
    }

    fn required(&self, name: &str) -> Result<&Value> {
        self.get(name)
            .ok_or_else(|| CommandError::invalid(format!("missing argument `{name}`")))
    }

    pub fn opt_str(&self, name: &str) -> Result<Option<String>> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(v @ (Value::Number(_) | Value::Bool(_))) => Ok(Some(v.to_string())),
            Some(other) => Err(invalid(name, "a string", other)),
        }
    }

    pub fn str(&self, name: &str) -> Result<String> {
        self.required(name)?;
        Ok(self.opt_str(name)?.unwrap_or_default())
    }

    pub fn opt_bool(&self, name: &str) -> Result<Option<bool>> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(Value::String(s)) => match s.trim() {
                "true" | "1" | "yes" => Ok(Some(true)),
                "false" | "0" | "no" => Ok(Some(false)),
                _ => Err(invalid(name, "true or false", &Value::from(s.as_str()))),
            },
            Some(other) => Err(invalid(name, "true or false", other)),
        }
    }

    /// A flag; absent means `false`.
    pub fn bool(&self, name: &str) -> Result<bool> {
        Ok(self.opt_bool(name)?.unwrap_or(false))
    }

    pub fn opt_i64(&self, name: &str) -> Result<Option<i64>> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Number(n)) => n
                .as_i64()
                .map(Some)
                .ok_or_else(|| invalid(name, "an integer", &Value::Number(n.clone()))),
            Some(Value::String(s)) => s
                .trim()
                .parse()
                .map(Some)
                .map_err(|_| invalid(name, "an integer", &Value::from(s.as_str()))),
            Some(other) => Err(invalid(name, "an integer", other)),
        }
    }

    pub fn opt_u32(&self, name: &str) -> Result<Option<u32>> {
        match self.opt_i64(name)? {
            None => Ok(None),
            Some(n) => u32::try_from(n)
                .map(Some)
                .map_err(|_| invalid(name, "a non-negative integer", &Value::from(n))),
        }
    }

    pub fn u32(&self, name: &str) -> Result<u32> {
        self.required(name)?;
        Ok(self.opt_u32(name)?.unwrap_or_default())
    }

    pub fn i32(&self, name: &str) -> Result<i32> {
        let n = self.required(name).and_then(|_| self.opt_i64(name))?;
        let n = n.unwrap_or_default();
        i32::try_from(n).map_err(|_| invalid(name, "a 32-bit integer", &Value::from(n)))
    }

    pub fn opt_f32(&self, name: &str) -> Result<Option<f32>> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::Number(n)) => Ok(n.as_f64().map(|f| f as f32)),
            Some(Value::String(s)) => s
                .trim()
                .parse()
                .map(Some)
                .map_err(|_| invalid(name, "a number", &Value::from(s.as_str()))),
            Some(other) => Err(invalid(name, "a number", other)),
        }
    }

    /// A list of strings: a JSON array, or a comma-separated string. Absent: empty.
    pub fn list(&self, name: &str) -> Result<Vec<String>> {
        match self.get(name) {
            None => Ok(Vec::new()),
            Some(Value::Array(items)) => items
                .iter()
                .map(|v| match v {
                    Value::String(s) => Ok(s.clone()),
                    other => Err(invalid(name, "a list of strings", other)),
                })
                .collect(),
            Some(Value::String(s)) => Ok(crate::config::split_list(s)),
            Some(other) => Err(invalid(name, "a list of strings", other)),
        }
    }

    /// The `project` argument, or the default project.
    pub fn project(&self) -> Result<String> {
        match self.opt_str("project")? {
            Some(p) => Ok(p),
            None => self.default_project.clone().ok_or_else(|| {
                CommandError::invalid(
                    "no project: pass `project`, set POLARION_PROJECT (or -p), or use PROJECT/ID",
                )
            }),
        }
    }

    pub fn opt_project(&self) -> Result<Option<String>> {
        Ok(self
            .opt_str("project")?
            .or_else(|| self.default_project.clone()))
    }

    fn default_project_for(&self, what: &str) -> Result<String> {
        self.default_project.clone().ok_or_else(|| {
            CommandError::invalid(format!(
                "no project for `{what}`: give the project in it, or set POLARION_PROJECT (or -p)"
            ))
        })
    }

    fn parse_item(&self, name: &str, value: &Value) -> Result<ItemRef> {
        match value {
            Value::String(s) if s.starts_with("subterra:") => Ok(ItemRef::from_uri(s.clone())),
            Value::String(s) => match s.split('/').collect::<Vec<_>>()[..] {
                [project, id] if !project.is_empty() && !id.is_empty() => {
                    Ok(ItemRef::new(project, id))
                }
                [id] if !id.is_empty() => Ok(ItemRef::new(self.default_project_for(s)?, id)),
                _ => Err(invalid(name, "ID, PROJECT/ID or a URI", value)),
            },
            Value::Object(map) => match (map.get("project"), map.get("id"), map.get("uri")) {
                (Some(Value::String(p)), Some(Value::String(id)), _) => Ok(ItemRef::new(p, id)),
                (_, _, Some(Value::String(uri))) => Ok(ItemRef::from_uri(uri.clone())),
                _ => Err(invalid(name, "{project, id} or {uri}", value)),
            },
            other => Err(invalid(name, "a work item reference", other)),
        }
    }

    pub fn item(&self, name: &str) -> Result<ItemRef> {
        let value = self.required(name)?.clone();
        self.parse_item(name, &value)
    }

    pub fn opt_item(&self, name: &str) -> Result<Option<ItemRef>> {
        self.get(name)
            .cloned()
            .map(|v| self.parse_item(name, &v))
            .transpose()
    }

    /// A list of work items (JSON array, or comma-separated string).
    pub fn items(&self, name: &str) -> Result<Vec<ItemRef>> {
        match self.get(name) {
            None => Ok(Vec::new()),
            Some(Value::Array(values)) => values.iter().map(|v| self.parse_item(name, v)).collect(),
            Some(Value::String(s)) => crate::config::split_list(s)
                .into_iter()
                .map(|s| self.parse_item(name, &Value::String(s)))
                .collect(),
            Some(other) => Err(invalid(name, "a list of work items", other)),
        }
    }

    pub fn document(&self, name: &str) -> Result<DocumentRef> {
        let value = self.required(name)?;
        match value {
            Value::String(s) if s.starts_with("subterra:") => Ok(DocumentRef::from_uri(s.clone())),
            Value::String(s) => match s.split('/').collect::<Vec<_>>()[..] {
                [project, space, doc] if ![project, space, doc].contains(&"") => {
                    Ok(DocumentRef::new(project, space, doc))
                }
                [space, doc] if !space.is_empty() && !doc.is_empty() => {
                    Ok(DocumentRef::new(self.default_project_for(s)?, space, doc))
                }
                _ => Err(invalid(
                    name,
                    "SPACE/NAME, PROJECT/SPACE/NAME or a URI",
                    value,
                )),
            },
            Value::Object(map) => match (
                map.get("project"),
                map.get("space"),
                map.get("name"),
                map.get("uri"),
            ) {
                (Some(Value::String(p)), Some(Value::String(sp)), Some(Value::String(n)), _) => {
                    Ok(DocumentRef::new(p, sp, n))
                }
                (_, _, _, Some(Value::String(uri))) => Ok(DocumentRef::from_uri(uri.clone())),
                _ => Err(invalid(name, "{project, space, name} or {uri}", value)),
            },
            other => Err(invalid(name, "a document reference", other)),
        }
    }

    pub fn opt_text(&self, name: &str) -> Result<Option<Text>> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(Text {
                content_type: "text/plain".into(),
                content: s.clone(),
            })),
            Some(Value::Object(map)) => {
                let content = map
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid(name, "{type, content}", &Value::Object(map.clone())))?;
                Ok(Some(Text {
                    content_type: map
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("text/plain")
                        .to_owned(),
                    content: content.to_owned(),
                }))
            }
            Some(other) => Err(invalid(name, "text or {type, content}", other)),
        }
    }

    pub fn text(&self, name: &str) -> Result<Text> {
        self.required(name)?;
        Ok(self.opt_text(name)?.unwrap_or_default())
    }

    /// `fields` (`standard`, `all`, or a list) plus `custom` (custom field keys). `None` when
    /// neither is given.
    pub fn opt_fields(&self) -> Result<Option<Fields>> {
        let base = match self.get("fields") {
            None => None,
            Some(Value::String(s)) if s.trim() == "standard" => Some(Fields::standard()),
            Some(Value::String(s)) if s.trim() == "all" => Some(Fields::all()),
            Some(_) => Some(Fields::new(self.list("fields")?)),
        };
        let custom = self.list("custom")?;
        if custom.is_empty() {
            return Ok(base);
        }
        Ok(Some(
            custom
                .iter()
                .fold(base.unwrap_or_default(), |f, key| f.with_custom(key)),
        ))
    }

    /// Like [`Args::opt_fields`], defaulting to [`Fields::standard`].
    pub fn fields(&self) -> Result<Fields> {
        Ok(self.opt_fields()?.unwrap_or_default())
    }

    /// A Lucene query from `query`, `project`, `sort`, `limit`, `all`, `fields`, `custom`,
    /// `baseline` and `revision`.
    pub fn query(&self) -> Result<Query> {
        let mut q = Query::new(self.opt_str("query")?.unwrap_or_default());
        if let Some(p) = self.opt_project()? {
            q = q.project(p);
        }
        if let Some(sort) = self.opt_str("sort")? {
            q = q.sort(sort);
        }
        if self.bool("all")? {
            q = q.unlimited();
        } else if let Some(limit) = self.opt_u32("limit")? {
            q = q.limit(limit);
        }
        if let Some(fields) = self.opt_fields()? {
            q = q.fields(fields);
        }
        if let Some(b) = self.opt_str("baseline")? {
            q = q.baseline(b);
        }
        if let Some(r) = self.opt_str("revision")? {
            q = q.revision(r);
        }
        Ok(q)
    }

    /// Binary content given as base64.
    pub fn opt_bytes(&self, name: &str) -> Result<Option<Vec<u8>>> {
        match self.get(name) {
            None => Ok(None),
            Some(Value::String(s)) => {
                let compact: String = s.split_whitespace().collect();
                BASE64
                    .decode(compact)
                    .map(Some)
                    .map_err(|e| CommandError::invalid(format!("`{name}`: invalid base64: {e}")))
            }
            Some(other) => Err(invalid(name, "base64 content", other)),
        }
    }

    pub fn bytes(&self, name: &str) -> Result<Vec<u8>> {
        self.required(name)?;
        Ok(self.opt_bytes(name)?.unwrap_or_default())
    }

    /// A JSON object argument (absent: empty).
    pub fn object(&self, name: &str) -> Result<Map<String, Value>> {
        match self.get(name) {
            None => Ok(Map::new()),
            Some(Value::Object(map)) => Ok(map.clone()),
            Some(Value::String(s)) => match serde_json::from_str(s) {
                Ok(Value::Object(map)) => Ok(map),
                _ => Err(invalid(name, "a JSON object", &Value::from(s.as_str()))),
            },
            Some(other) => Err(invalid(name, "a JSON object", other)),
        }
    }

    /// Custom field values from a JSON object, typed by the given definitions
    /// (or inferred from the JSON where a key has no definition). `null` clears a field.
    pub fn custom_values(
        &self,
        name: &str,
        definitions: &[FieldDefinition],
    ) -> Result<BTreeMap<String, Option<FieldValue>>> {
        self.object(name)?
            .into_iter()
            .map(|(key, value)| {
                let typed = match definitions.iter().find(|d| d.id == key) {
                    Some(def) => FieldValue::coerce(&value, def)?,
                    None => infer(&key, &value)?,
                };
                Ok((key, typed))
            })
            .collect()
    }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn args(v: Value) -> Args {
        Args::from_value(v, Some("P".into())).unwrap()
    }

    #[test]
    fn references() {
        let a = args(json!({
            "short": "P-1", "full": "Q/Q-2", "uri": "subterra:data-service:objects:/default/R${WorkItem}R-3",
            "obj": {"project": "S", "id": "S-4"}, "doc": "Specs/SRS", "doc3": "Q/Specs/SRS",
            "bad": "a/b/c", "list": "P-1, Q/Q-2"
        }));
        assert_eq!(a.item("short").unwrap(), ItemRef::new("P", "P-1"));
        assert_eq!(a.item("full").unwrap(), ItemRef::new("Q", "Q-2"));
        assert_eq!(a.item("uri").unwrap().id.as_deref(), Some("R-3"));
        assert_eq!(a.item("obj").unwrap(), ItemRef::new("S", "S-4"));
        assert_eq!(
            a.document("doc").unwrap(),
            DocumentRef::new("P", "Specs", "SRS")
        );
        assert_eq!(
            a.document("doc3").unwrap(),
            DocumentRef::new("Q", "Specs", "SRS")
        );
        assert!(a.item("bad").is_err());
        assert!(a.item("missing").is_err());
        assert_eq!(a.items("list").unwrap().len(), 2);

        let no_default = Args::from_value(json!({"i": "P-1"}), None).unwrap();
        assert!(
            no_default
                .item("i")
                .unwrap_err()
                .message
                .contains("no project")
        );
    }

    #[test]
    fn queries_and_fields() {
        let q = args(json!({"query": "type:task", "limit": "5", "custom": "asil"}))
            .query()
            .unwrap();
        assert_eq!(q.lucene(), "project.id:P AND (type:task)");
        let f = args(json!({"fields": "id,title", "custom": ["asil"]}))
            .fields()
            .unwrap();
        assert_eq!(f.keys(), ["id", "title", "customFields.asil"]);
        assert_eq!(args(json!({})).fields().unwrap(), Fields::standard());
    }

    #[test]
    fn values() {
        let a = args(
            json!({"n": "7", "b": "true", "t": "x", "rich": {"type": "text/html", "content": "<b>x</b>"}, "data": "aGk="}),
        );
        assert_eq!(a.u32("n").unwrap(), 7);
        assert!(a.bool("b").unwrap());
        assert_eq!(a.text("t").unwrap().content_type, "text/plain");
        assert_eq!(a.text("rich").unwrap().content_type, "text/html");
        assert_eq!(a.bytes("data").unwrap(), b"hi");
        assert!(a.u32("t").is_err());
    }

    #[test]
    fn custom_values_use_definitions_or_inference() {
        let defs = [FieldDefinition {
            id: "asil".into(),
            kind: Some("EnumOptionId".into()),
            ..FieldDefinition::default()
        }];
        let a = args(
            json!({"cf": {"asil": "D", "count": 3, "due": {"date": "2024-01-31"}, "gone": null}}),
        );
        let values = a.custom_values("cf", &defs).unwrap();
        assert_eq!(values["asil"], Some(FieldValue::Enum("D".into())));
        assert_eq!(values["count"], Some(FieldValue::Int(3)));
        assert_eq!(values["due"], Some(FieldValue::Date("2024-01-31".into())));
        assert_eq!(values["gone"], None);
    }
}
