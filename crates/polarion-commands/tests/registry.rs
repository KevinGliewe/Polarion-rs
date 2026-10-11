//! The generated registry: names, schemas, and the JSON boundary of every command.

use std::collections::BTreeSet;

use polarion_commands::{AREAS, AnyCommand, View, catalog, find};
use serde_json::{Map, Value, json};

/// A value of the schema's type (enough to satisfy it).
fn example(schema: &Value) -> Value {
    if schema.get("contentEncoding").is_some() {
        return json!("aGVsbG8=");
    }
    match schema.get("format").and_then(Value::as_str) {
        Some("work-item") => return json!("P/P-1"),
        Some("document") => return json!("P/Space/Doc"),
        _ => {}
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values[0].clone();
    }
    let ty = match schema.get("type") {
        Some(Value::String(t)) => t.as_str(),
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .unwrap_or("null"),
        _ => "string",
    };
    match ty {
        "integer" => json!(1),
        "number" => json!(1.5),
        "boolean" => json!(true),
        "array" => json!([]),
        "object" => json!({}),
        _ => json!("x"),
    }
}

#[test]
fn every_command_parses_and_round_trips() {
    let mut names = BTreeSet::new();
    for entry in catalog() {
        let name = entry.full_name();
        assert!(names.insert(name.clone()), "duplicate command {name}");
        assert!(
            AREAS.iter().any(|(a, _)| *a == entry.area),
            "{name}: unknown area"
        );
        assert!(!entry.summary.is_empty(), "{name}: no summary");

        let schema = entry.input_schema();
        assert_eq!(schema["type"], "object", "{name}: {schema}");
        assert_eq!(schema["additionalProperties"], false, "{name}");
        let properties = schema["properties"].as_object().unwrap();
        let mut args = Map::new();
        for required in schema["required"].as_array().into_iter().flatten() {
            let key = required.as_str().unwrap();
            args.insert(key.to_owned(), example(&properties[key]));
        }

        let command = entry
            .parse(Value::Object(args.clone()))
            .unwrap_or_else(|e| panic!("{name} with {args:?}: {}", e.message));
        assert_eq!(command.name(), name);

        // The whole command serializes as {"command", "args"} and comes back as the same command;
        // the serialized arguments use the schema's names.
        let serialized = serde_json::to_value(&command).unwrap();
        assert_eq!(serialized["command"], name.as_str());
        let back: AnyCommand = serde_json::from_value(serialized.clone()).unwrap();
        assert_eq!(back.name(), name);
        let again = entry.parse(serialized["args"].clone()).unwrap_or_else(|e| {
            panic!(
                "{name} from its own serialization {serialized}: {}",
                e.message
            )
        });
        assert_eq!(again.name(), name);
    }
    assert_eq!(names.len(), 253);
}

#[test]
fn markdown_is_text_in_json() {
    let read = find("items.description-markdown").unwrap();
    assert!(!read.write && matches!(read.view, View::Text));
    assert_eq!(read.output_schema()["type"], "string");

    let set = find("items.set-description-markdown").unwrap();
    assert!(set.write && matches!(set.view, View::Object));
    let schema = set.input_schema();
    assert_eq!(schema["required"], json!(["item", "markdown"]));
    assert_eq!(schema["properties"]["markdown"]["type"], "string");
    // `@FILE` is for the command line: in JSON it is the text itself.
    let command = set
        .parse(json!({"item": "P-1", "markdown": "@desc.md", "id-prefixes": "MP, *"}))
        .unwrap();
    assert_eq!(
        serde_json::to_value(&command).unwrap()["args"],
        json!({"item": "P-1", "markdown": "@desc.md", "id-prefixes": ["MP", "*"]})
    );
    let append = find("items.append-description-markdown").unwrap();
    assert!(append.write);
    assert!(
        append.input_schema()["properties"]
            .get("keep-orphans")
            .is_none(),
        "appending deletes nothing"
    );
}

#[test]
fn unknown_arguments_are_refused() {
    let e = find("items.get")
        .unwrap()
        .parse(json!({"item": "P-1", "itme": "P-2"}))
        .unwrap_err();
    assert_eq!(e.code, "invalid");
    assert!(
        e.message.contains("`itme`") && e.message.contains("item"),
        "{}",
        e.message
    );
    let e = find("builds.all")
        .unwrap()
        .parse(json!({"x": 1}))
        .unwrap_err();
    assert!(e.message.contains("takes none"), "{}", e.message);
}

#[test]
fn scalars_are_accepted_as_people_write_them() {
    let command = find("items.query")
        .unwrap()
        .parse(json!({"query": "type:task", "limit": "5", "all": "false", "custom": "asil, severity", "sort": null}))
        .unwrap();
    let args = &serde_json::to_value(&command).unwrap()["args"];
    assert_eq!(
        args,
        &json!({"query": "type:task", "limit": 5, "custom": ["asil", "severity"]})
    );

    let e = find("items.query")
        .unwrap()
        .parse(json!({"limit": "many"}))
        .unwrap_err();
    assert!(
        e.message.contains("limit") || e.message.contains("integer"),
        "{}",
        e.message
    );
}

#[test]
fn enums_are_typed() {
    let schema = find("documents.export-pdf").unwrap().input_schema();
    // `null` too: the option is optional.
    assert_eq!(
        schema["properties"]["orientation"]["enum"],
        json!(["portrait", "landscape", null])
    );
    let e = find("documents.export-pdf")
        .unwrap()
        .parse(json!({"document": "Specs/SRS", "orientation": "diagonal"}))
        .unwrap_err();
    assert!(e.message.contains("diagonal"), "{}", e.message);
}
