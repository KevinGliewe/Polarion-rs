# Polarion-rs

Rust workspace: `crates/polarion-rest` (generated REST client), `crates/polarion-soap` (generated SOAP client),
`crates/polarion` (hand-written library on top of `polarion-soap`), `crates/polarion-commands` (every library method
as a JSON command), `crates/polarion-cli` (`polarion` binary) and `crates/polarion-mcp` (`polarion-mcp` MCP server),
both built from the command catalog, `xtask` (code generators).
The repository is English only: code, comments, docs and commit messages.

- Never edit `crates/*/src/generated/*` or `crates/*/tests/spec_examples.rs` by hand. Change the generator
  (`xtask/src/` for REST, `xtask/src/soap/` for SOAP) and run `cargo xtask codegen`.
- Hand-written REST runtime: `client.rs` (HTTP plumbing, pagination), `jsonapi.rs` (envelope types), `query.rs`, `error.rs`.
- Hand-written SOAP runtime: `client.rs` (envelope, session incl. renewal after expiry, faults, read-only allowlist), `xml.rs` (XML helpers), `error.rs`.
  WSDL quirks are handled in `xtask/src/soap/emit.rs`: params documented as accepting null become `Option`
  (`nullable_params`), plus `NILLABLE_PARAMS`/`OPTIONAL_PARAMS` overrides; `READ_PREFIXES`/`OTHER_READS` decide what
  read-only clients allow; subtype elements (e.g. `EnumCustomFieldType.enumId`) are decoded into the base struct.
- `polarion`: one handle per area (`items.rs`, `documents.rs`, `schema.rs`, `tests_mgmt.rs`, `plans.rs`, `wiki.rs`,
  `baselines.rs`, `revisions.rs`, `activities.rs`, `folders.rs`, `projects.rs`, `security.rs`, `builds.rs`), domain types in
  `model.rs` (serde, camelCase, empty fields skipped), write inputs and JSON coercion in `write.rs`, SOAP ↔ domain
  mappings in `convert.rs`, URI parsing/building in `uri.rs`. `Polarion::new` is read-only; keep it that way.
  **Every method that modifies data must call `Polarion::check_write` (or `check_write_uri`, `check_global_write`,
  `check_write_scope`, `check_any_write`) before sending anything** (tests in `tests/items_write.rs`, `tests/areas.rs`).
  `tests/coverage.rs` fails when a SOAP operation is neither called by the library nor listed in its `EXCLUDED` with a
  reason; new operations from a newer WSDL must be exposed or excluded there. Writes are verified against wiremock only; keep the
  "Not yet verified" list in README.md current.
- `polarion-commands`: one file per area (mirroring the library handles) with a `COMMANDS` table (`command!` macro)
  and one `async fn(&Polarion, &Args) -> Result<Value>` per command; parameter types in `catalog.rs`, JSON argument
  parsing (item/document refs, queries, rich text, custom field coercion) in `args.rs`, environment configuration in
  `config.rs`. `tests/coverage.rs` fails when a public library method has no command: **a new library method needs a
  command**. Write commands set `write: true` and rely on the library's guard.
- `polarion-cli` and `polarion-mcp` contain no Polarion logic: the clap tree (`cli.rs`) and the MCP tools
  (`server.rs`) are generated from the catalog. CLI text output in `render.rs` (table columns come from the
  command's `Output`); JSON output is the library's serde form, don't reshape it. The MCP server speaks
  newline-delimited JSON-RPC on stdio; its curated tools are the `CURATED` table. End-to-end tests (`tests/cli.rs`,
  `tests/mcp.rs`) spawn the binaries against wiremock.
- New resource types in the spec need a Rust name in `xtask/src/names.rs` (codegen fails otherwise).
- Before pushing: `cargo xtask codegen --check && cargo fmt --all --check && cargo clippy --workspace --all-targets && cargo test --workspace`.
- Live tests (`crates/polarion-rest/tests/live.rs`) only ever target testdrive.polarion.com; never point anything
  that writes at another server. They need `POLARION_TESTDRIVE_TOKEN` and `POLARION_TESTDRIVE_PROJECT`.
