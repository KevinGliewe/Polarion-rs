# Polarion-rs

Rust tooling for [Siemens Polarion ALM](https://polarion.plm.automation.siemens.com/).

| Crate | Status | Purpose |
|---|---|---|
| `polarion-rest` | in progress | Low-level async client for the Polarion REST API (v1), generated from the OpenAPI spec |
| `polarion` | planned | Ergonomic library on top of `polarion-rest` |
| `polarion-cli` | planned | Command-line interface, with JSON output for automation |
| `polarion-mcp` | planned | MCP server so AI agents can work with Polarion |

## OpenAPI spec

`spec/polarion-openapi.json` is the Polarion REST API definition (Apache-2.0), fetched from
`https://testdrive.polarion.com/polarion/rest/v1/definition`. Your own server serves the same
document at `<polarion>/rest/v1/definition`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
