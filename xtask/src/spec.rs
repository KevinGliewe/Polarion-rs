//! Navigation helpers over the raw OpenAPI document.

use anyhow::{Context, Result};
use serde_json::Value;

pub struct Spec {
    pub root: Value,
}

impl Spec {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let root =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self { root })
    }

    pub fn schema(&self, name: &str) -> Option<&Value> {
        self.root
            .pointer(&format!("/components/schemas/{}", escape_pointer(name)))
    }

    /// Follow `$ref`s (schemas and parameters) until reaching a concrete object.
    pub fn resolve<'a>(&'a self, mut value: &'a Value) -> &'a Value {
        for _ in 0..32 {
            match value.get("$ref").and_then(Value::as_str) {
                Some(r) => match r.strip_prefix('#').and_then(|p| self.root.pointer(p)) {
                    Some(target) => value = target,
                    None => return value,
                },
                None => return value,
            }
        }
        value
    }
}

/// Name of the schema a `$ref` points to (`#/components/schemas/Foo` → `Foo`).
pub fn ref_name(schema: &Value) -> Option<&str> {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|r| r.strip_prefix("#/components/schemas/"))
}

fn escape_pointer(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Get,
    Post,
    Patch,
    Delete,
}

/// A JSON:API envelope schema, named `{resource}{Single|List}{Verb}{Request|Response}`.
#[derive(Debug, Clone)]
pub struct EnvelopeSchema {
    pub list: bool,
    pub verb: Verb,
    pub request: bool,
    /// The single resource type of `data`, if the schema pins one.
    pub resource_type: Option<String>,
}

impl EnvelopeSchema {
    /// The resource object schema (`data` or `data.items`).
    pub fn data_item<'a>(&self, spec: &'a Spec, schema: &'a Value) -> Option<&'a Value> {
        let data = spec.resolve(schema.pointer("/properties/data")?);
        Some(if self.list {
            spec.resolve(data.get("items")?)
        } else {
            data
        })
    }
}

pub fn envelope(spec: &Spec, name: &str) -> Option<EnvelopeSchema> {
    let (rest, request) = if let Some(r) = name.strip_suffix("Request") {
        (r, true)
    } else {
        (name.strip_suffix("Response")?, false)
    };
    let (rest, verb) = [
        ("Get", Verb::Get),
        ("Post", Verb::Post),
        ("Patch", Verb::Patch),
        ("Delete", Verb::Delete),
    ]
    .into_iter()
    .find_map(|(s, v)| rest.strip_suffix(s).map(|r| (r, v)))?;
    let (prefix, list) = if let Some(p) = rest.strip_suffix("Single") {
        (p, false)
    } else {
        (rest.strip_suffix("List")?, true)
    };
    if prefix.is_empty() || !prefix.chars().all(|c| c.is_ascii_alphabetic() || c == '_') {
        return None;
    }
    let mut env = EnvelopeSchema {
        list,
        verb,
        request,
        resource_type: None,
    };
    let schema = spec.schema(name)?;
    let types = env
        .data_item(spec, schema)
        .and_then(|item| item.pointer("/properties/type/enum"))
        .and_then(Value::as_array);
    if let Some([Value::String(t)]) = types.map(Vec::as_slice) {
        env.resource_type = Some(t.clone());
    }
    Some(env)
}
