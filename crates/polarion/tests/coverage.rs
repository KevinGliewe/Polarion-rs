//! The library exposes the whole SOAP API: every generated operation is called somewhere
//! in `src/`, or listed here with the reason it is not.

/// Operations deliberately not called by the library, and why.
const EXCLUDED: &[(&str, &str)] = &[
    (
        "log_in",
        "used by polarion_soap::Client::login (Polarion::login, and implicitly on the first call)",
    ),
    ("log_in_with_token", "used by polarion_soap::Client::login"),
    (
        "end_session",
        "used by polarion_soap::Client::logout (Polarion::logout)",
    ),
    (
        "add_test_record",
        "same data as addTestRecordToTestRun, which TestRuns::add_record uses",
    ),
    (
        "update_test_record",
        "same data as updateTestRecordAtIndex, which TestRuns::update_record uses",
    ),
    (
        "create_comment",
        "deprecated; addComment (WorkItems::add_comment) does the same with a title",
    ),
    (
        "create_document_comment",
        "deprecated; addComment on the document (Documents::add_comment) does the same",
    ),
    (
        "create_document_comment_reply",
        "deprecated; addComment on the comment (Documents::reply_to_comment) does the same",
    ),
    (
        "move_work_item_to_module",
        "deprecated; moveWorkItemToDocument (WorkItems::move_to_document) does the same and more",
    ),
];

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn every_soap_operation_is_exposed() {
    let services = read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../polarion-soap/src/generated/services.rs"
    ));
    let operations: Vec<&str> = services
        .lines()
        .filter_map(|l| l.strip_prefix("    pub async fn "))
        .filter_map(|l| l.split('(').next())
        .collect();
    assert!(
        operations.len() > 300,
        "parsed {} operations",
        operations.len()
    );

    let src_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let library: String = std::fs::read_dir(src_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .map(|p| read(p.to_str().unwrap()))
        .collect();
    let called = |op: &str| library.contains(&format!(".{op}("));
    let excluded = |op: &str| EXCLUDED.iter().any(|(e, _)| *e == op);

    let missing: Vec<&str> = operations
        .iter()
        .copied()
        .filter(|op| !called(op) && !excluded(op))
        .collect();
    assert!(
        missing.is_empty(),
        "SOAP operations neither exposed by the library nor excluded in tests/coverage.rs: {missing:?}"
    );

    for (op, _) in EXCLUDED {
        assert!(
            operations.contains(op),
            "excluded `{op}` is not an operation"
        );
        assert!(
            !called(op),
            "excluded `{op}` is used; remove it from EXCLUDED"
        );
    }
}
