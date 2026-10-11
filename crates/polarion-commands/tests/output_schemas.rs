//! Output schemas describe what results really serialize to: real values validate against the
//! schema of their type (strict MCP clients check `structuredContent` against `outputSchema`).

use std::collections::BTreeMap;

use polarion::soap::types as soap;
use polarion::{
    Comment, DiagramAttachment, DocumentRef, FieldValue, ItemRef, Link, MarkdownUpdate, TestRun,
    Text, WorkItem,
};
use polarion_commands::documents::CreatedDocument;
use polarion_commands::items::{Created, ItemLinks};
use polarion_commands::{Changed, Done, Download, catalog, output_schema_of};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

#[track_caller]
fn valid<T: JsonSchema + Serialize>(value: &T) {
    let schema = output_schema_of::<T>();
    let instance = serde_json::to_value(value).unwrap();
    let validator = jsonschema::validator_for(&schema)
        .unwrap_or_else(|e| panic!("invalid schema ({e}): {schema:#}"));
    let errors: Vec<String> = validator
        .iter_errors(&instance)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{errors:#?}\ninstance: {instance:#}\nschema: {schema:#}"
    );
}

fn work_item() -> WorkItem {
    let mut custom = BTreeMap::new();
    custom.insert("text".into(), FieldValue::String("a".into()));
    custom.insert("flag".into(), FieldValue::Bool(true));
    custom.insert("count".into(), FieldValue::Int(3));
    custom.insert("ratio".into(), FieldValue::Float(1.5));
    custom.insert("money".into(), FieldValue::Decimal("1.10".into()));
    custom.insert("day".into(), FieldValue::Date("2024-01-31".into()));
    custom.insert("asil".into(), FieldValue::Enum("D".into()));
    custom.insert(
        "html".into(),
        FieldValue::Text(Text {
            content_type: "text/html".into(),
            content: "<p>x</p>".into(),
        }),
    );
    custom.insert(
        "platforms".into(),
        FieldValue::List(vec![FieldValue::Enum("a".into())]),
    );
    custom.insert(
        "odd".into(),
        FieldValue::Other {
            xsi_type: Some("x:Mystery".into()),
            xml: None,
        },
    );
    WorkItem {
        id: Some("P-1".into()),
        project: Some("P".into()),
        title: Some("Brakes".into()),
        description: Some(Text {
            content_type: "text/plain".into(),
            content: "d".into(),
        }),
        assignees: vec!["amy".into()],
        document: Some(DocumentRef::new("P", "Specs", "SRS")),
        links: vec![Link {
            role: Some("parent".into()),
            item: ItemRef::new("P", "P-0"),
            suspect: true,
            revision: None,
        }],
        comments: vec![Comment {
            id: Some("c1".into()),
            text: Some(Text::default()),
            resolved: true,
            ..Comment::default()
        }],
        custom_fields: custom,
        uri: Some("subterra:data-service:objects:/default/P${WorkItem}P-1".into()),
        ..WorkItem::default()
    }
}

#[test]
fn library_results_match_their_schemas() {
    valid(&work_item());
    valid(&WorkItem::default());
    valid(&vec![work_item(), WorkItem::default()]);
    valid(&ItemRef::new("P", "P-1"));
    valid(&ItemRef::from_uri("subterra:x".to_owned()));
    valid(&DocumentRef::new("P", "Specs", "SRS"));
    valid(&DocumentRef::from_uri("subterra:y".to_owned()));
    valid(&Some(TestRun::default()));
    valid(&Option::<FieldValue>::None);
}

#[test]
fn soap_results_match_their_schemas() {
    let custom = soap::Custom {
        key: "asil".into(),
        value: Some(soap::AnyValue::EnumOptionId(Box::new(soap::EnumOptionId {
            id: Some("D".into()),
        }))),
    };
    let build = soap::Build {
        id: Some("b1".into()),
        build_status: Some("OK".into()),
        log_files: Some(vec!["a.log".into()]),
        linked_work_items: Some(vec![soap::BuildLinkedWorkItem {
            revision: Some("12".into()),
            work_item: Some(soap::WorkItem {
                id: Some("P-1".into()),
                custom_fields: Some(vec![custom]),
                ..soap::WorkItem::default()
            }),
            ..soap::BuildLinkedWorkItem::default()
        }]),
        unresolvable: Some(false),
        ..soap::Build::default()
    };
    valid(&vec![build, soap::Build::default()]);
    valid(&soap::Activity {
        global_id: Some("g".into()),
        resource_locations: Some(vec!["x".into()]),
        ..soap::Activity::default()
    });
    valid(&soap::ProductLicense::default());
    valid(&vec![soap::Folder::default()]);
}

#[test]
fn command_results_match_their_schemas() {
    valid(&Changed::from(true));
    valid(&Done);
    valid(&Download(b"hello".to_vec()));
    valid(&ItemLinks {
        outgoing: vec![],
        incoming: work_item().links,
    });
    valid(&Created {
        item: ItemRef::new("P", "P-2"),
        uri: "u".into(),
    });
    valid(&CreatedDocument::from(DocumentRef::new("P", "S", "D")));
    assert_eq!(output_schema_of::<u32>()["type"], json!("integer"));

    // The Markdown writes: nothing to report, and everything.
    valid(&MarkdownUpdate::default());
    let diagram = |name: &str, attachment: Option<&str>, uploaded| DiagramAttachment {
        file_name: name.into(),
        attachment: attachment.map(Into::into),
        uploaded,
    };
    valid(&MarkdownUpdate {
        updated: true,
        diagrams: vec![
            diagram(
                "mermaid-0123456789abcdef.svg",
                Some("1-mermaid-0123456789abcdef.svg"),
                true,
            ),
            diagram(
                "mermaid-fedcba9876543210.svg",
                Some("2-mermaid-fedcba9876543210.svg"),
                false,
            ),
            // Written as a code block.
            diagram("mermaid-00000000ffffffff.svg", None, false),
        ],
        deleted: vec!["3-mermaid-1111111111111111.svg".into()],
        warnings: vec!["mermaid diagram (graph) is written as a code block: ...".into()],
    });
    let validator = jsonschema::validator_for(&output_schema_of::<MarkdownUpdate>()).unwrap();
    assert!(!validator.is_valid(&json!({})), "`updated` is always there");
    assert!(!validator.is_valid(&json!({"updated": true, "diagrams": [{"attachment": "a"}]})));
    // Agents read the description: no rustdoc links.
    let schema = output_schema_of::<MarkdownUpdate>();
    let description = schema["description"].as_str().unwrap();
    assert!(
        !description.contains("[`") && !description.contains('\n'),
        "{description}"
    );

    // The check has teeth: a wrong shape is refused.
    let validator = jsonschema::validator_for(&output_schema_of::<WorkItem>()).unwrap();
    assert!(!validator.is_valid(&json!({"links": "not a list"})));
    assert!(!validator.is_valid(&json!({"customFields": [1]})));
}

#[test]
fn every_command_has_an_output_schema() {
    for entry in catalog() {
        let schema = entry.output_schema();
        assert!(schema.is_object(), "{}: {schema}", entry.full_name());
        jsonschema::validator_for(&schema)
            .unwrap_or_else(|e| panic!("{}: invalid schema: {e}", entry.full_name()));
        let _: Option<&Value> = schema.get("type");
    }
}
