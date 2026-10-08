# Polarion-rs

Rust workspace: `crates/polarion-rest` (generated REST client), `crates/polarion-soap` (generated SOAP client),
`xtask` (code generators). Planned: `polarion` (ergonomic library), `polarion-cli`, `polarion-mcp`.
The repository is English only: code, comments, docs and commit messages.

- Never edit `crates/*/src/generated/*` or `crates/*/tests/spec_examples.rs` by hand. Change the generator
  (`xtask/src/` for REST, `xtask/src/soap/` for SOAP) and run `cargo xtask codegen`.
- Hand-written REST runtime: `client.rs` (HTTP plumbing, pagination), `jsonapi.rs` (envelope types), `query.rs`, `error.rs`.
- Hand-written SOAP runtime: `client.rs` (envelope, session, faults, read-only allowlist), `xml.rs` (XML helpers), `error.rs`.
  WSDL quirks are handled in `xtask/src/soap/emit.rs` (`NILLABLE_PARAMS`, `OPTIONAL_PARAMS`, `READ_PREFIXES`).
- New resource types in the spec need a Rust name in `xtask/src/names.rs` (codegen fails otherwise).
- Before pushing: `cargo xtask codegen --check && cargo fmt --all --check && cargo clippy --workspace --all-targets && cargo test --workspace`.
- Live tests (`crates/polarion-rest/tests/live.rs`) only ever target testdrive.polarion.com; never point anything
  that writes at another server. They need `POLARION_TESTDRIVE_TOKEN` and `POLARION_TESTDRIVE_PROJECT`.
