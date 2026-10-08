//! How results are shown to people (the command line); programs always get the serialized value.

use serde_json::Value;

/// How a result is shown to people. Programs always get the JSON value.
#[derive(Debug, Clone, Copy)]
pub enum View {
    /// A list shown as a table.
    Table(&'static [Column]),
    /// An object shown as an indented `key: value` tree.
    Object,
    /// A list of strings, one per line.
    Lines,
    /// `{"outgoing": [...], "incoming": [...]}` link lists.
    Links,
    /// A number, string or boolean.
    Scalar,
    /// `{"size": .., "base64": ..}`: binary content.
    Bytes,
    /// Nothing (`null`) or a flag whether something changed.
    Done,
}

/// A table column: a header and how to get the cell from a result row.
#[derive(Debug)]
pub struct Column {
    pub header: &'static str,
    pub cell: fn(&Value) -> String,
}

pub(crate) fn str_at(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

macro_rules! columns {
    ($($header:literal => $key:literal),* $(,)?) => {
        &[$(Column { header: $header, cell: |v| str_at(v, $key) }),*]
    };
}

pub const ITEM_COLUMNS: &[Column] =
    columns!["ID" => "id", "TYPE" => "type", "STATUS" => "status", "TITLE" => "title"];
pub const DOCUMENT_ITEM_COLUMNS: &[Column] = columns![
    "#" => "outlineNumber", "ID" => "id", "TYPE" => "type", "STATUS" => "status", "TITLE" => "title"
];
pub const DOCUMENT_COLUMNS: &[Column] = &[
    Column {
        header: "DOCUMENT",
        cell: |v| format!("{}/{}", str_at(v, "space"), str_at(v, "name")),
    },
    Column {
        header: "TYPE",
        cell: |v| str_at(v, "type"),
    },
    Column {
        header: "STATUS",
        cell: |v| str_at(v, "status"),
    },
    Column {
        header: "TITLE",
        cell: |v| str_at(v, "title"),
    },
];
pub const PROJECT_COLUMNS: &[Column] = columns!["ID" => "id", "NAME" => "name"];
pub const REF_COLUMNS: &[Column] = columns!["PROJECT" => "project", "ID" => "id", "URI" => "uri"];
pub const USER_COLUMNS: &[Column] = columns!["ID" => "id", "NAME" => "name", "EMAIL" => "email"];
pub const OPTION_COLUMNS: &[Column] = columns!["ID" => "id", "NAME" => "name"];
pub const FIELD_COLUMNS: &[Column] = columns!["ID" => "id", "TYPE" => "type", "NAME" => "name"];
pub const ACTION_COLUMNS: &[Column] =
    columns!["ID" => "id", "NAME" => "name", "TARGET" => "targetStatus", "MESSAGE" => "message"];
pub const RUN_COLUMNS: &[Column] =
    columns!["ID" => "id", "TYPE" => "type", "STATUS" => "status", "TITLE" => "title"];
pub const RECORD_COLUMNS: &[Column] = &[
    Column {
        header: "TEST CASE",
        cell: |v| {
            v.get("testCase")
                .map(|t| format!("{}/{}", str_at(t, "project"), str_at(t, "id")))
                .unwrap_or_default()
        },
    },
    Column {
        header: "RESULT",
        cell: |v| str_at(v, "result"),
    },
    Column {
        header: "EXECUTED",
        cell: |v| str_at(v, "executed"),
    },
    Column {
        header: "BY",
        cell: |v| str_at(v, "executedBy"),
    },
];
pub const PLAN_COLUMNS: &[Column] =
    columns!["ID" => "id", "NAME" => "name", "STATUS" => "status", "DUE" => "dueDate"];
pub const PAGE_COLUMNS: &[Column] =
    columns!["SPACE" => "space", "NAME" => "name", "TITLE" => "title"];
pub const BASELINE_COLUMNS: &[Column] =
    columns!["NAME" => "name", "REVISION" => "revision", "DESCRIPTION" => "description"];
pub const REVISION_COLUMNS: &[Column] = columns!["REVISION" => "name", "AUTHOR" => "author", "CREATED" => "created", "MESSAGE" => "message"];
pub const CHANGE_COLUMNS: &[Column] = &[
    Column {
        header: "REVISION",
        cell: |v| str_at(v, "revision"),
    },
    Column {
        header: "DATE",
        cell: |v| str_at(v, "date"),
    },
    Column {
        header: "USER",
        cell: |v| str_at(v, "user"),
    },
    Column {
        header: "FIELDS",
        cell: |v| {
            v.get("changes")
                .and_then(Value::as_array)
                .map(|c| {
                    c.iter()
                        .map(|f| str_at(f, "field"))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_else(|| {
                    if v.get("creation") == Some(&Value::Bool(true)) {
                        "(created)".into()
                    } else {
                        String::new()
                    }
                })
        },
    },
];
pub const FOLDER_COLUMNS: &[Column] = columns!["NAME" => "name", "TITLE" => "title"];
pub const ATTACHMENT_COLUMNS: &[Column] =
    columns!["ID" => "id", "FILE" => "fileName", "TITLE" => "title", "SIZE" => "length"];
pub const GROUP_COLUMNS: &[Column] = columns!["NAME" => "name", "LOCATION" => "location"];
pub const BUILD_COLUMNS: &[Column] = columns!["ID" => "id", "STATUS" => "buildStatus", "TAG" => "buildTag", "FINISHED" => "finishTime"];
pub const ACTIVITY_COLUMNS: &[Column] =
    columns!["TIME" => "timestamp", "USER" => "userId", "TYPE" => "type", "ID" => "globalId"];
