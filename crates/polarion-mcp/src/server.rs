//! The MCP server: an rmcp [`ServerHandler`] whose tools are built at runtime from the command
//! catalog.

use std::borrow::Cow;
use std::sync::Arc;

use polarion::Polarion;
use polarion_commands::{
    AREAS, CommandError, Config, Context, Entry, View, WriteTarget, catalog, find,
};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};
use tokio::runtime::RuntimeFlavor;

use crate::config::{self, RequireGuides, Selection};
use crate::files::{self, Exchange};
use crate::guides::{self, Guide};

/// Commands that change or renew the Polarion session: refused over HTTP, where all clients share
/// one. (Commands of several writes open a session of their own for them and are fine there.)
const SESSION_BOUND: &[&str] = &[
    "session.begin-transaction",
    "session.commit",
    "session.rollback",
    "session.check",
];

/// Commands that keep their thread busy for up to seconds (they render mermaid diagrams).
const CPU_BOUND: &[&str] = &[
    "items.set-description-markdown",
    "items.append-description-markdown",
];

/// What a server may write.
enum Scope {
    ReadOnly,
    /// Projects, and `global writes`.
    Projects(String),
    /// Only these objects (`POLARION_WRITE_TARGETS`).
    Targets(String),
}

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
project, or with POLARION_WRITE_TARGETS naming the object.";

/// What a curated write tool changes or names: the kind of object a write target must be for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    WorkItem,
    Document,
    TestRun,
    Plan,
}

/// The areas of the curated write tools: (command area, objects, one object, the kind its
/// writes change).
const WRITE_AREAS: &[(&str, &str, &str, Kind)] = &[
    ("items", "work items", "work item", Kind::WorkItem),
    ("documents", "documents", "document", Kind::Document),
    ("tests", "test runs", "test run", Kind::TestRun),
    ("plans", "plans", "plan", Kind::Plan),
];

impl Kind {
    /// The kinds of object the curated write `command` changes (the first) or always names
    /// (the library requires write targets of all of them in restricted mode):
    /// `tests.set-steps` writes a test case, a work item; a test record names its test case,
    /// plan items are work items.
    fn needed(command: &str) -> &'static [Self] {
        match command {
            "tests.set-steps" => &[Self::WorkItem],
            "tests.add-record" => &[Self::TestRun, Self::WorkItem],
            "plans.add-items" | "plans.remove-items" => &[Self::Plan, Self::WorkItem],
            _ => match command.split_once('.').map(|(area, _)| area) {
                Some("documents") => &[Self::Document],
                Some("tests") => &[Self::TestRun],
                Some("plans") => &[Self::Plan],
                _ => &[Self::WorkItem],
            },
        }
    }

    /// The kind of a write target; `None` for other objects (comments, baselines, ...). A URI
    /// is taken by its object marker (`…/PROJ${Plan}ID` is a plan).
    fn of_target(target: &WriteTarget) -> Option<Self> {
        match target {
            WriteTarget::Item(_) => Some(Self::WorkItem),
            WriteTarget::Document(_) => Some(Self::Document),
            WriteTarget::TestRun { .. } => Some(Self::TestRun),
            WriteTarget::Uri(uri) => {
                let marker = uri.split_once("${")?.1.split_once('}')?.0;
                match marker {
                    "WorkItem" => Some(Self::WorkItem),
                    "Module" => Some(Self::Document),
                    "TestRun" => Some(Self::TestRun),
                    "Plan" => Some(Self::Plan),
                    _ => None,
                }
            }
        }
    }
}

/// `a`, `a and b`, `a, b and c`.
fn join_and(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The commands that read and write descriptions as Markdown, and whether they write.
const MARKDOWN: &[(&str, bool)] = &[
    ("items.description-markdown", false),
    ("items.set-description-markdown", true),
    ("items.append-description-markdown", true),
];

enum ToolKind {
    Command(&'static Entry),
    Guide,
    ListCommands,
    Describe,
    Call,
}

/// The media type of the guides.
const MARKDOWN_TYPE: &str = "text/markdown";

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
        "polarion_guide" => {
            let areas: Vec<&str> = Guide::ALL.iter().map(|g| g.name()).collect();
            let about: Vec<String> = Guide::ALL
                .iter()
                .map(|g| format!("`{}`: {}", g.name(), g.about()))
                .collect();
            tool(
                name.into(),
                format!(
                    "The guide to one area of this server (Markdown), with a section read from the project: its work item types, statuses and, for work items, link roles and custom fields. Read an area's guide before using its tools. Areas: {}.",
                    about.join("; ")
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "area": {"type": "string", "enum": areas, "description": "The area: `overview` for the server as a whole, `queries` for the query syntax"},
                        "project": {"type": "string", "description": "Project id for the project section (default: the default project)"}
                    },
                    "required": ["area"],
                    "additionalProperties": false
                }),
                true,
                ToolKind::Guide,
            )
        }
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
    /// The write targets (`POLARION_WRITE_TARGETS`); `None` when writes are not restricted to
    /// targets.
    targets: Option<Vec<WriteTarget>>,
    transport: Transport,
    exchange: Option<Exchange>,
    /// The guides read by `polarion_guide` in this process (all clients over HTTP).
    read: guides::Read,
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
        // The curated reads, then the curated writes (only selected when writes are enabled),
        // then the generic tools, then the rest.
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
        // Targets that do not resolve leave the client unusable (`connect` fails): nothing is
        // written, and the instructions say so.
        let targets = config
            .resolve_write_targets()
            .unwrap_or_else(|_| Some(Vec::new()));
        Self(Arc::new(Shared {
            tools,
            selection,
            polarion: config.connect(),
            config,
            targets,
            transport,
            exchange,
            read: guides::Read::default(),
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

    /// What may be written: the write targets, or the projects plus `global writes`.
    fn write_scope(&self) -> Scope {
        if let Some(targets) = &self.0.targets {
            if targets.is_empty() {
                return Scope::ReadOnly;
            }
            let names: Vec<String> = targets.iter().map(ToString::to_string).collect();
            return Scope::Targets(names.join(", "));
        }
        let mut scope = self.0.config.write_projects.clone();
        if self.0.config.global_writes {
            scope.push("global writes".to_owned());
        }
        if scope.is_empty() {
            Scope::ReadOnly
        } else {
            Scope::Projects(scope.join(", "))
        }
    }

    /// Whether objects of all `kinds` may be written at all: write projects, or a write target
    /// of each kind; global writes alone are not enough. The library writes plans only with
    /// write projects or to targets that are plans.
    fn writable(&self, kinds: &[Kind]) -> bool {
        self.0.config.writes_enabled()
            && match &self.0.targets {
                Some(targets) => kinds
                    .iter()
                    .all(|k| targets.iter().any(|t| Kind::of_target(t) == Some(*k))),
                None => !self.0.config.write_projects.is_empty(),
            }
    }

    /// Whether the curated write `command` can go through at all: [`Self::writable`] for what
    /// it changes and names; with write targets, creates are always refused.
    fn tool_writable(&self, command: &str) -> bool {
        let create = command.ends_with(".create") && self.0.targets.is_some();
        !create && self.writable(Kind::needed(command))
    }

    /// Whether anything may be written at all.
    fn writes(&self) -> bool {
        !matches!(self.write_scope(), Scope::ReadOnly)
    }

    /// The guide `command` needs first, while it was not read (`require_guides`): with `write`,
    /// the commands that write, as far as anything may be written; with `all`, every command.
    fn required_guide(&self, command: &Entry) -> Option<Guide> {
        let required = match self.0.selection.require_guides {
            RequireGuides::None => false,
            RequireGuides::Write => command.write && self.writes(),
            RequireGuides::All => true,
        };
        let guide = Guide::of_area(command.area);
        (required && !self.0.read.has(guide)).then_some(guide)
    }

    /// Whether `command` may run here, before anything is sent: no transactions over HTTP, and
    /// the guide of its area read when the server requires that. The library's write guard
    /// checks the write afterwards, as always.
    fn admit(&self, command: &Entry) -> Result<(), CommandError> {
        if self.0.transport == Transport::Http
            && SESSION_BOUND.contains(&command.full_name().as_str())
        {
            return Err(CommandError::invalid(format!(
                "`{}` is not available over HTTP: all clients share one Polarion session (commands \
                 of several writes run as one transaction in a session of their own anyway)",
                command.full_name()
            )));
        }
        match self.required_guide(command) {
            None => Ok(()),
            Some(guide) => Err(CommandError {
                code: "guideRequired",
                message: format!(
                    "`{}` needs the {} guide first: call polarion_guide with {{\"area\": \"{}\"}}, \
                     then repeat this call (nothing was sent)",
                    command.full_name(),
                    guide.name(),
                    guide.name()
                ),
            }),
        }
    }

    /// `read-only`, or what may be written.
    pub fn access(&self) -> String {
        match self.write_scope() {
            Scope::ReadOnly => "read-only".to_owned(),
            Scope::Projects(scope) => format!("writes enabled for {scope}"),
            Scope::Targets(targets) => format!("writes restricted to {targets}"),
        }
    }

    fn instructions(&self) -> String {
        let writes = match self.write_scope() {
            Scope::ReadOnly => " This server is read-only.".to_owned(),
            Scope::Projects(scope) => format!(" Writes are enabled for: {scope}."),
            Scope::Targets(targets) => format!(
                " Writes are restricted to these objects: {targets}. Every other write is \
                 refused, and every object a write names (link targets, test cases, defects, \
                 plan items) must be one of them too. Refused even for these objects: creates, \
                 deletes of whole objects, workflow actions, status and resolution changes, \
                 and the body of a document that holds work items. Their attachments, work \
                 records and test records may be added, changed and deleted. A change can still \
                 mark links from other work items to a target as suspect, and the server may \
                 send notifications."
            ),
        };
        let project = self
            .0
            .config
            .project
            .as_deref()
            .map(|p| format!(" The default project is {p}."))
            .unwrap_or_default();
        let shared = if self.0.transport == Transport::Http {
            " Explicit transactions and session checks are not available: all clients share one \
             Polarion session. Commands of several writes (a create with categories or links, an \
             update of several fields) still run as one transaction, in a session of their own."
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
            "Access to a Siemens Polarion ALM server. {}{}{INSTRUCTIONS}{}{writes}{}{project}{shared}{files}",
            self.guide_instructions(),
            self.generic_instructions(),
            self.markdown_instructions(),
            self.write_tool_instructions(),
        )
    }

    /// The curated write tools that are listed (`config::CURATED_WRITES`; only with writes
    /// enabled), by area, and those that are always refused (`tool_writable`): an area's when
    /// all its listed tools are refused and one of them changes the area's own kind, otherwise
    /// single tools.
    fn write_tool_instructions(&self) -> String {
        let listed: Vec<&(&str, &str)> = config::CURATED_WRITES
            .iter()
            .filter(|(name, _)| self.0.tools.iter().any(|t| t.tool.name == *name))
            .collect();
        let mut groups = Vec::new();
        let mut refused_areas = Vec::new();
        let mut refused_tools = Vec::new();
        for (area, objects, object, kind) in WRITE_AREAS {
            let tools: Vec<&&(&str, &str)> = listed
                .iter()
                .filter(|(_, command)| command.split_once('.').is_some_and(|(a, _)| a == *area))
                .collect();
            if tools.is_empty() {
                continue;
            }
            let names: Vec<String> = tools.iter().map(|(name, _)| format!("`{name}`")).collect();
            groups.push(format!("{objects} ({})", names.join(", ")));
            let refused: Vec<String> = tools
                .iter()
                .filter(|(_, command)| !self.tool_writable(command))
                .map(|(name, _)| format!("`{name}`"))
                .collect();
            let own_kind = tools.iter().any(|(_, c)| Kind::needed(c)[0] == *kind);
            if refused.len() == tools.len() && own_kind {
                refused_areas.push((*object).to_owned());
            } else {
                refused_tools.extend(refused);
            }
        }
        if groups.is_empty() {
            return String::new();
        }
        let mut text = format!(" Write tools of their own for {}.", join_and(&groups));
        let mut refused = Vec::new();
        if !refused_areas.is_empty() {
            refused.push(format!("the {} write tools", join_and(&refused_areas)));
        }
        refused.extend(refused_tools);
        if !refused.is_empty() {
            text.push_str(&format!(
                " Of these, this server refuses {}: its write scope does not cover them.",
                join_and(&refused)
            ));
        }
        text
    }

    /// How to read and write descriptions as Markdown, as far as this server offers it: the
    /// commands that are not disabled, by their tool or through `polarion_call`, and the writes
    /// only when writes are enabled.
    fn markdown_instructions(&self) -> String {
        let selection = &self.0.selection;
        let writes = self.writable(&[Kind::WorkItem]);
        let offered: Vec<(String, bool)> = MARKDOWN
            .iter()
            .filter(|(_, write)| writes || !write)
            .filter_map(|(name, write)| {
                let entry = find(name).filter(|e| !selection.is_disabled(e))?;
                let how = if selection.tools.iter().any(|t| t.full_name() == *name) {
                    format!("`{}`", config::tool_name(entry))
                } else if selection.lists("polarion_call") {
                    format!("`{name}` (through `polarion_call`)")
                } else {
                    return None;
                };
                Some((how, *write))
            })
            .collect();
        if offered.is_empty() {
            return String::new();
        }
        let names: Vec<&str> = offered.iter().map(|(how, _)| how.as_str()).collect();
        let reads = offered.iter().any(|(_, write)| !write);
        let (verb, diagrams) = match (reads, offered.len() > usize::from(reads)) {
            (_, true) => (
                if reads { "read and written" } else { "written" },
                "Mermaid code blocks become SVG attachments shown as images and come back as \
                 code blocks; formatting Markdown cannot express comes back as inline HTML, \
                 which writing the Markdown back keeps.",
            ),
            _ => (
                "read",
                "Diagrams come back as mermaid code blocks; formatting Markdown cannot express \
                 comes back as inline HTML.",
            ),
        };
        format!(
            " Work item descriptions can be {verb} as Markdown: {}. {diagrams}",
            names.join(", ")
        )
    }

    /// `polarion_guide`, if it is listed, and which commands need their guide first.
    fn guide_instructions(&self) -> String {
        if !self.0.selection.lists("polarion_guide") {
            return String::new();
        }
        let areas: Vec<String> = Guide::ALL
            .iter()
            .map(|g| format!("`{}`", g.name()))
            .collect();
        let areas = areas.join(", ");
        let refused = match self.0.selection.require_guides {
            RequireGuides::All => Some("every command of an area is"),
            RequireGuides::Write if self.writes() => Some("the writes of an area are"),
            _ => None,
        };
        let Some(refused) = refused else {
            return format!(
                "`polarion_guide(area)` explains each area of this server ({areas}) and lists the \
                 project's work item types and statuses; read it before working in an area you do \
                 not know. "
            );
        };
        let by = if self.0.transport == Transport::Http {
            " on this server (by any of its clients)"
        } else {
            ""
        };
        format!(
            "Read `polarion_guide(area)` before using an area's tools ({areas}): {refused} \
             refused until its guide was read{by}. Work item commands need `work-items`; \
             documents, tests, plans and wiki their own guide; every other area `overview`. "
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
        let command = command.parse(arguments)?;
        let polarion = self.0.polarion.as_ref().map_err(|e| CommandError {
            code: e.code,
            message: e.message.clone(),
        })?;
        let cx = Context::new(polarion, self.0.config.project.as_deref());
        let busy = CPU_BOUND.contains(&command.name());
        let output = match tokio::runtime::Handle::try_current() {
            // The thread is busy for a while: the runtime moves its other tasks to another one.
            Ok(runtime) if busy && runtime.runtime_flavor() == RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(|| runtime.block_on(command.run(&cx)))?
            }
            _ => command.run(&cx).await?,
        };
        Ok(serde_json::to_value(&output).expect("results serialize"))
    }

    /// Run a command for the agent: exchange arguments in, byte results as image/text blocks.
    /// `structured`: also return the result as structured content (tools with an output schema).
    async fn execute(&self, command: &Entry, arguments: Value, structured: bool) -> CallToolResult {
        let exchange = self.0.exchange.as_ref();
        let outcome = async {
            self.admit(command)?;
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

    /// `polarion_guide`: the guide of `area`, with the section read from the project (the
    /// `project` argument or the default project). Marks the guide as read.
    async fn guide(&self, arguments: &Value) -> Result<String, CommandError> {
        if let Some(unknown) = arguments.as_object().and_then(|o| {
            o.keys()
                .find(|k| !["area", "project"].contains(&k.as_str()))
        }) {
            return Err(CommandError::invalid(format!(
                "unknown argument `{unknown}`"
            )));
        }
        let area = arguments
            .get("area")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let guide = Guide::parse(area).ok_or_else(|| {
            let names: Vec<&str> = Guide::ALL.iter().map(|g| g.name()).collect();
            CommandError::invalid(format!(
                "unknown area `{area}`: one of {}",
                names.join(", ")
            ))
        })?;
        let project = match arguments.get("project") {
            None | Some(Value::Null) => None,
            Some(Value::String(p)) => Some(p.trim()).filter(|p| !p.is_empty()),
            Some(_) => return Err(CommandError::invalid("`project` must be a string")),
        };
        self.0.read.mark(guide);
        let project = project.or(self.0.config.project.as_deref());
        let section = guides::project_section(self.0.polarion.as_ref(), project, guide).await;
        Ok(format!("{}{section}", guide.text().trim_end()))
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
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_resources()
                .enable_tools()
                .build(),
        )
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

    /// The static guides (without the project section).
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult::with_all_items(
            Guide::ALL
                .iter()
                .map(|g| {
                    Resource::new(g.uri(), g.name())
                        .with_title(format!("Polarion guide: {}", g.name()))
                        .with_description(g.about())
                        .with_mime_type(MARKDOWN_TYPE)
                        .with_size(g.text().len() as u64)
                })
                .collect(),
        ))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let guide = request
            .uri
            .strip_prefix(guides::URI_PREFIX)
            .and_then(Guide::parse)
            .ok_or_else(|| {
                McpError::resource_not_found(format!("unknown resource `{}`", request.uri), None)
            })?;
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(guide.text(), guide.uri()).with_mime_type(MARKDOWN_TYPE),
        ])
        .into())
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
            ToolKind::Guide => {
                return Ok(match self.guide(&arguments).await {
                    Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
                    Err(e) => error_result(&e),
                }
                .into());
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_tools_change_objects_of_their_area() {
        use Kind::*;
        for (_, command) in config::CURATED_WRITES {
            let expected: &[Kind] = match *command {
                "tests.set-steps" => &[WorkItem],
                "tests.add-record" => &[TestRun, WorkItem],
                "plans.add-items" | "plans.remove-items" => &[Plan, WorkItem],
                _ => match command.split_once('.').unwrap().0 {
                    "items" => &[WorkItem],
                    "documents" => &[Document],
                    "tests" => &[TestRun],
                    "plans" => &[Plan],
                    other => panic!("no write area `{other}`"),
                },
            };
            assert_eq!(Kind::needed(command), expected, "{command}");
            let area = command.split_once('.').unwrap().0;
            assert!(WRITE_AREAS.iter().any(|(a, ..)| *a == area), "{command}");
        }
    }

    #[test]
    fn targets_are_taken_by_kind() {
        let base = "subterra:data-service:objects:/default/";
        let kind = |entry: &str| Kind::of_target(&WriteTarget::parse(entry, Some("P")).unwrap());
        assert_eq!(kind("P-1"), Some(Kind::WorkItem));
        assert_eq!(kind("doc:Specs/SRS"), Some(Kind::Document));
        assert_eq!(kind("run:R1"), Some(Kind::TestRun));
        assert_eq!(kind(&format!("{base}P${{TestRun}}R1")), Some(Kind::TestRun));
        assert_eq!(kind(&format!("{base}P${{Plan}}rel1")), Some(Kind::Plan));
        assert_eq!(kind(&format!("{base}P${{Comment}}{{WorkItem}}P-1#1")), None);
        assert_eq!(kind(&format!("{base}P${{Baseline}}b1")), None);
    }

    #[test]
    fn lists_are_joined_with_and() {
        let parts = |p: &[&str]| p.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(join_and(&parts(&[])), "");
        assert_eq!(join_and(&parts(&["a"])), "a");
        assert_eq!(join_and(&parts(&["a", "b"])), "a and b");
        assert_eq!(join_and(&parts(&["a", "b", "c"])), "a, b and c");
    }
}
