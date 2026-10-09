# Front ends (`polarion-cli`, `polarion-mcp`)

Both binaries are thin. They turn their input into an `AnyCommand`, connect with the settings from the environment
(`polarion_commands::Config`), run the command, and present the serialized result. Neither contains Polarion logic:
the command tree and the tools come from the catalog (see [Commands](commands.md)).

## CLI

```mermaid
flowchart LR
    argv["argv"] --> clap["Opts::parse()<br/>cli.rs"]
    clap -- "polarion commands AREA" --> list["list: AREAS + CATALOG<br/>--json adds input and output schemas"]
    clap -- "polarion AREA COMMAND ..." --> cmd["Top::Run(Cli) → AnyCommand::from"]
    cmd --> connect["Global::config() → Config::connect()<br/>env, overridden by options"]
    connect --> run["AnyCommand::run(Context)"]
    run --> logout["Polarion::logout()<br/>every run ends its session"]
    logout --> value["serde_json::to_value(AnyOutput)"]
    value -- "--json" --> json["render::json<br/>pretty on a terminal, one line when piped"]
    value -- "default" --> text["render::text(value, Entry::view)"]
    value -- "View::Bytes with --out" --> file["write the decoded bytes"]
```

- **The command tree:** [`cli.rs`](../crates/polarion-cli/src/cli.rs) has two top-level variants. `Top::Commands`
  lists commands, and `Top::Run` flattens the generated `polarion_commands::Cli` tree into the top level, so
  `polarion items get MP-42` is the `ItemsCli::Get` variant. Global options (`--url`, `-p`, `--json`, `--out`,
  `--write-projects`, ...) are in `Global` and override the environment.
- **Output:** JSON output is the library's serde form, unchanged. Text output in
  [`render.rs`](../crates/polarion-cli/src/render.rs) follows the command's `View`:
  - `Table(columns)`: a table with the given columns
  - `Object`: an indented `key: value` tree
  - `Lines`, `Links`, `Scalar`: one value per line, outgoing and incoming links, a single value
  - `Bytes`: a size summary, or the file written with `--out`
  - `Done`: nothing, or whether something changed

  Rich text is shown with its HTML tags stripped.
- **Errors:** errors go to stderr, as `{"error": {"code", "message"}}` with `--json`. The exit code comes from
  `CommandError::exit_code`: 1 error, 2 usage error, 3 not found.

## MCP server

### Startup

```mermaid
flowchart LR
    main["main.rs"] --> args["--http, --config<br/>or POLARION_MCP_HTTP, POLARION_MCP_CONFIG"]
    args --> cfg["Config::from_env()<br/>connection and write settings"]
    cfg --> sel["Selection::load()<br/>config.rs: which tools"]
    sel --> ex["Exchange::from_env()<br/>files.rs: POLARION_MCP_FILES"]
    ex --> new["PolarionMcp::new<br/>tools built once, Polarion client created<br/>(logs in on the first call)"]
    new -- "default" --> stdio["stdio::run<br/>rmcp over stdin and stdout"]
    new -- "--http ADDR" --> http["http::run<br/>Streamable HTTP at /mcp"]
    sel -. "bad config" .-> exit(["exit 1 with a message"])
    ex -. "not a directory" .-> exit
```

`PolarionMcp` is an rmcp `ServerHandler` whose tools are built at run time from the catalog (no `#[tool]` macros),
in [`server.rs`](../crates/polarion-mcp/src/server.rs). Its state (tools, client, settings) sits behind an `Arc`,
so the clones rmcp makes, one per HTTP request, share it.

### Which tools exist

[`config.rs`](../crates/polarion-mcp/src/config.rs) resolves the optional TOML file and `POLARION_MCP_TOOLS` into
a `Selection`:

```mermaid
flowchart TD
    file["config file<br/>tools, disable, output_schemas"] --> load["Selection::load"]
    env["POLARION_MCP_TOOLS<br/>replaces the file's tools"] --> load
    load --> t["tools selectors<br/>default: default"]
    load --> d["disable selectors"]
    load --> o["output_schemas<br/>default: true"]
    t & d --> select["select() per selector:<br/>default → the CURATED commands<br/>all → every command<br/>with a dot → command names, e.g. items.*<br/>otherwise → tool names, e.g. get_work_item"]
    select -- "a selector matches nothing" --> fail(["startup error"])
    select --> sel["Selection<br/>tools: selected minus disabled<br/>disabled: command names<br/>generic: the generic tools not disabled"]
    sel --> list["tools/list order:<br/>1. curated tools, in CURATED order<br/>2. polarion_commands, polarion_describe, polarion_call<br/>3. the other selected commands, in catalog order<br/>SESSION_BOUND commands left out over HTTP"]
    sel --> refuse["disabled commands: refused by<br/>polarion_call and polarion_describe,<br/>not listed by polarion_commands"]
```

- **Tool names:** each command gets one tool. The 15 everyday reads in `CURATED` keep their names
  (`get_work_item`); every other command is `area_command` (`items_add_link`). A selector matches a command by
  either name.
- **Disabled means disabled:** a disabled command is gone from every tool, so `disable` works as a deny list, not
  just a shorter tool list.

### Calling a tool

```mermaid
flowchart TD
    req["tools/call name, arguments"] --> kind{"which tool?"}
    kind -- "unknown name" --> bad(["JSON-RPC error: unknown tool"])
    kind -- "a command tool" --> exec["execute(command, args, structured = output_schemas)"]
    kind -- "polarion_call" --> look["command(name):<br/>unknown or disabled → tool error"]
    look --> exec2["execute(command, args, structured = false)"]
    kind -- "polarion_describe" --> look2["command(name)"] --> desc["description, write flag,<br/>input and output schema"]
    kind -- "polarion_commands" --> lc["areas, or one area's<br/>enabled commands"]
    exec & exec2 --> prep["files::prepare<br/>data-file → base64, take out save_as"]
    prep --> run["run(): SESSION_BOUND over HTTP → refused<br/>Entry::parse → AnyCommand::run(Context)"]
    run --> v{"View::Bytes?"}
    v -- yes --> present["files::present:<br/>image block, text block,<br/>saved file, or size and type"]
    v -- no --> textres["text block: compact JSON<br/>+ structuredContent when structured"]
    run -- "error" --> err(["tool error: error (code): message"])
```

- **Errors:** failures of a command, including refused writes, are tool results with `isError`, so the agent can
  read and react to them. Only an unknown tool name is a protocol error.
- **Files:** bytes never reach the agent as base64. [`files.rs`](../crates/polarion-mcp/src/files.rs) sniffs the
  type from the first bytes: images up to 4 MB become image blocks, UTF-8 text up to 100 KB a text block,
  anything else a size-and-type summary. With an exchange folder (`POLARION_MCP_FILES`), `save_as` writes the
  file there, and every byte argument `x` gets an `x-file` alternative that reads one. Only plain file names are
  accepted: no paths, no `..`, no symbolic links.

### Output schemas

Command tools declare an `outputSchema` from `Entry::output_schema()`, unless the command returns nothing
(`View::Done`) or bytes (`View::Bytes`), or `output_schemas = false`:

```mermaid
flowchart LR
    os["Entry::output_schema()"] --> obj{"root type: object?"}
    obj -- yes --> asis["outputSchema as is<br/>structuredContent = the result"]
    obj -- "no: list, number, flag, optional" --> wrap["outputSchema = object with one property result<br/>$defs moved to the wrapper's root<br/>structuredContent = {result: value}"]
```

MCP 2025-06-18 and 2025-11-25 require an object at the root of `outputSchema` and of `structuredContent`, and
clients built for them reject anything else. So non-object results are wrapped for every protocol version.
`polarion_call` returns text only, since its result type depends on the command; `polarion_describe` shows the raw
output schema instead.

### HTTP

```mermaid
flowchart LR
    client["MCP client"] -- "POST /mcp<br/>Authorization: Bearer token" --> router["axum Router"]
    router --> auth{"authorize middleware<br/>constant_time_eq"}
    auth -- "missing or wrong" --> r401(["401, WWW-Authenticate: Bearer"])
    auth -- ok --> svc["rmcp StreamableHttpService<br/>Host and Origin validation,<br/>32 MiB body limit,<br/>stateless, JSON replies"]
    svc --> h["PolarionMcp clone<br/>shared tools and client"]
    h --> pol[("one Polarion session<br/>for all clients")]
```

- **Access:** [`http.rs`](../crates/polarion-mcp/src/http.rs) puts a bearer-token check
  (`POLARION_MCP_AUTH_TOKEN`, at least 16 characters, compared in constant time) in front of rmcp's service.
  rmcp checks `Host` against `POLARION_MCP_ALLOWED_HOSTS` (default: localhost names) and refuses browser `Origin`s
  that are not in `POLARION_MCP_ALLOWED_ORIGINS`, which protects against DNS rebinding.
- **Stateless:** no `Mcp-Session-Id` and no server-initiated streams; every POST gets a JSON reply. That keeps the
  server free of per-client state.
- **One identity:** all clients act as the server's Polarion account and share its session. Transactions would
  mix clients' changes, so `SESSION_BOUND` commands are left out of the tool list and refused by `run()` over
  HTTP.
- **Shutdown:** Ctrl-C or SIGTERM cancels rmcp's token, axum shuts down gracefully, and `PolarionMcp::shutdown`
  ends the Polarion session. Over stdio, the session ends when stdin closes.
