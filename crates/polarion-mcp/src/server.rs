//! The MCP server: an rmcp [`ServerHandler`] whose tools are built at runtime from the command
//! catalog.

use std::borrow::Cow;
use std::sync::Arc;

use polarion::Polarion;
use polarion_commands::{AREAS, CommandError, Config, Context, Entry, View, catalog, find};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};

use crate::config::{self, Selection};
use crate::files::{self, Exchange};

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
Work items are given as ID (in the default \
project), PROJECT/ID or a URI; documents as SPACE/NAME or PROJECT/SPACE/NAME. Queries \
use Polarion's Lucene syntax, e.g. `type:requirement AND status:open`, and return at \
most 100 results unless `limit` or `all` is given. Commands marked as writes are \
refused unless the server was started with POLARION_WRITE_PROJECTS for the target \
project.";

enum ToolKind {
    Command(&'static Entry),
    ListCommands,
    Describe,
    Call,
}

struct Registered {
    tool: Tool,
    kind: ToolKind,
}

fn tool(
    name: String,
    description: String,
    schema: Value,
    read_only: bool,
    kind: ToolKind,
) -> Registered {
    let Value::Object(schema) = schema else {
        unreachable!("input schemas are objects")
    };
    let tool = Tool::new_with_raw(name, Some(Cow::Owned(description)), Arc::new(schema))
        .with_annotations(ToolAnnotations::new().read_only(read_only));
    Registered { tool, kind }
}

fn command_tool(
    command: &'static Entry,
    exchange: Option<&Exchange>,
    output_schemas: bool,
) -> Registered {
    let mut registered = tool(
        config::tool_name(command),
        command.description.to_owned(),
        files::schema(command, exchange),
        !command.write,
        ToolKind::Command(command),
    );
    if let Some((Value::Object(schema), _)) = output_schema(command).filter(|_| output_schemas) {
        registered.tool = registered.tool.with_raw_output_schema(Arc::new(schema));
    }
    registered
}

/// A command's MCP output schema, and whether its results are wrapped as `{"result": ...}`:
/// protocol versions before 2026-07-28 (and the clients built for them) want an object at the
/// root. `None` for commands without a structured result (nothing, or file content).
fn output_schema(command: &Entry) -> Option<(Value, bool)> {
    if matches!(command.view, View::Done | View::Bytes) {
        return None;
    }
    let mut schema = command.output_schema();
    if schema.get("type").and_then(Value::as_str) == Some("object") {
        return Some((schema, false));
    }
    // References of recursive types point at the root's `$defs`.
    let defs = schema.as_object_mut().and_then(|o| o.remove("$defs"));
    let mut wrapper = json!({
        "type": "object",
        "properties": {"result": schema},
        "required": ["result"],
    });
    if let Some(defs) = defs {
        wrapper["$defs"] = defs;
    }
    Some((wrapper, true))
}

/// One of the tools that reach every command (`config::GENERIC`).
fn generic_tool(name: &str) -> Registered {
    match name {
        "polarion_commands" => tool(
            name.into(),
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
        ),
        "polarion_describe" => tool(
            name.into(),
            "The description and JSON Schema of a command's arguments.".into(),
            json!({
                "type": "object",
                "properties": {"command": {"type": "string", "description": "Command name, e.g. `items.history`"}},
                "required": ["command"],
                "additionalProperties": false
            }),
            true,
            ToolKind::Describe,
        ),
        "polarion_call" => tool(
            name.into(),
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
        ),
        other => unreachable!("`{other}` is not a generic tool"),
    }
}

struct Shared {
    tools: Vec<Registered>,
    selection: Selection,
    polarion: Result<Polarion, CommandError>,
    config: Config,
    transport: Transport,
    exchange: Option<Exchange>,
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
    /// `selection`: the tools to offer and the commands switched off. `exchange`: the folder
    /// files are saved to and uploaded from.
    pub fn new(
        config: Config,
        selection: Selection,
        transport: Transport,
        exchange: Option<Exchange>,
    ) -> Self {
        let available = |c: &&&Entry| {
            transport == Transport::Stdio || !SESSION_BOUND.contains(&c.full_name().as_str())
        };
        let (curated, others): (Vec<&&Entry>, Vec<&&Entry>) = selection
            .tools
            .iter()
            .filter(available)
            .partition(|c| config::is_curated(c));
        let command_tool =
            |c: &&&'static Entry| command_tool(c, exchange.as_ref(), selection.output_schemas);
        let tools = curated
            .iter()
            .map(command_tool)
            .chain(selection.generic.iter().map(|g| generic_tool(g)))
            .chain(others.iter().map(command_tool))
            .collect();
        Self(Arc::new(Shared {
            tools,
            selection,
            polarion: config.connect(),
            config,
            transport,
            exchange,
        }))
    }

    /// A command by name for the generic tools: unknown and switched-off commands are errors.
    fn command(&self, name: &str) -> Result<&'static Entry, CommandError> {
        let command = find(name).ok_or_else(|| {
            CommandError::invalid(format!("unknown command `{name}` (see polarion_commands)"))
        })?;
        if self.0.selection.is_disabled(command) {
            return Err(CommandError::invalid(format!(
                "command `{name}` is disabled on this server"
            )));
        }
        Ok(command)
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
        let files = match &self.0.exchange {
            Some(exchange) => format!(
                " Attachment downloads and PDF exports can be saved with `save_as`, and uploads read \
                 with `data-file`, in the exchange folder {}{}.",
                exchange.dir().display(),
                if self.0.transport == Transport::Http {
                    " (on the server)"
                } else {
                    ""
                }
            ),
            None => String::new(),
        };
        format!(
            "Access to a Siemens Polarion ALM server. {}{INSTRUCTIONS}{writes}{project}{shared}{files}",
            self.generic_instructions()
        )
    }

    /// What the generic tools offer, as far as they are listed.
    fn generic_instructions(&self) -> String {
        let selection = &self.0.selection;
        let mut text = String::new();
        if selection.lists("polarion_call") {
            let enabled = catalog()
                .iter()
                .filter(|c| !selection.is_disabled(c))
                .count();
            text.push_str(&format!(
                "Besides the dedicated tools, `polarion_call` runs any of {enabled} commands \
                 (work items, documents, test runs, plans, wiki, baselines, revisions, schema, \
                 users, permissions, ...)"
            ));
            let helpers: Vec<&str> = [
                ("polarion_commands", "`polarion_commands` lists them"),
                (
                    "polarion_describe",
                    "`polarion_describe` shows their arguments",
                ),
            ]
            .into_iter()
            .filter(|(tool, _)| selection.lists(tool))
            .map(|(_, text)| text)
            .collect();
            if !helpers.is_empty() {
                text.push_str("; ");
                text.push_str(&helpers.join(" and "));
            }
            text.push_str(". ");
        }
        if selection.disables_commands() {
            text.push_str("Some commands are disabled on this server. ");
        }
        text
    }

    async fn run(&self, command: &Entry, arguments: Value) -> Result<Value, CommandError> {
        if self.0.transport == Transport::Http
            && SESSION_BOUND.contains(&command.full_name().as_str())
        {
            return Err(CommandError::invalid(
                "transactions are not available over HTTP: all clients share one Polarion session",
            ));
        }
        let command = command.parse(arguments)?;
        let polarion = self.0.polarion.as_ref().map_err(|e| CommandError {
            code: e.code,
            message: e.message.clone(),
        })?;
        let cx = Context::new(polarion, self.0.config.project.as_deref());
        let output = command.run(&cx).await?;
        Ok(serde_json::to_value(&output).expect("results serialize"))
    }

    /// Run a command for the agent: exchange arguments in, byte results as image/text blocks.
    /// `structured`: also return the result as structured content (tools with an output schema).
    async fn execute(&self, command: &Entry, arguments: Value, structured: bool) -> CallToolResult {
        let exchange = self.0.exchange.as_ref();
        let outcome = async {
            let (arguments, save_as) = files::prepare(command, arguments, exchange)?;
            let value = self.run(command, arguments).await?;
            if matches!(command.view, View::Bytes) {
                return Ok((files::present(&value, save_as.as_deref(), exchange)?, None));
            }
            let content = vec![ContentBlock::text(text(&value))];
            let structured = match structured.then(|| output_schema(command)).flatten() {
                Some((_, true)) => Some(json!({ "result": value })),
                Some((_, false)) => Some(value),
                None => None,
            };
            Ok((content, structured))
        };
        match outcome.await {
            Ok((content, structured)) => {
                let mut result = CallToolResult::success(content);
                result.structured_content = structured;
                result
            }
            Err(e) => error_result(&e),
        }
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
        let lookup = match &entry.kind {
            ToolKind::Command(c) => {
                let structured = self.0.selection.output_schemas;
                return Ok(self.execute(c, arguments, structured).await.into());
            }
            ToolKind::Call => {
                let name = arguments
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match self.command(name) {
                    Ok(c) => {
                        let args = arguments.get("args").cloned().unwrap_or(Value::Null);
                        return Ok(self.execute(c, args, false).await.into());
                    }
                    Err(e) => Err(e),
                }
            }
            ToolKind::ListCommands => list_commands(&arguments, &self.0.selection),
            ToolKind::Describe => {
                let name = arguments
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.command(name)
                    .map(|c| describe(c, self.0.exchange.as_ref()))
            }
        };
        Ok(match lookup {
            Ok(value) => CallToolResult::success(vec![ContentBlock::text(text(&value))]),
            Err(e) => error_result(&e),
        }
        .into())
    }
}

fn error_result(e: &CommandError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!(
        "error ({}): {}",
        e.code, e.message
    ))])
}

/// The result as tool text: compact JSON, `done` for no result.
fn text(value: &Value) -> String {
    match value {
        Value::Null => "done".to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The commands of `area` that are not switched off.
fn enabled<'a>(
    area: &'a str,
    selection: &'a Selection,
) -> impl Iterator<Item = &'static Entry> + 'a {
    catalog()
        .iter()
        .filter(move |c| c.area == area && !selection.is_disabled(c))
}

fn list_commands(arguments: &Value, selection: &Selection) -> Result<Value, CommandError> {
    match arguments.get("area").and_then(Value::as_str) {
        None => Ok(Value::Array(
            AREAS
                .iter()
                .filter_map(|(area, about)| {
                    let count = enabled(area, selection).count();
                    (count > 0)
                        .then(|| json!({"area": area, "description": about, "commands": count}))
                })
                .collect(),
        )),
        Some(area) => {
            let list: Vec<Value> = enabled(area, selection)
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

fn describe(c: &Entry, exchange: Option<&Exchange>) -> Value {
    json!({
        "name": c.full_name(),
        "description": c.description,
        "write": c.write,
        "inputSchema": files::schema(c, exchange),
        "outputSchema": c.output_schema(),
    })
}
