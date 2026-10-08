//! The MCP server: an rmcp [`ServerHandler`] whose tools are built at runtime from the command
//! catalog.

use std::borrow::Cow;
use std::sync::Arc;

use polarion::Polarion;
use polarion_commands::{AREAS, Args, Command, CommandError, Config, commands, find};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};

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

struct Entry {
    tool: Tool,
    kind: ToolKind,
}

/// `items.back-links` → `items_back_links`.
fn tool_name(command: &Command) -> String {
    format!("{}_{}", command.area, command.name.replace('-', "_"))
}

fn tool(
    name: String,
    description: String,
    schema: Value,
    read_only: bool,
    kind: ToolKind,
) -> Entry {
    let Value::Object(schema) = schema else {
        unreachable!("input schemas are objects")
    };
    let tool = Tool::new_with_raw(name, Some(Cow::Owned(description)), Arc::new(schema))
        .with_annotations(ToolAnnotations::new().read_only(read_only));
    Entry { tool, kind }
}

fn command_tool(name: String, command: &'static Command) -> Entry {
    tool(
        name,
        command.description(),
        command.input_schema(),
        !command.write,
        ToolKind::Command(command),
    )
}

struct Shared {
    tools: Vec<Entry>,
    polarion: Result<Polarion, CommandError>,
    config: Config,
    transport: Transport,
}

/// The tool set and the Polarion client; clones share both (one per HTTP request, all alike).
#[derive(Clone)]
pub struct PolarionMcp(Arc<Shared>);

impl std::fmt::Debug for PolarionMcp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolarionMcp")
            .field("tools", &self.0.tools.len())
            .field("config", &self.0.config)
            .field("transport", &self.0.transport)
            .finish_non_exhaustive()
    }
}

impl PolarionMcp {
    /// `all_tools`: also register every command as its own tool.
    pub fn new(config: Config, all_tools: bool, transport: Transport) -> Self {
        let mut tools: Vec<Entry> = CURATED
            .iter()
            .map(|(name, command)| {
                command_tool(
                    (*name).to_owned(),
                    find(command).expect("curated commands exist"),
                )
            })
            .collect();
        tools.push(tool(
            "polarion_commands".into(),
            "List Polarion command areas, or the commands of one area (name, summary, whether it \
             writes). Use polarion_describe for a command's arguments and polarion_call to run it."
                .into(),
            json!({
                "type": "object",
                "properties": {"area": {"type": "string", "description": "Area, e.g. `items`, `tests`, `plans`; omit to list the areas"}},
                "additionalProperties": false
            }),
            true,
            ToolKind::ListCommands,
        ));
        tools.push(tool(
            "polarion_describe".into(),
            "The description and JSON Schema of a command's arguments.".into(),
            json!({
                "type": "object",
                "properties": {"command": {"type": "string", "description": "Command name, e.g. `items.history`"}},
                "required": ["command"],
                "additionalProperties": false
            }),
            true,
            ToolKind::Describe,
        ));
        tools.push(tool(
            "polarion_call".into(),
            "Run any Polarion command by name with its arguments (see polarion_commands and \
             polarion_describe)."
                .into(),
            json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Command name, e.g. `plans.statistics`"},
                    "args": {"type": "object", "description": "Arguments as described by polarion_describe"}
                },
                "required": ["command"],
                "additionalProperties": false
            }),
            false,
            ToolKind::Call,
        ));
        if all_tools {
            let available = |c: &&Command| {
                transport == Transport::Stdio || !SESSION_BOUND.contains(&c.full_name().as_str())
            };
            for c in commands().iter().filter(available) {
                tools.push(command_tool(tool_name(c), c));
            }
        }
        Self(Arc::new(Shared {
            tools,
            polarion: config.connect(),
            config,
            transport,
        }))
    }

    /// What may be written: the projects, plus `global writes`; `None` when read-only.
    fn write_scope(&self) -> Option<String> {
        let mut scope = self.0.config.write_projects.clone();
        if self.0.config.global_writes {
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

    fn instructions(&self) -> String {
        let writes = match self.write_scope() {
            Some(scope) => format!(" Writes are enabled for: {scope}."),
            None => " This server is read-only.".to_owned(),
        };
        let project = self
            .0
            .config
            .project
            .as_deref()
            .map(|p| format!(" The default project is {p}."))
            .unwrap_or_default();
        let shared = if self.0.transport == Transport::Http {
            " Transactions are not available: all clients share one Polarion session."
        } else {
            ""
        };
        format!("{INSTRUCTIONS}{writes}{project}{shared}")
    }

    async fn run(&self, command: &Command, arguments: Value) -> Result<Value, CommandError> {
        if self.0.transport == Transport::Http
            && SESSION_BOUND.contains(&command.full_name().as_str())
        {
            return Err(CommandError::invalid(
                "transactions are not available over HTTP: all clients share one Polarion session",
            ));
        }
        let polarion = self.0.polarion.as_ref().map_err(|e| CommandError {
            code: e.code,
            message: e.message.clone(),
        })?;
        let args = Args::from_value(arguments, self.0.config.project.clone())?;
        (command.run)(polarion, &args).await
    }

    /// End the Polarion session.
    pub async fn shutdown(&self) {
        if let Ok(polarion) = &self.0.polarion {
            let _ = polarion.logout().await;
        }
    }
}

impl ServerHandler for PolarionMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "polarion-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(self.instructions())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(
            self.0.tools.iter().map(|e| e.tool.clone()).collect(),
        ))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.0
            .tools
            .iter()
            .find(|e| e.tool.name == name)
            .map(|e| e.tool.clone())
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let entry = self
            .0
            .tools
            .iter()
            .find(|e| e.tool.name == request.name)
            .ok_or_else(|| {
                McpError::invalid_params(format!("unknown tool `{}`", request.name), None)
            })?;
        let arguments = request.arguments.map_or(Value::Null, Value::Object);
        let outcome = match &entry.kind {
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
            Ok(value) => CallToolResult::success(vec![ContentBlock::text(text(&value))]),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!(
                "error ({}): {}",
                e.code, e.message
            ))]),
        }
        .into())
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
