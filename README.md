# Polarion-rs

Rust tooling for [Siemens Polarion ALM](https://polarion.plm.automation.siemens.com/).

| Crate | Status | Purpose |
|---|---|---|
| [`polarion-rest`](crates/polarion-rest) | usable | Low-level async client covering the whole Polarion REST API (v1), generated from the OpenAPI spec |
| `polarion` | planned | Ergonomic library on top of `polarion-rest` |
| `polarion-cli` | planned | Command-line interface, with JSON output for automation |
| `polarion-mcp` | planned | MCP server so AI agents can work with Polarion |

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

- **Coverage:** all 284 operations of the spec, including multipart uploads (`FilePart`) and binary downloads (`Vec<u8>`).
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

Try it against a real server (read-only, at most six small `GET` requests; each step reports OK or FAIL independently):

```sh
POLARION_URL=https://polarion.example.com/polarion POLARION_TOKEN=... POLARION_PROJECT=MyProject POLARION_SAVE_SPEC=spec.json \
  cargo run -p polarion-rest --example smoke
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
`spec/polarion-openapi.json` by the generator in [`xtask/`](xtask). The generated code is committed; CI fails if it
is stale.

```sh
cargo xtask codegen          # regenerate
cargo xtask codegen --check  # verify it is up to date
```

To target another Polarion version, replace the spec with the one your server serves at
`<polarion>/rest/v1/definition` and regenerate. The committed spec was fetched from
`https://testdrive.polarion.com/polarion/rest/v1/definition` (Apache-2.0).

`spec_examples.rs` builds an example document from every schema in the spec and checks it round-trips through the
generated types, so mapping gaps show up as test failures rather than silently dropped fields.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
