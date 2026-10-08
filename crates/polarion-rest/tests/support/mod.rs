//! Spec-driven round-trip checks: build an example JSON document from a schema
//! in the OpenAPI spec, decode it into the generated Rust type, encode it again
//! and compare. Catches wrong field names, wrong scalar types and envelope
//! mismatches across every schema without hand-written fixtures.

use std::sync::OnceLock;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

const SPEC: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../spec/polarion-openapi.json"
));

fn spec() -> &'static Value {
    static SPEC_VALUE: OnceLock<Value> = OnceLock::new();
    SPEC_VALUE.get_or_init(|| serde_json::from_str(SPEC).expect("spec is valid JSON"))
}

fn resolve(mut schema: &Value) -> &Value {
    while let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        schema = spec()
            .pointer(r.trim_start_matches('#'))
            .unwrap_or_else(|| panic!("dangling $ref {r}"));
    }
    schema
}

/// An example instance of `schema`, with every property populated.
pub fn example(schema: &Value, depth: usize) -> Value {
    let schema = resolve(schema);
    if depth > 16 {
        return Value::Null;
    }
    for combinator in ["oneOf", "anyOf"] {
        if let Some(first) = schema.get(combinator).and_then(|v| v.get(0)) {
            return example(first, depth + 1);
        }
    }
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        let obj: Map<String, Value> = props
            .iter()
            .map(|(k, v)| (k.clone(), example(v, depth + 1)))
            .collect();
        return Value::Object(obj);
    }
    if let Some(ex) = schema.get("example") {
        return ex.clone();
    }
    if let Some(first) = schema.get("enum").and_then(|e| e.get(0)) {
        return first.clone();
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("array") => Value::Array(vec![example(
            schema.get("items").unwrap_or(&Value::Null),
            depth + 1,
        )]),
        Some("object") => match schema.get("additionalProperties") {
            Some(addl) if addl.is_object() => Value::Object(Map::from_iter([(
                "key".to_owned(),
                example(addl, depth + 1),
            )])),
            _ => Value::Object(Map::new()),
        },
        Some("string") => Value::String("text".into()),
        Some("integer") => Value::from(7),
        Some("number") => Value::from(1.5),
        Some("boolean") => Value::Bool(true),
        _ => Value::Null,
    }
}

/// Drop nulls and normalize numbers so semantically equal documents compare equal.
fn normalize(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, normalize(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize).collect()),
        Value::Number(n) => n.as_f64().map(Value::from).unwrap_or(Value::Number(n)),
        other => other,
    }
}

/// Decode the example for `schema_name` as `T`, re-encode and compare.
pub fn roundtrip<T: DeserializeOwned + Serialize>(schema_name: &str) {
    let schema = spec()
        .pointer(&format!("/components/schemas/{schema_name}"))
        .unwrap_or_else(|| panic!("schema {schema_name} not in spec"));
    let input = example(schema, 0);
    let decoded: T = serde_json::from_value(input.clone())
        .unwrap_or_else(|e| panic!("{schema_name}: example does not decode: {e}\n{input:#}"));
    let output = serde_json::to_value(&decoded).expect("encodes");
    assert_eq!(
        normalize(output),
        normalize(input),
        "{schema_name}: round-trip changed the document"
    );
}
