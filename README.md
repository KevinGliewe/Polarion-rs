# Polarion-rs

Rust tooling for [Siemens Polarion ALM](https://polarion.plm.automation.siemens.com/).

| Crate | Status | Purpose |
|---|---|---|
| [`polarion-rest`](crates/polarion-rest) | usable | Low-level async client covering the whole Polarion REST API (v1), generated from the OpenAPI spec |
| [`polarion-soap`](crates/polarion-soap) | usable | Async client for the Polarion SOAP web services, generated from the WSDLs |
| [`polarion`](crates/polarion) | usable (reads) | Convenient, JSON-serializable view of projects, work items and documents, on top of `polarion-soap` |
| [`polarion-cli`](crates/polarion-cli) | usable (reads) | `polarion` command-line tool, with JSON output for automation |
| `polarion-mcp` | planned | MCP server so AI agents can work with Polarion |

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

```sh
polarion project show
polarion item query "type:requirement AND status:open" --sort ~updated --limit 20
polarion item query "type:requirement" --fields id,title,description --custom asil --json
polarion item count "type:requirement"
polarion item show MP-42                    # or OtherProject/OP-7
polarion item links MP-42                   # outgoing and incoming
polarion doc list [--space Specs]
polarion doc show Specs/SRS [--body]
polarion doc items Specs/SRS
polarion user show jdoe
```

- **Read-only:** the client refuses every operation that may modify data, and each run ends its server session.
- **Text output** for people: tables for lists, an indented `key: value` view for single objects, rich text with
  HTML tags stripped.
- **`--json`** prints the library's JSON form (pretty on a terminal, one line when piped). Errors go to stderr, in
  JSON mode as `{"error": {"code": "notFound", "message": "..."}}`.
- **Exit codes:** 0 success, 1 error, 2 usage error, 3 not found.
- **Limits:** queries return at most 100 results unless `--limit N` or `--all`; a note on stderr says when results
  were cut off.

## polarion

The library the CLI and MCP server build on. It uses the SOAP services (so it works with ordinary user permissions)
and returns flat domain types that serialize to compact JSON: enum options, users and linked work items are reduced
to their ids, custom fields become a `key → value` map, and empty fields are left out.

```rust
use polarion::{Auth, Fields, Polarion, WorkItemQuery};

// Read-only: operations that may modify data are refused before they are sent.
let polarion = Polarion::new("https://polarion.example.com/polarion", Auth::Token("<token>".into()))?;

let query = WorkItemQuery::new("type:requirement AND status:open")
    .project("MyProject")
    .sort("~updated")
    .limit(20)
    .fields(Fields::standard().with("description").with_custom("asil"));
let open = polarion.query_work_items(&query).await?;
println!("{}", serde_json::to_string_pretty(&open)?);

let item = polarion.work_item("MyProject", "MP-42").await?;   // all fields, outgoing links
let back_links = polarion.back_links(item.uri.as_deref().unwrap()).await?;

let docs = polarion.documents("MyProject", None).await?;      // all spaces, without bodies
let srs = polarion.document("MyProject", "Specs", "SRS", true).await?; // with body
let items = polarion.document_work_items(srs.uri.as_deref().unwrap(), &Fields::standard()).await?;
```

```json
{"id": "MP-42", "project": "MyProject", "type": "requirement", "title": "Brakes", "status": "open",
 "author": "jdoe", "document": {"project": "MyProject", "space": "Specs", "name": "SRS"},
 "links": [{"role": "parent", "item": {"project": "MyProject", "id": "MP-7"}}],
 "customFields": {"asil": "D", "platforms": ["a", "b"], "reviewed": "2024-01-31"}}
```

Anything not covered is reachable through `Polarion::soap()`. Try it against a real server (read-only, about a dozen
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
  session (`sessionID` header plus cookie) for all services.
- **Untyped values:** custom field values and other untyped elements are `AnyValue`, decoded from the `xsi:type` the
  server sends (enum options, rich text, dates, numbers, arrays, ...). Unknown types are kept as raw XML, never
  dropped.
- **Errors:** SOAP faults surface with their code and message (`Error::Fault`).
- **Read-only mode:** `read_only(true)` refuses every operation not named `get…`, `query…`, `list…`, `search…`,
  `has…`, `can…`, `is…`, `was…` or `export…` (plus login/logout) before anything is sent.

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
