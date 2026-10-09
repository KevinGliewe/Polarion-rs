# Polarion-rs

Rust workspace: `crates/polarion-rest` (generated REST client), `crates/polarion-soap` (generated SOAP client),
`crates/polarion` (hand-written library on top of `polarion-soap`), `crates/polarion-commands` (every library method
as a typed command DTO; macros in `crates/polarion-commands-derive`), `crates/polarion-cli` (`polarion` binary) and `crates/polarion-mcp` (`polarion-mcp` MCP server),
both built from the command catalog, `xtask` (code generators).
The repository is English only: code, comments, docs and commit messages.

- Never edit `crates/*/src/generated/*` or `crates/*/tests/spec_examples.rs` by hand. Change the generator
  (`xtask/src/` for REST, `xtask/src/soap/` for SOAP) and run `cargo xtask codegen`.
- Hand-written REST runtime: `client.rs` (HTTP plumbing, pagination), `jsonapi.rs` (envelope types), `query.rs`, `error.rs`.
- Hand-written SOAP runtime: `client.rs` (envelope, session incl. renewal after expiry, faults, read-only allowlist), `xml.rs` (XML helpers), `error.rs`.
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
  mappings in `convert.rs`, URI parsing/building in `uri.rs`. `Polarion::new` is read-only; keep it that way.
  **Every method that modifies data must call `Polarion::check_write` (or `check_write_uri`, `check_global_write`,
  `check_write_scope`, `check_any_write`) before sending anything** (tests in `tests/items_write.rs`, `tests/areas.rs`).
  `tests/coverage.rs` fails when a SOAP operation is neither called by the library nor listed in its `EXCLUDED` with a
  reason; new operations from a newer WSDL must be exposed or excluded there. Writes are verified against wiremock only; keep the
  "Not yet verified" list in README.md current.
- `polarion-commands`: one module per area (mirroring the library handles); every command is a DTO struct with
  `#[cmd(write?, view = ...)]` and a hand-written `impl Run` (`type Output` is the typed result; it must implement
  `JsonSchema` describing its serialized form, which becomes the MCP output schema). The name comes
  from the type name (kebab-case), the summary and help from doc comments; `#[positional]` and `#[flatten]` mark
  fields, a field named `project` is the default-project override. Every DTO must be listed in `registry!` in
  `lib.rs`, which generates `AnyCommand`/`AnyOutput`, the catalog (`Entry`) and the clap tree (feature `cli`).
  Typed argument types (`ItemArg`, `DocumentArg`, `RichText`, `FieldSelection`, `Data`, `QueryArgs`, ...) are in
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
  the curated tools; a disabled command is refused by every tool, `polarion_call` included). Byte results
  are shown as image/text blocks or saved to the opt-in exchange folder (`files.rs`, `POLARION_MCP_FILES`:
  `save_as`, `<param>-file`, plain file names only). Over HTTP all clients share one Polarion
  session, so commands that need a session of their own (transactions, `SESSION_BOUND`) are refused there. End-to-end tests (`tests/cli.rs`,
  `tests/mcp.rs`) spawn the binaries against wiremock.
- New resource types in the spec need a Rust name in `xtask/src/names.rs` (codegen fails otherwise).
- Architecture docs are in `docs/` (Markdown with Mermaid diagrams; README.md covers usage). The diagrams name real
  functions, types and files: when you change a mechanism a page shows, update the page in the same change.
- Before pushing: `cargo xtask codegen --check && cargo fmt --all --check && cargo clippy --workspace --all-targets && cargo test --workspace`.
- Live tests (`crates/polarion-rest/tests/live.rs`) only ever target testdrive.polarion.com; never point anything
  that writes at another server. They need `POLARION_TESTDRIVE_TOKEN` and `POLARION_TESTDRIVE_PROJECT`.
