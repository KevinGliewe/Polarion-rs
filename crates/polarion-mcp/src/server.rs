//! The MCP protocol: JSON-RPC 2.0 messages in, responses out.

use polarion::Polarion;
use polarion_commands::{AREAS, Args, Command, CommandError, Config, commands, find};
use serde_json::{Map, Value, json};

/// Protocol versions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Commands that need a Polarion session of their own: refused over HTTP, where all clients share one.
const SESSION_BOUND: &[&str] = &[
    "session.begin-transaction",
    "session.commit",
    "session.rollback",
];

/// How clients reach the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// One client, the process that started the server.
    Stdio,
    /// Any number of clients sharing the server's Polarion session.
    Http,
}

/// Everyday reads as dedicated tools: (tool name, command).
const CURATED: &[(&str, &str)] = &[
    ("get_work_item", "items.get"),
    ("query_work_items", "items.query"),
    ("count_work_items", "items.count"),
    ("work_item_links", "items.links"),
    ("work_item_history", "items.history"),
    ("list_documents", "documents.list"),
    ("get_document", "documents.get"),
    ("document_work_items", "documents.work-items"),
    ("custom_fields", "schema.custom-fields"),
    ("field_options", "schema.field-options"),
    ("search_test_runs", "tests.search"),
    ("get_test_run", "tests.get"),
    ("search_plans", "plans.search"),
    ("get_project", "projects.get"),
    ("get_user", "users.get"),
];

const INSTRUCTIONS: &str = "\
Access to a Siemens Polarion ALM server. The dedicated tools cover everyday reads; \
`polarion_commands`, `polarion_describe` and `polarion_call` reach every operation \
(about 250: work items, documents, test runs, plans, wiki, baselines, revisions, \
schema, users, permissions, ...). Work items are given as ID (in the default \
project), PROJECT/ID or a URI; documents as SPACE/NAME or PROJECT/SPACE/NAME. Queries \
use Polarion's Lucene syntax, e.g. `type:requirement AND status:open`, and return at \
most 100 results unless `limit` or `all` is given. Commands marked as writes are \
refused unless the server was started with POLARION_WRITE_PROJECTS for the target \
project.";

enum ToolKind {
    Command(&'static Command),
    ListCommands,
    Describe,
    Call,
}

struct Tool {
    name: String,
    description: String,
    input_schema: Value,
    read_only: bool,
    kind: ToolKind,
}

/// The tool set and the Polarion client, shared by every client of the transport.
pub struct Server {
    tools: Vec<Tool>,
    polarion: Result<Polarion, CommandError>,
    config: Config,
    transport: Transport,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("tools", &self.tools.len())
            .field("config", &self.config)
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

/// `items.back-links` → `items_back_links`.
fn tool_name(command: &Command) -> String {
    format!("{}_{}", command.area, command.name.replace('-', "_"))
}

impl Server {
    /// `all_tools`: also register every command as its own tool.
    pub fn new(config: Config, all_tools: bool, transport: Transport) -> Self {
        let mut tools: Vec<Tool> = CURATED
            .iter()
            .map(|(name, command)| {
                let c = find(command).expect("curated commands exist");
                Tool {
                    name: (*name).to_owned(),
                    description: c.description(),
                    input_schema: c.input_schema(),
                    read_only: !c.write,
                    kind: ToolKind::Command(c),
                }
            })
            .collect();
        tools.push(Tool {
            name: "polarion_commands".into(),
            description: "List Polarion command areas, or the commands of one area (name, summary, \
                          whether it writes). Use polarion_describe for a command's arguments and \
                          polarion_call to run it."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {"area": {"type": "string", "description": "Area, e.g. `items`, `tests`, `plans`; omit to list the areas"}},
                "additionalProperties": false
            }),
            read_only: true,
            kind: ToolKind::ListCommands,
        });
        tools.push(Tool {
            name: "polarion_describe".into(),
            description: "The description and JSON Schema of a command's arguments.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"command": {"type": "string", "description": "Command name, e.g. `items.history`"}},
                "required": ["command"],
                "additionalProperties": false
            }),
            read_only: true,
            kind: ToolKind::Describe,
        });
        tools.push(Tool {
            name: "polarion_call".into(),
            description: "Run any Polarion command by name with its arguments (see polarion_commands \
                          and polarion_describe)."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Command name, e.g. `plans.statistics`"},
                    "args": {"type": "object", "description": "Arguments as described by polarion_describe"}
                },
                "required": ["command"],
                "additionalProperties": false
            }),
            read_only: false,
            kind: ToolKind::Call,
        });
        if all_tools {
            let available = |c: &&Command| {
                transport == Transport::Stdio || !SESSION_BOUND.contains(&c.full_name().as_str())
            };
            for c in commands().iter().filter(available) {
                tools.push(Tool {
                    name: tool_name(c),
                    description: c.description(),
                    input_schema: c.input_schema(),
                    read_only: !c.write,
                    kind: ToolKind::Command(c),
                });
            }
        }
        Self {
            tools,
            polarion: config.connect(),
            config,
            transport,
        }
    }

    /// What may be written: the projects, plus `global writes`; `None` when read-only.
    fn write_scope(&self) -> Option<String> {
        let mut scope = self.config.write_projects.clone();
        if self.config.global_writes {
            scope.push("global writes".to_owned());
        }
        (!scope.is_empty()).then(|| scope.join(", "))
    }

    /// `read-only`, or what may be written.
    pub fn access(&self) -> String {
        match self.write_scope() {
            Some(scope) => format!("writes enabled for {scope}"),
            None => "read-only".to_owned(),
        }
    }

    /// Handle one message; `None` for notifications (no response).
    pub async fn handle(&self, message: &Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        let Some(id) = id else {
            // Notifications (`notifications/initialized`, `notifications/cancelled`, ...).
            return None;
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            Some("initialize") => Ok(self.initialize(&params)),
            Some("ping") => Ok(json!({})),
            Some("tools/list") => Ok(self.list_tools()),
            Some("tools/call") => self.call_tool(&params).await,
            Some(other) => Err((-32601, format!("method not found: {other}"))),
            None => Err((-32600, "invalid request: no method".to_owned())),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
            }
        })
    }

    fn initialize(&self, params: &Value) -> Value {
        let requested = params.get("protocolVersion").and_then(Value::as_str);
        let version = requested
            .filter(|v| PROTOCOL_VERSIONS.contains(v))
            .unwrap_or(PROTOCOL_VERSIONS[0]);
        let writes = match self.write_scope() {
            Some(scope) => format!(" Writes are enabled for: {scope}."),
            None => " This server is read-only.".to_owned(),
        };
        let project = self
            .config
            .project
            .as_deref()
            .map(|p| format!(" The default project is {p}."))
            .unwrap_or_default();
        let shared = if self.transport == Transport::Http {
            " Transactions are not available: all clients share one Polarion session."
        } else {
            ""
        };
        json!({
            "protocolVersion": version,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "polarion-mcp", "version": env!("CARGO_PKG_VERSION")},
            "instructions": format!("{INSTRUCTIONS}{writes}{project}{shared}"),
        })
    }

    fn list_tools(&self) -> Value {
        let tools: Vec<Value> = self
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.input_schema,
                    "annotations": {"readOnlyHint": t.read_only},
                })
            })
            .collect();
        json!({ "tools": tools })
    }

    async fn call_tool(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((-32602, "tools/call: missing `name`".to_owned()))?;
        let tool = self
            .tools
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| (-32602, format!("unknown tool `{name}`")))?;
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
        let outcome = match &tool.kind {
            ToolKind::Command(c) => self.run(c, arguments).await,
            ToolKind::ListCommands => list_commands(&arguments),
            ToolKind::Describe => describe(&arguments),
            ToolKind::Call => {
                let command = arguments
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match find(command) {
                    None => Err(CommandError::invalid(format!(
                        "unknown command `{command}` (see polarion_commands)"
                    ))),
                    Some(c) => {
                        self.run(c, arguments.get("args").cloned().unwrap_or(Value::Null))
                            .await
                    }
                }
            }
        };
        Ok(match outcome {
            Ok(value) => json!({
                "content": [{"type": "text", "text": text(&value)}],
                "isError": false,
            }),
            Err(e) => json!({
                "content": [{"type": "text", "text": format!("error ({}): {}", e.code, e.message)}],
                "isError": true,
            }),
        })
    }

    async fn run(&self, command: &Command, arguments: Value) -> Result<Value, CommandError> {
        if self.transport == Transport::Http
            && SESSION_BOUND.contains(&command.full_name().as_str())
        {
            return Err(CommandError::invalid(
                "transactions are not available over HTTP: all clients share one Polarion session",
            ));
        }
        let polarion = self.polarion.as_ref().map_err(|e| CommandError {
            code: e.code,
            message: e.message.clone(),
        })?;
        let args = Args::from_value(arguments, self.config.project.clone())?;
        (command.run)(polarion, &args).await
    }

    /// End the Polarion session.
    pub async fn shutdown(&self) {
        if let Ok(polarion) = &self.polarion {
            let _ = polarion.logout().await;
        }
    }
}

/// The result as tool text: compact JSON, `done` for no result.
fn text(value: &Value) -> String {
    match value {
        Value::Null => "done".to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn list_commands(arguments: &Value) -> Result<Value, CommandError> {
    match arguments.get("area").and_then(Value::as_str) {
        None => Ok(Value::Array(
            AREAS
                .iter()
                .map(|(area, about)| {
                    let count = commands().iter().filter(|c| c.area == *area).count();
                    json!({"area": area, "description": about, "commands": count})
                })
                .collect(),
        )),
        Some(area) => {
            let list: Vec<Value> = commands()
                .iter()
                .filter(|c| c.area == area)
                .map(|c| {
                    let mut entry = Map::new();
                    entry.insert("name".into(), Value::from(c.full_name()));
                    entry.insert("summary".into(), Value::from(c.summary));
                    if c.write {
                        entry.insert("write".into(), Value::Bool(true));
                    }
                    Value::Object(entry)
                })
                .collect();
            if list.is_empty() {
                Err(CommandError::invalid(format!("unknown area `{area}`")))
            } else {
                Ok(Value::Array(list))
            }
        }
    }
}

fn describe(arguments: &Value) -> Result<Value, CommandError> {
    let name = arguments
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let c = find(name).ok_or_else(|| {
        CommandError::invalid(format!("unknown command `{name}` (see polarion_commands)"))
    })?;
    Ok(json!({
        "name": c.full_name(),
        "description": c.description(),
        "write": c.write,
        "inputSchema": c.input_schema(),
    }))
}
