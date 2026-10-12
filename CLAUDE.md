# Polarion-rs

Rust workspace: `crates/polarion-rest` (generated REST client), `crates/polarion-soap` (generated SOAP client),
`crates/polarion` (hand-written library on top of `polarion-soap`), `crates/polarion-markdown` (pure Markdown ↔
Polarion rich text conversion, mermaid diagrams as SVG), `crates/polarion-commands` (every library method
as a typed command DTO; macros in `crates/polarion-commands-derive`), `crates/polarion-cli` (`polarion` binary) and `crates/polarion-mcp` (`polarion-mcp` MCP server),
both built from the command catalog, `xtask` (code generators).
The repository is English only: code, comments, docs and commit messages.

- Never edit `crates/*/src/generated/*` or `crates/*/tests/spec_examples.rs` by hand. Change the generator
  (`xtask/src/` for REST, `xtask/src/soap/` for SOAP) and run `cargo xtask codegen`.
- Hand-written REST runtime: `client.rs` (HTTP plumbing, pagination), `jsonapi.rs` (envelope types), `query.rs`, `error.rs`.
- Hand-written SOAP runtime: `client.rs` (envelope, session incl. renewal after expiry, faults, read-only allowlist,
  write filter; after a renewal a write is repeated only after a SOAP fault and through the filter again), `xml.rs` (XML helpers), `error.rs`.
  WSDL quirks are handled in `xtask/src/soap/emit.rs`: params documented as accepting null become `Option`
  (`nullable_params`), plus `NILLABLE_PARAMS`/`OPTIONAL_PARAMS` overrides; `READ_PREFIXES`/`OTHER_READS` decide what
  read-only clients allow; subtype elements (e.g. `EnumCustomFieldType.enumId`) are decoded into the base struct.
- JSON Schemas of results (feature `schemars` in `polarion-soap` and `polarion`) must match what `Serialize`
  writes: the SOAP generator emits them next to the `Serialize` impls; library types derive them; a type with a
  hand-written `Serialize` (`ItemRef`, `DocumentRef`, `Download`) needs a hand-written schema
  (`polarion-commands/tests/output_schemas.rs` validates real values against them).
- `polarion`: one handle per area (`items.rs`, `documents.rs`, `schema.rs`, `tests_mgmt.rs`, `plans.rs`, `wiki.rs`,
  `baselines.rs`, `revisions.rs`, `activities.rs`, `folders.rs`, `projects.rs`, `security.rs`, `builds.rs`), domain types in
  `model.rs` (serde, camelCase, empty fields skipped), write inputs and JSON coercion in `write.rs`, SOAP ↔ domain
  mappings in `convert.rs`, URI parsing/building in `uri.rs`, restricted mode's SOAP write filter in `restricted.rs`.
  `Polarion::new` is read-only; keep it that way.
  **Every method that modifies data must call a write guard before sending anything:** a target guard
  (`check_write_target` for the URI that is sent, `check_write_targets` when it names further objects,
  `check_write_owned` for comments and work records via `uri::owner_of`, `check_create` for creates), or
  `check_unbounded_write` followed by the project check (`check_write`, `check_write_uri`, `check_global_write`,
  `check_write_scope`); field writes also `check_field_keys`. Restricted mode (`restrict_writes`/`allow_creates`)
  is fail-closed: the project checks (and `check_any_write`) always refuse there, `check_unbounded_write` refuses,
  and `build()` installs the SOAP write filter (`restricted::Filter`, `ClientBuilder::write_filter`), which also
  covers `Polarion::soap()` (tests in `tests/items_write.rs`, `tests/areas.rs`, `tests/write_targets.rs`).
  A new SOAP write operation must be classified in `tests/write_targets.rs`: allowed (in `OPERATIONS` in
  `restricted.rs` and in `ALLOWED`, same class) or `NEVER` with a reason; `every_write_operation_is_classified` fails
  otherwise.
  Markdown descriptions (`markdown.rs`, feature `markdown`): the guard comes first; every new diagram is rendered, then
  uploaded, before `updateWorkItem`; pruning deletes only this work item's diagram attachments the old description showed.
  `tests/coverage.rs` fails when a SOAP operation is neither called by the library nor listed in its `EXCLUDED` with a
  reason; new operations from a newer WSDL must be exposed or excluded there. Writes are verified against wiremock only; keep the
  "Not yet verified" list in README.md current.
- `polarion-commands`: one module per area (mirroring the library handles); every command is a DTO struct with
  `#[cmd(write?, view = ...)]` and a hand-written `impl Run` (`type Output` is the typed result; it must implement
  `JsonSchema` describing its serialized form, which becomes the MCP output schema). The name comes
  from the type name (kebab-case), the summary and help from doc comments; `#[positional]` and `#[flatten]` mark
  fields, a field named `project` is the default-project override. Every DTO must be listed in `registry!` in
  `lib.rs`, which generates `AnyCommand`/`AnyOutput`, the catalog (`Entry`) and the clap tree (feature `cli`).
  Typed argument types (`ItemArg`, `DocumentArg`, `RichText`, `TextArg`, `FieldSelection`, `Data`, `QueryArgs`, ...) are in
  `arg.rs`; fixed value sets become enums. Only runtime-typed values (custom fields, test parameters) stay JSON
  (`JsonObject`/`JsonValue`, typed in `values.rs`). JSON parsing (unknown keys refused, lenient scalars) is
  `parse_as` in `command.rs`. The macros live in `crates/polarion-commands-derive`. `tests/coverage.rs` fails when a
  public library method has no command: **a new library method needs a command**. Write commands set `write` and
  rely on the library's guard.
- `polarion-cli` and `polarion-mcp` contain no Polarion logic: the clap tree (`cli.rs`) and the MCP tools
  (`server.rs`) are generated from the catalog. CLI text output in `render.rs` (table columns come from the
  command's `View`); JSON output is the library's serde form, don't reshape it. The MCP server is an
  `rmcp` `ServerHandler` (`server.rs`) whose tools are built at runtime from the catalog (no `#[tool]` macros),
  served over stdio (`stdio.rs`) or rmcp's Streamable HTTP service behind a bearer-token layer (`http.rs`:
  stateless, JSON replies, rmcp's Host/Origin checks); which tools are offered is `config.rs` (`--config`/`POLARION_MCP_CONFIG`
  TOML file plus `POLARION_MCP_TOOLS`: `tools` and `disable` selectors, `output_schemas`; its `CURATED` table holds
  the curated read tools and `CURATED_WRITES` the curated write tools (both grouped by area), which are listed only when
  `Config::writes_enabled()` (read-only, they get no tool at all; the instructions name those
  the write scope cannot reach, `tool_writable` in `server.rs`); a command with a curated tool never gets an
  `area_command` one; a disabled command is refused by every tool, `polarion_call` included). Per-area guides for
  agents are Markdown in `crates/polarion-mcp/guides/` (embedded by `guides.rs`; only facts this repository backs):
  `polarion_guide` returns one plus a section read from the project, and they are MCP resources
  (`polarion://guide/<area>`). The gate `require_guides` (config file, `POLARION_MCP_REQUIRE_GUIDES`; `write` by
  default, `all`, `none`) refuses an area's writes (or all its commands) with `guideRequired` until its guide was read
  in this process (`admit()` in `server.rs`, before anything is sent, on top of the library's write guard). Byte results
  are shown as image/text blocks or saved to the opt-in exchange folder (`files.rs`, `POLARION_MCP_FILES`:
  `save_as`, `<param>-file`, plain file names only). Over HTTP all clients share one Polarion
  session, so commands that need a session of their own (transactions, `SESSION_BOUND`) are refused there. End-to-end tests (`tests/cli.rs`,
  `tests/mcp.rs`) spawn the binaries against wiremock.
- `polarion-markdown`: both directions stay inverse. A change in `emit/` (Markdown → HTML) needs the matching rule in
  `reverse/` (HTML → Markdown) and updated fixtures and golden files (`UPDATE_GOLDEN=1`); `docs/markdown.md` is the spec.
  Hand-built comrak trees must pass `validate()`: never an empty `Code` (the writer panics), missing escapes go in as
  `Raw` (a hand-built `Escaped` writes nothing). A change to the default theme or the SVG embedding needs a new
  `mermaid::PROFILE` (it is part of every diagram's hash and attachment name).
- New resource types in the spec need a Rust name in `xtask/src/names.rs` (codegen fails otherwise).
- Architecture docs are in `docs/` (Markdown with Mermaid diagrams; README.md covers usage). The diagrams name real
  functions, types and files: when you change a mechanism a page shows, update the page in the same change.
- Releases: a semver tag runs `.github/workflows/release.yml` (`docs/releasing.md`): the GitHub release (an existing
  one is only added to), `polarion` and `polarion-mcp` for every target, then `cargo publish` per crate. A new
  published crate must be added to the publish loop there (dependencies first); its version is the workspace's.
- Before pushing: `cargo xtask codegen --check && cargo fmt --all --check && cargo clippy --workspace --all-targets && cargo test --workspace && cargo test -p polarion-markdown --no-default-features && cargo check -p polarion` (CI also runs the last two: the conversion without mermaid and highlighting, the library without markdown).
- Live tests: `crates/polarion-rest/tests/live.rs` only ever targets testdrive.polarion.com (`POLARION_TESTDRIVE_TOKEN`,
  `POLARION_TESTDRIVE_PROJECT`). Any other server, production included, is written to only by the live suite
  (`crates/polarion/tests/live.rs`, the CLI's and the MCP server's `tests/live.rs`, the bootstrap
  `crates/polarion/examples/live_fixtures.rs`, sharing `crates/polarion/tests/live_support.rs`), only to its fixtures,
  through a client in restricted mode (`restrict_writes` with the fixtures' URIs), after its gate (`docs/testing.md`);
  never point anything else that writes at such a server. The tiers are `#[ignore]` and the bootstrap is an example,
  so `cargo test` never contacts a server.
