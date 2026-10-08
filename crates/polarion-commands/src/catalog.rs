//! The command catalog types.

use std::future::Future;
use std::pin::Pin;

use polarion::Polarion;
use serde_json::{Map, Value, json};

use crate::args::Args;
use crate::error::Result;

pub type BoxFuture<'a> = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;
pub type Handler = for<'a> fn(&'a Polarion, &'a Args) -> BoxFuture<'a>;

/// One operation of the library as a named command taking and returning JSON.
#[derive(Clone, Copy)]
pub struct Command {
    /// Area, e.g. `items`; the command's full name is `area.name`.
    pub area: &'static str,
    /// Kebab-case name within the area, e.g. `back-links`.
    pub name: &'static str,
    pub summary: &'static str,
    pub params: &'static [Param],
    /// Modifies data (refused unless writes are enabled for the target).
    pub write: bool,
    pub output: Output,
    pub run: Handler,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Command")
            .field("name", &self.full_name())
            .field("write", &self.write)
            .finish_non_exhaustive()
    }
}

impl Command {
    /// `area.name`.
    pub fn full_name(&self) -> String {
        format!("{}.{}", self.area, self.name)
    }

    /// JSON Schema of the arguments object.
    pub fn input_schema(&self) -> Value {
        let mut properties = Map::new();
        let mut required = Vec::new();
        for p in self.params {
            properties.insert(p.name.to_owned(), p.kind.schema(p.help));
            if p.required {
                required.push(Value::from(p.name));
            }
        }
        json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
    }

    /// The summary plus a note for writes.
    pub fn description(&self) -> String {
        if self.write {
            format!(
                "{} (Modifies data: needs POLARION_WRITE_PROJECTS for the target project.)",
                self.summary
            )
        } else {
            self.summary.to_owned()
        }
    }
}

/// A command parameter.
#[derive(Debug, Clone, Copy)]
pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
    /// Taken positionally on the command line (in declaration order).
    pub positional: bool,
    pub help: &'static str,
}

impl Param {
    pub const fn req(name: &'static str, kind: Kind, help: &'static str) -> Self {
        Self {
            name,
            kind,
            required: true,
            positional: false,
            help,
        }
    }

    pub const fn opt(name: &'static str, kind: Kind, help: &'static str) -> Self {
        Self {
            name,
            kind,
            required: false,
            positional: false,
            help,
        }
    }

    /// Required and positional.
    pub const fn pos(name: &'static str, kind: Kind, help: &'static str) -> Self {
        Self {
            name,
            kind,
            required: true,
            positional: true,
            help,
        }
    }

    /// Optional and positional (must come after the required positionals).
    pub const fn pos_opt(name: &'static str, kind: Kind, help: &'static str) -> Self {
        Self {
            name,
            kind,
            required: false,
            positional: true,
            help,
        }
    }
}

/// Parameter types. On the command line every value is a string; [`crate::Args`] parses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    String,
    Integer,
    Number,
    Boolean,
    /// A list of strings (JSON array, or comma-separated).
    StringList,
    /// A work item: `ID` (in the default project), `PROJECT/ID`, or a URI.
    Item,
    /// A list of work items.
    ItemList,
    /// A document: `SPACE/NAME` (in the default project), `PROJECT/SPACE/NAME`, or a URI.
    Document,
    /// Rich text: a string (plain text) or `{"type": "text/html", "content": ".."}`.
    Text,
    /// `standard`, `all`, or a list of field keys.
    Fields,
    /// A JSON object.
    Json,
    /// Binary content: base64 (on the command line `@FILE` reads a file).
    Bytes,
    /// Any JSON value (e.g. a custom field value; `null` clears).
    Any,
}

impl Kind {
    fn schema(self, help: &str) -> Value {
        let mut s = match self {
            Kind::String => json!({"type": "string"}),
            Kind::Integer => json!({"type": "integer"}),
            Kind::Number => json!({"type": "number"}),
            Kind::Boolean => json!({"type": "boolean"}),
            Kind::StringList => json!({"type": "array", "items": {"type": "string"}}),
            Kind::Item => json!({"type": "string"}),
            Kind::ItemList => json!({"type": "array", "items": {"type": "string"}}),
            Kind::Document => json!({"type": "string"}),
            Kind::Text => json!({"anyOf": [
                {"type": "string"},
                {"type": "object", "properties": {"type": {"type": "string"}, "content": {"type": "string"}}, "required": ["content"]}
            ]}),
            Kind::Fields => json!({"anyOf": [
                {"type": "string", "description": "`standard` or `all`"},
                {"type": "array", "items": {"type": "string"}}
            ]}),
            Kind::Json => json!({"type": "object"}),
            Kind::Bytes => json!({"type": "string", "contentEncoding": "base64"}),
            Kind::Any => json!({}),
        };
        s["description"] = Value::from(help);
        s
    }
}

/// How a result is shown to people. Programs always get the JSON value.
#[derive(Debug, Clone, Copy)]
pub enum Output {
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

// ---- parameters shared by many commands

pub(crate) const ITEM: Param = Param::pos(
    "item",
    Kind::Item,
    "Work item: ID (in the default project), PROJECT/ID, or URI",
);
pub(crate) const DOCUMENT: Param = Param::pos(
    "document",
    Kind::Document,
    "Document: SPACE/NAME (in the default project), PROJECT/SPACE/NAME, or URI",
);
pub(crate) const PROJECT: Param = Param::opt(
    "project",
    Kind::String,
    "Project id (default: POLARION_PROJECT or -p)",
);
pub(crate) const QUERY: Param = Param::pos_opt(
    "query",
    Kind::String,
    "Lucene query, e.g. `type:requirement AND status:open`; empty matches everything",
);
pub(crate) const SORT: Param = Param::opt(
    "sort",
    Kind::String,
    "Sort order, e.g. `id`, `~updated` (descending)",
);
pub(crate) const LIMIT: Param = Param::opt(
    "limit",
    Kind::Integer,
    "Maximum number of results (default 100)",
);
pub(crate) const ALL: Param = Param::opt("all", Kind::Boolean, "Return every match (no limit)");
pub(crate) const FIELDS: Param = Param::opt(
    "fields",
    Kind::Fields,
    "Fields to fetch: `standard`, `all`, or a list of field keys",
);
pub(crate) const CUSTOM: Param = Param::opt(
    "custom",
    Kind::StringList,
    "Custom field keys to fetch in addition",
);
pub(crate) const BASELINE: Param = Param::opt(
    "baseline",
    Kind::String,
    "Search as of this baseline revision",
);
pub(crate) const REVISION: Param = Param::opt(
    "revision",
    Kind::String,
    "Return work items as they were in this revision",
);
pub(crate) const SQL: Param = Param::pos("sql", Kind::String, "SQL query (SELECT ... FROM ...)");
pub(crate) const URI: Param = Param::pos("uri", Kind::String, "Object URI");
pub(crate) const TEXT: Param = Param::req(
    "text",
    Kind::Text,
    "Text: a string (plain text) or {type: text/html, content}",
);
