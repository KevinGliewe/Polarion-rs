//! Schema → Rust type model.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde_json::Value;

use crate::docs::{doc_lines, example};
use crate::names::{pascal, resource_name, singular, snake};
use crate::spec::{Spec, envelope, ref_name};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    String,
    Bool,
    I32,
    I64,
    F64,
    Value,
    TextContent,
    Vec(Box<Ty>),
    Map(Box<Ty>),
    Named(String),
}

impl Ty {
    pub fn rust(&self) -> String {
        match self {
            Ty::String => "String".into(),
            Ty::Bool => "bool".into(),
            Ty::I32 => "i32".into(),
            Ty::I64 => "i64".into(),
            Ty::F64 => "f64".into(),
            Ty::Value => "Value".into(),
            Ty::TextContent => "TextContent".into(),
            Ty::Vec(t) => format!("Vec<{}>", t.rust()),
            Ty::Map(t) => format!("BTreeMap<String, {}>", t.rust()),
            Ty::Named(n) => n.clone(),
        }
    }
}

/// Spec locations whose `properties` are examples of free-form keys, not a
/// fixed set of fields. Keyed by `<schema>.<property path>`.
fn override_for(path: &str) -> Option<Ty> {
    match path {
        "executeJobRequestBody.params" => Some(Ty::Map(Box::new(Ty::String))),
        "fieldsMetadataActionResponseBody.data.attributes"
        | "fieldsMetadataActionResponseBody.data.relationships" => {
            Some(Ty::Map(Box::new(Ty::Value)))
        }
        _ => None,
    }
}

#[derive(Debug)]
pub struct Field {
    pub json: String,
    pub ident: String,
    pub ty: Ty,
    pub doc: Vec<String>,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub enum Kind {
    /// Attributes of a resource type.
    Attributes {
        resource_type: String,
        alias: String,
    },
    /// Any other object.
    Plain,
}

#[derive(Debug, Default)]
pub struct Rel {
    pub to_many: bool,
    pub targets: BTreeSet<String>,
}

#[derive(Debug)]
pub struct StructDef {
    pub name: String,
    pub doc: Vec<String>,
    pub fields: BTreeMap<String, Field>,
    pub kind: Kind,
    pub relationships: BTreeMap<String, Rel>,
}

pub struct Registry<'a> {
    spec: &'a Spec,
    pub structs: BTreeMap<String, StructDef>,
    pub warnings: Vec<String>,
    plain_seen: BTreeMap<String, Ty>,
}

impl<'a> Registry<'a> {
    pub fn new(spec: &'a Spec) -> Self {
        Self {
            spec,
            structs: BTreeMap::new(),
            warnings: Vec::new(),
            plain_seen: BTreeMap::new(),
        }
    }

    /// Merge one resource object schema (from any envelope variant) into the
    /// attributes struct of its resource type.
    pub fn register_resource(&mut self, resource_type: &str, item: &Value) -> Result<String> {
        let alias = resource_name(resource_type)?;
        let name = format!("{alias}Attributes");
        self.structs
            .entry(name.clone())
            .or_insert_with(|| StructDef {
                name: name.clone(),
                doc: Vec::new(),
                fields: BTreeMap::new(),
                kind: Kind::Attributes {
                    resource_type: resource_type.to_owned(),
                    alias: alias.to_owned(),
                },
                relationships: BTreeMap::new(),
            });
        if let Some(attrs) = item.pointer("/properties/attributes") {
            let attrs = self.spec.resolve(attrs);
            self.merge_object(
                &name,
                attrs,
                alias,
                &format!("{resource_type}.attributes"),
                false,
            );
        }
        if let Some(rels) = item
            .pointer("/properties/relationships/properties")
            .and_then(Value::as_object)
        {
            let mut found = Vec::new();
            for (rel_name, rel) in rels {
                let rel = self.spec.resolve(rel);
                let Some(data) = rel
                    .pointer("/properties/data")
                    .map(|d| self.spec.resolve(d))
                else {
                    continue;
                };
                let to_many = data.get("type").and_then(Value::as_str) == Some("array");
                let target = if to_many {
                    data.get("items").map(|i| self.spec.resolve(i))
                } else {
                    Some(data)
                };
                let targets: Vec<String> = target
                    .and_then(|t| t.pointer("/properties/type/enum"))
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                found.push((rel_name.clone(), to_many, targets));
            }
            let def = self.structs.get_mut(&name).expect("inserted above");
            for (rel_name, to_many, targets) in found {
                let rel = def.relationships.entry(rel_name).or_default();
                rel.to_many |= to_many;
                rel.targets.extend(targets);
            }
        }
        Ok(name)
    }

    /// Named schemas mapped through [`Registry::plain`] so far.
    pub fn plain_schemas(&self) -> impl Iterator<Item = (&String, &Ty)> {
        self.plain_seen.iter()
    }

    /// Rust type for a named, non-envelope schema.
    pub fn plain(&mut self, schema_name: &str) -> Ty {
        if let Some(ty) = self.plain_seen.get(schema_name) {
            return ty.clone();
        }
        let name = pascal(schema_name);
        // Insert first so recursive references terminate.
        self.plain_seen
            .insert(schema_name.to_owned(), Ty::Named(name.clone()));
        let schema = self
            .spec
            .schema(schema_name)
            .cloned()
            .unwrap_or(Value::Null);
        let ty = if ref_name(&schema).is_some() {
            self.map(&schema, &name, schema_name, true)
        } else if schema.get("properties").is_some() && !is_text_content(&schema) {
            self.merge_object(&name, &schema, &name, schema_name, true);
            Ty::Named(name)
        } else {
            self.map(&schema, &name, schema_name, true)
        };
        self.plain_seen.insert(schema_name.to_owned(), ty.clone());
        ty
    }

    fn map(&mut self, schema: &Value, hint: &str, path: &str, honor_required: bool) -> Ty {
        if let Some(ty) = override_for(path) {
            return ty;
        }
        if let Some(name) = ref_name(schema) {
            if envelope(self.spec, name).is_some() {
                self.warnings.push(format!(
                    "{path}: envelope reference `{name}` mapped to Value"
                ));
                return Ty::Value;
            }
            return self.plain(name);
        }
        for combinator in ["oneOf", "anyOf", "allOf"] {
            if schema.get(combinator).is_some() {
                self.warnings
                    .push(format!("{path}: `{combinator}` mapped to Value"));
                return Ty::Value;
            }
        }
        let has_props = schema.get("properties").is_some();
        match schema.get("type").and_then(Value::as_str) {
            Some("object") | None if has_props => {
                if is_text_content(schema) {
                    return Ty::TextContent;
                }
                self.merge_object(hint, schema, hint, path, honor_required);
                Ty::Named(hint.to_owned())
            }
            Some("object") => match schema.get("additionalProperties") {
                Some(addl) if addl.as_object().is_some_and(|o| !o.is_empty()) => {
                    Ty::Map(Box::new(self.map(
                        addl,
                        &format!("{hint}Value"),
                        &format!("{path}.*"),
                        honor_required,
                    )))
                }
                _ => Ty::Value,
            },
            Some("array") => {
                let items = schema.get("items").cloned().unwrap_or(Value::Null);
                Ty::Vec(Box::new(self.map(
                    &items,
                    &singular(hint),
                    &format!("{path}[]"),
                    honor_required,
                )))
            }
            Some("string") => Ty::String,
            Some("integer") => match schema.get("format").and_then(Value::as_str) {
                Some("int32") => Ty::I32,
                _ => Ty::I64,
            },
            Some("number") => Ty::F64,
            Some("boolean") => Ty::Bool,
            other => {
                self.warnings.push(format!(
                    "{path}: unsupported schema type {other:?} mapped to Value"
                ));
                Ty::Value
            }
        }
    }

    /// Add the properties of an object schema to struct `name`, creating it if needed.
    fn merge_object(
        &mut self,
        name: &str,
        schema: &Value,
        child_prefix: &str,
        path: &str,
        honor_required: bool,
    ) {
        let required: BTreeSet<&str> = if honor_required {
            schema
                .get("required")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default()
        } else {
            BTreeSet::new()
        };
        let mut fields = Vec::new();
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (prop, prop_schema) in props {
                let hint = format!("{child_prefix}{}", pascal(prop));
                let ty = self.map(
                    prop_schema,
                    &hint,
                    &format!("{path}.{prop}"),
                    honor_required,
                );
                fields.push(Field {
                    json: prop.clone(),
                    ident: snake(prop),
                    ty,
                    doc: field_doc(self.spec.resolve(prop_schema)),
                    required: required.contains(prop.as_str()),
                });
            }
        }
        let def = self
            .structs
            .entry(name.to_owned())
            .or_insert_with(|| StructDef {
                name: name.to_owned(),
                doc: Vec::new(),
                fields: BTreeMap::new(),
                kind: Kind::Plain,
                relationships: BTreeMap::new(),
            });
        if def.doc.is_empty()
            && let Some(d) = schema.get("description").and_then(Value::as_str)
        {
            def.doc = doc_lines(d);
        }
        for field in fields {
            match def.fields.get_mut(&field.json) {
                None => {
                    def.fields.insert(field.json.clone(), field);
                }
                Some(existing) => {
                    if existing.doc.is_empty() {
                        existing.doc = field.doc;
                    }
                    if existing.ty != field.ty && existing.ty != Ty::Value {
                        self.warnings.push(format!(
                            "{name}.{}: conflicting types {} / {}, using Value",
                            field.json,
                            existing.ty.rust(),
                            field.ty.rust()
                        ));
                        existing.ty = Ty::Value;
                    }
                    existing.required &= field.required;
                }
            }
        }
    }
}

/// `{"type": "text/html" | "text/plain", "value": "..."}`.
fn is_text_content(schema: &Value) -> bool {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return false;
    };
    props.len() == 2
        && props
            .get("value")
            .and_then(|v| v.get("type"))
            .and_then(Value::as_str)
            == Some("string")
        && props
            .get("type")
            .and_then(|v| v.get("type"))
            .and_then(Value::as_str)
            == Some("string")
}

fn field_doc(schema: &Value) -> Vec<String> {
    let mut doc = schema
        .get("description")
        .and_then(Value::as_str)
        .map(doc_lines)
        .unwrap_or_default();
    let mut notes = Vec::new();
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let values: Vec<String> = values
            .iter()
            .filter_map(Value::as_str)
            .map(|v| format!("`{v}`"))
            .collect();
        notes.push(format!("Allowed values: {}.", values.join(", ")));
    }
    if let Some(format) = schema.get("format").and_then(Value::as_str)
        && matches!(format, "date" | "date-time" | "time")
    {
        notes.push(format!("Format: `{format}`."));
    }
    if let Some(ex) = schema.get("example").and_then(example) {
        notes.push(format!("Example: {ex}."));
    }
    if !notes.is_empty() {
        if !doc.is_empty() {
            doc.push(String::new());
        }
        doc.push(notes.join(" "));
    }
    doc
}
