# Polarion-rs

Rust workspace: `crates/polarion-rest` (generated REST client), `xtask` (code generator). Planned: `polarion`
(ergonomic library), `polarion-cli`, `polarion-mcp`.

- Never edit `crates/polarion-rest/src/generated/*` or `crates/polarion-rest/tests/spec_examples.rs` by hand.
  Change the generator in `xtask/src/` and run `cargo xtask codegen`.
- Hand-written runtime: `client.rs` (HTTP plumbing, pagination), `jsonapi.rs` (envelope types), `query.rs`, `error.rs`.
- New resource types in the spec need a Rust name in `xtask/src/names.rs` (codegen fails otherwise).
- Before pushing: `cargo xtask codegen --check && cargo fmt --all --check && cargo clippy --workspace --all-targets && cargo test --workspace`.
