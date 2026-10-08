# Polarion-rs

Rust tooling for [Siemens Polarion ALM](https://polarion.plm.automation.siemens.com/).

| Crate | Status | Purpose |
|---|---|---|
| [`polarion-rest`](crates/polarion-rest) | usable | Low-level async client covering the whole Polarion REST API (v1), generated from the OpenAPI spec |
| [`polarion-soap`](crates/polarion-soap) | usable | Async client for the Polarion SOAP web services, generated from the WSDLs |
| [`polarion`](crates/polarion) | usable (writes mock-tested only) | Convenient, JSON-serializable API over the whole SOAP API, by area, on top of `polarion-soap` |
| [`polarion-commands`](crates/polarion-commands) | usable | Every library method as a named JSON command with a parameter schema; shared by the CLI and the MCP server |
| [`polarion-cli`](crates/polarion-cli) | usable (writes mock-tested only) | `polarion` command-line tool covering the whole library, with JSON output for automation |
| [`polarion-mcp`](crates/polarion-mcp) | usable (writes mock-tested only) | MCP server (stdio) so AI agents can work with Polarion |

Both front ends are **read-only unless writes are enabled** for named projects (see [Writes](#writes)).

## polarion (command line)

```sh
cargo install --path crates/polarion-cli    # installs the `polarion` binary
```

Configuration comes from the environment (credentials only from there, never from flags):

```sh
export POLARION_URL=https://polarion.example.com/polarion
export POLARION_TOKEN=<personal access token>   # or POLARION_USER + POLARION_PASSWORD
export POLARION_PROJECT=MyProject               # default project, or -p/--project
```

```powershell
$env:POLARION_URL = "https://polarion.example.com/polarion"
$env:POLARION_TOKEN = "<personal access token>"
$env:POLARION_PROJECT = "MyProject"
```

Commands are `polarion <area> <command>`, one per library method (about 250). `polarion commands [AREA]` lists them
and `--help` on any command shows its arguments.

```sh
polarion commands items
polarion projects get
polarion items query "type:requirement AND status:open" --sort ~updated --limit 20
polarion items query "type:requirement" --fields id,title,description --custom asil --json
polarion items count "type:requirement"
polarion items get MP-42                    # or OtherProject/OP-7, or a URI
polarion items links MP-42                  # outgoing and incoming
polarion items history MP-42
polarion documents list [--space Specs]
polarion documents get Specs/SRS [--body]
polarion documents work-items Specs/SRS
polarion schema custom-fields --type requirement
polarion tests search "status:open"
polarion plans statistics Release-1
polarion users get jdoe
polarion items attachment MP-42 --attachment 1-spec.pdf --out spec.pdf
```

- **Read-only by default:** commands marked `write` in `polarion commands` are refused before anything is sent,
  unless `POLARION_WRITE_PROJECTS` (or `--write-projects`) names the target project. Writes that are not tied to a
  project (users, global folders) additionally need `POLARION_ALLOW_GLOBAL_WRITES=1`. Each run ends its server
  session.
- **Arguments:** work items are `ID` (in the default project), `PROJECT/ID` or a URI; documents `SPACE/NAME`,
  `PROJECT/SPACE/NAME` or a URI. Structured arguments (custom fields, test records) take JSON, e.g.
  `--custom-fields '{"asil": "D"}'`; rich text takes a string or `{"type": "text/html", "content": "..."}`; binary
  arguments take base64 or `@FILE`, binary results go to `--out FILE`.
- **Text output** for people: tables for lists, an indented `key: value` view for single objects, rich text with
  HTML tags stripped.
- **`--json`** prints the library's JSON form (pretty on a terminal, one line when piped). Errors go to stderr, in
  JSON mode as `{"error": {"code": "notFound", "message": "..."}}`. `polarion --json commands` includes the JSON
  Schema of every command's arguments.
- **Exit codes:** 0 success, 1 error, 2 usage error, 3 not found.
- **Limits:** queries return at most 100 results unless `--limit N` or `--all`; a note on stderr says when results
  were cut off.

## polarion-mcp (MCP server)

```sh
cargo install --path crates/polarion-mcp    # installs the `polarion-mcp` binary
```

An [MCP](https://modelcontextprotocol.io) server over stdio, configured by the same environment variables as the
CLI. It offers:

- **Dedicated tools for everyday reads:** `get_work_item`, `query_work_items`, `count_work_items`,
  `work_item_links`, `work_item_history`, `list_documents`, `get_document`, `document_work_items`, `custom_fields`,
  `field_options`, `search_test_runs`, `get_test_run`, `search_plans`, `get_project`, `get_user`.
- **Three generic tools that reach every command:** `polarion_commands` (areas, or the commands of one area),
  `polarion_describe` (a command's argument schema) and `polarion_call` (run a command).
- With `POLARION_MCP_TOOLS=all`, every command is also registered as its own tool (`items_add_link`, ...). That
  is about 270 tools, which costs the agent context on every turn; the default set is usually better.

Results are the library's JSON. Failures, including refused writes, are tool errors with a message such as
`error (readOnly): ...`. The server is read-only unless `POLARION_WRITE_PROJECTS` is set, and it tells the agent so
when it connects.

Claude Code:

```sh
claude mcp add polarion -e POLARION_URL=https://polarion.example.com/polarion \
  -e POLARION_TOKEN=<token> -e POLARION_PROJECT=MyProject -- polarion-mcp
```

Claude Desktop and other clients (`claude_desktop_config.json`, `.mcp.json`):

```json
{
  "mcpServers": {
    "polarion": {
      "command": "polarion-mcp",
      "env": {
        "POLARION_URL": "https://polarion.example.com/polarion",
        "POLARION_TOKEN": "<token>",
        "POLARION_PROJECT": "MyProject"
      }
    }
  }
}
```

## polarion-commands

The catalog both front ends are built from: every library method as a command with a name (`items.add-link`), a
summary, typed parameters (exported as JSON Schema), a write flag, and a handler that takes JSON arguments and
returns the library's JSON. `polarion_commands::Config::from_env()` reads the environment variables above.
`tests/coverage.rs` fails when a library method has no command.

```rust
let config = polarion_commands::Config::from_env();
let polarion = config.connect()?;
let args = polarion_commands::Args::from_value(serde_json::json!({"item": "MP-42"}), config.project.clone())?;
let item = polarion_commands::call(&polarion, "items.get", &args).await?;
```

## polarion

The library the CLI and MCP server build on. It uses the SOAP services (so it works with ordinary user permissions)
and returns flat domain types that serialize to compact JSON: enum options, users and linked work items are reduced
to their ids, custom fields become a `key → value` map, and empty fields are left out.

Operations are grouped by area and cover the **whole SOAP API**: every one of the 302 operations is reachable
through a library method (variants such as `…WithFields`, `…Limited`, `…InBaseline` or `…BySQL` are options of one
method). `tests/coverage.rs` enforces this; the few operations it excludes are listed there with the reason
(login/logout, deprecated duplicates).

| Handle | What it covers |
|---|---|
| `items()` | Work items: get (by id/URI, fields, revision), Lucene queries (limit, baseline, revision), SQL, count, back links, history, revisions, workflow actions and transitions, field options, custom fields, attachments, localized texts; create, update, delete, move, comments (incl. restricted visibility), links of every kind, assignees, categories, approvals, attachments, work records, planning constraints |
| `documents()` | Spaces, list, get (with or without body), Lucene/SQL queries (baseline), work items in document order; create, update, delete, comments, reuse and derived documents, title heading, workflow, PDF export |
| `schema()` | Custom field definitions and keys, enum options (by field, enum id, control value, object), control keys, initial workflow actions, categories, time points, languages, duration settings |
| `tests()` | Test runs and templates (get, search), records, steps, step configuration, test configuration, parameters, custom field definitions, report page; create, update, records, execute, steps, attachments, summary defects, project span |
| `plans()` | Plans and templates (get, search), statistics, report page; create, update, delete, plan items, allowed types, project span |
| `wiki()` | Spaces, pages, Lucene/SQL queries (baseline) |
| `baselines()`, `revisions()` | Baselines (query, create, update, delete); revisions (get, query, linked work items) |
| `activities()` | Activity streams (all, project, group), sources, comments |
| `folders()` | Document & page folders, project or global |
| `projects()`, `users()` | Projects and project groups; users (get, list, avatar, create, update) |
| `security()` | Permission checks, roles, location of objects, token owner, licenses, account vault |
| `Polarion` | Login/logout, session check, transactions (`begin_transaction`, `commit`, `rollback`) |

Results are flat domain types (`WorkItem`, `Document`, `TestRun`, `Plan`, ...). Configuration-like results
(test configuration, licenses, builds, activities, folders, ...) are the generated SOAP types, which implement
`serde::Serialize` with the `serde` feature of `polarion-soap` (enabled by this crate).

```rust
use polarion::{Auth, Fields, ItemRef, Polarion, WorkItemQuery};

// Read-only: operations that may modify data are refused before they are sent.
let polarion = Polarion::new("https://polarion.example.com/polarion", Auth::Token("<token>".into()))?;

let query = WorkItemQuery::new("type:requirement AND status:open")
    .project("MyProject")
    .sort("~updated")
    .limit(20)
    .fields(Fields::standard().with("description").with_custom("asil"));
let open = polarion.items().query(&query).await?;
println!("{}", serde_json::to_string_pretty(&open)?);

let item = ItemRef::new("MyProject", "MP-42");
let full = polarion.items().get(&item).await?;              // all fields, outgoing links, comments, ...
let back_links = polarion.items().back_links(&item).await?;
let history = polarion.items().history(&item).await?;
let fields = polarion.schema().custom_fields("MyProject", "requirement").await?;

let docs = polarion.documents().list("MyProject", None).await?;            // all spaces, without bodies
let srs = polarion.documents().get("MyProject", "Specs", "SRS", true).await?; // with body
let items = polarion.documents().work_items(&srs.reference().unwrap(), &Fields::standard()).await?;
```

```json
{"id": "MP-42", "project": "MyProject", "type": "requirement", "title": "Brakes", "status": "open",
 "author": "jdoe", "document": {"project": "MyProject", "space": "Specs", "name": "SRS"},
 "links": [{"role": "parent", "item": {"project": "MyProject", "id": "MP-7"}}],
 "customFields": {"asil": "D", "platforms": ["a", "b"], "reviewed": "2024-01-31"}}
```

### Writes

Writes are enabled per project. Every write method checks the target's project against the allowlist and fails with
`Error::WriteNotAllowed` before anything is sent; a client built with `Polarion::new` or `Polarion::from_client`
writes nothing. Writes that are not tied to a project (creating and updating users, global folders, comments on
activities, the account vault) additionally need `.allow_global_writes(true)`. (`Polarion::soap()` is the raw
client and is not checked.)

```rust
use polarion::{Auth, FieldValue, ItemRef, NewWorkItem, Polarion, WorkItemUpdate};

let polarion = Polarion::builder("https://polarion.example.com/polarion", Auth::Token("<token>".into()))
    .allow_writes(["Sandbox"])
    .build()?;
let items = polarion.items();

let mut new = NewWorkItem::new("Sandbox", "requirement", "Brakes must work");
new.custom_fields.insert("asil".into(), FieldValue::Enum("D".into()));
let item = items.create(&new).await?;

items.update(&item, &WorkItemUpdate { title: Some("Brakes must always work".into()), ..Default::default() }).await?;
items.add_link(&item, "relates_to", &ItemRef::new("Sandbox", "SB-1")).await?;
items.transition(&item, "reviewed").await?;   // status changes go through workflow actions
```

`FieldValue::coerce(&json, &field_definition)` types JSON input (from a CLI or an agent) for a custom field using its
definition from `schema()`.

**Not yet verified against a real server.** The writes are tested against a mock server only. These assumptions
need one run against a sandbox project (the smoke example checks the first two read-only, see `CHECK` lines):

- Work item and document URIs built from ids (`ItemRef::new`, `DocumentRef::new`):
  `subterra:data-service:objects:/default/PROJECT${WorkItem}ID` and `...PROJECT${Module}{moduleFolder}SPACE/NAME`.
- Field keys accepted by `fields` selections (`moduleURI`, bare `customFields`).
- `setFieldsNull` taking a custom field's key to clear it.
- Custom field values: floats sent as `xsd:float`, currency as `xsd:decimal`, rich text as `types:Text`.
- `createWorkItem` honouring custom fields given at creation.
- Work records: the user sent by id only; test records: the executing user sent as
  `subterra:data-service:objects:/default/${User}ID`.
- Update operations for documents, test runs and plans sending only the changed fields (as `updateWorkItem` is
  documented to accept).
- Session renewal (reads too): a call with an expired session fails with a SOAP fault or HTTP error, and
  `hasSubject` then answers `false` (or a fault). The fault Polarion sends is undocumented, so the client asks
  instead of matching its text.

The raw SOAP client stays available through `Polarion::soap()`. Try it against a real server (read-only, about twenty
small requests, prints the JSON of each result):

```sh
POLARION_URL=https://polarion.example.com/polarion POLARION_TOKEN=... POLARION_PROJECT=MyProject \
  cargo run -p polarion --example smoke
```

## polarion-rest

```rust
use polarion_rest::models::{WorkItem, WorkItemAttributes};
use polarion_rest::{Client, Query, Resource, ResourceKind};

let client = Client::new("https://polarion.example.com/polarion", "<personal access token>")?;

// Every operation is a method named after its operationId: getWorkItems → get_work_items.
let page = client
    .get_work_items("MyProject", &Query::new().query("type:requirement").all_fields(WorkItemAttributes::TYPE))
    .await?;
for item in &page.data {
    let attrs = item.attributes.as_ref().unwrap();
    println!("{:?} {:?} custom: {:?}", item.id, attrs.title, attrs.extra);
}

// Create: attribute structs double as create/patch payloads (all fields optional).
let new: WorkItem = Resource::from_attributes(WorkItemAttributes {
    type_: Some("task".into()),
    title: Some("Check brakes".into()),
    ..Default::default()
});
let created = client.post_work_items("MyProject", &[new]).await?;
```

- **Coverage:** all 221 operations of the Polarion 2410 spec, including multipart uploads (`FilePart`) and binary downloads (`Vec<u8>`).
- **Types:** one attributes struct per resource type (`WorkItemAttributes`, `DocumentAttributes`, ...) shared by
  read, create and update. Custom fields land in `extra`.
- **Query parameters:** `Query` builder for paging, sparse fieldsets (`fields[workitems]=@all`), `include`, Lucene `query`,
  `sort`, `revision`, and any other parameter via `Query::param`.
- **Pagination:** `Client::paginate` fetches all pages of any list operation.
- **Escape hatch:** `Client::request_json` for raw calls.
- **Read-only mode:** `Client::builder(..).read_only(true)` refuses every non-`GET` request before it is sent,
  for exploring production servers safely.
- **TLS:** the OS trust store is used, so corporate CAs work; extra PEM CAs or (for test servers) disabled
  validation via `Client::builder`.

Try it against a real server (read-only, at most five small `GET` requests; each step reports OK or FAIL independently):

```sh
POLARION_URL=https://polarion.example.com/polarion POLARION_TOKEN=... POLARION_PROJECT=MyProject POLARION_SAVE_SPEC=spec.json \
  cargo run -p polarion-rest --example rest_smoke
```

## polarion-soap

Polarion's older SOAP web services are not subject to the REST API permissions (see below) and cover things the
2410 REST API lacks, such as listing documents, SQL queries, baselines and wiki pages.

```rust
use polarion_soap::types::AnyValue;
use polarion_soap::{Auth, Client};

let client = Client::builder("https://polarion.example.com/polarion", Auth::Token("<token>".into()))
    .read_only(true)
    .build()?;

// One accessor per service, one method per WSDL operation (queryWorkItemsLimited → query_work_items_limited).
let items = client
    .tracker()
    .query_work_items_limited("project.id:MyProject AND type:requirement", "id", &["id".into(), "title".into()], 10)
    .await?;
let item = client.tracker().get_work_item_by_id("MyProject", "MP-1").await?;
for field in item.custom_fields.unwrap_or_default() {
    match field.value {
        Some(AnyValue::EnumOptionId(e)) => println!("{} = enum {:?}", field.key, e.id),
        Some(AnyValue::Text(t)) => println!("{} = text {}", field.key, t.content),
        other => println!("{} = {other:?}", field.key),
    }
}
```

- **Coverage:** all 302 operations of the 7 services (Session, Project, Tracker, TestManagement, Planning, Security,
  Builder), with Javadoc from the WSDLs as method docs.
- **Session:** logs in lazily with a personal access token (`logInWithToken`) or user/password and keeps the
  session (`sessionID` header plus cookie) for all services. Concurrent calls share one login. When a call fails with
  a fault or HTTP error and `hasSubject` says the session is gone, the client logs in again and repeats the call
  once, so long-running processes (the MCP server) survive the server's session timeout. Inside an explicit
  transaction it reports `Error::SessionExpired` instead, since the transaction's changes are lost.
- **Untyped values:** custom field values and other untyped elements are `AnyValue`, decoded from the `xsi:type` the
  server sends (enum options, rich text, dates, numbers, arrays, ...). Unknown types are kept as raw XML, never
  dropped.
- **Errors:** SOAP faults surface with their code and message (`Error::Fault`).
- **Read-only mode:** `read_only(true)` refuses every operation not named `get…`, `query…`, `list…`, `search…`,
  `has…`, `can…`, `is…`, `was…` or `export…` (plus `generateHistory` and login/logout) before anything is sent.

Try it against a real server (read-only, at most seven small requests, each reported as OK or FAIL):

```sh
POLARION_URL=https://polarion.example.com/polarion POLARION_TOKEN=... POLARION_PROJECT=MyProject \
  cargo run -p polarion-soap --example soap_smoke
```

## Live tests (testdrive only)

`crates/polarion-rest/tests/live.rs` runs against Siemens' public test drive server. The URL is hard-coded so the
suite can never touch a production instance. `read_tour` is read-only; `work_item_lifecycle` creates three work
items in your Sandbox project, exercises update, bulk update, query + pagination, links, comments and attachment
upload/download, then deletes them again (also when a step fails).

```sh
export POLARION_TESTDRIVE_TOKEN=<personal access token created on testdrive.polarion.com>
export POLARION_TESTDRIVE_PROJECT=<your project in the testdrive Sandbox>
cargo test -p polarion-rest --test live -- --ignored --nocapture --test-threads=1
```

## Code generation

`crates/polarion-rest/src/generated/` and `crates/polarion-rest/tests/spec_examples.rs` are generated from
`spec/polarion-openapi.json`, `crates/polarion-soap/src/generated/` and `crates/polarion-soap/tests/spec_examples.rs`
from `spec/soap/*.wsdl`, both by the generators in [`xtask/`](xtask). The generated code is committed; CI fails if
it is stale.

```sh
cargo xtask codegen          # regenerate
cargo xtask codegen --check  # verify it is up to date
```

The committed spec is the definition served by a Polarion **2410** server (Apache-2.0). Newer releases add
endpoints (testdrive's spec has 284 operations, including Pages, licensing, jobs and `GET /user`). To target
another version, replace the spec with the one your server serves at `<polarion>/rest/v1/definition` (the `rest_smoke`
example's `POLARION_SAVE_SPEC` does this) and regenerate.

## REST permissions

Polarion guards the REST API with its own per-role permissions (REST API endpoint `GET`, `PATCH`, `POST`, `DELETE`,
configured in the Global context). By default only `admin` and `project_admin` have them, so other users get
`403 Forbidden` on every endpoint even with a valid token, including for projects they can open in the UI. An
administrator has to grant at least `GET`, e.g. to `project_user`. See
[Siemens: REST API authorization](https://developer.siemens.com/polarion/authorization.html).

The WSDLs were fetched from the same 2410 server (`<polarion>/ws/services/<Name>WebService?wsdl`, no login needed).
Unlike REST, the SOAP API barely changes between versions: compared with testdrive's newer release, all 302
operations are the same and the only type difference is `Hyperlink.title`, which 2410 lacks.

Both `spec_examples.rs` test suites build an example document from every schema (independently of the generator)
and check it round-trips through the generated types, so mapping gaps show up as test failures rather than silently
dropped fields.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
