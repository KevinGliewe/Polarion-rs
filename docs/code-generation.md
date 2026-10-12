# Code generation (`xtask`)

The SOAP client is generated from the WSDLs Polarion serves. The generated code is committed,
so building the workspace needs no generator. CI runs `cargo xtask codegen --check` and fails when the committed
code is stale.

```sh
cargo xtask codegen          # regenerate
cargo xtask codegen --check  # verify it is up to date
```

Never edit `src/generated/*` or `tests/spec_examples.rs` by hand: change the generator and regenerate.

## The pipeline

```mermaid
flowchart LR
    wsdl["spec/soap/*.wsdl<br/>7 services"] --> wmodel["soap/wsdl.rs<br/>Model::load"]
    wmodel --> semit["soap/emit.rs<br/>Gen: types(), services(), tests()<br/>names.rs, docs.rs: Rust names, rustdoc"]
    semit --> sout["polarion-soap/src/generated/<br/>types.rs, services.rs"]
    semit --> stest["polarion-soap/tests/<br/>spec_examples.rs"]
    sout & stest --> fmt["rustfmt --edition 2024"]
    fmt --> check{"--check?"}
    check -- no --> write(["write changed files"])
    check -- yes --> stale(["compare: fail and name the stale files"])
```

`cargo xtask codegen` runs it ([`xtask/src/main.rs`](../xtask/src/main.rs)).

## SOAP: `spec/soap/*.wsdl` → `polarion-soap`

The WSDLs come from a Polarion 2410 server (`<polarion>/ws/services/<Name>WebService?wsdl`, no login needed). The
SOAP API barely changes between versions: compared with testdrive's newer release, all 302 operations are
the same, and the only type difference is `Hyperlink.title`, which 2410 lacks.

[`soap/emit.rs`](../xtask/src/soap/emit.rs) generates:

| Output | What it contains |
|---|---|
| `types.rs` | a struct per complex type with XML encoding and decoding; `Serialize` (feature `serde`) and a `JsonSchema` that mirrors it (feature `schemars`); `AnyValue` with a variant per type; `NAMESPACES` |
| `services.rs` | one accessor per service (`client.tracker()`), one method per operation, each passing the operation's read-only flag to `Client::call` |
| `tests/spec_examples.rs` | a round-trip test per type, directly and as an `xsi:type`-tagged `AnyValue` |

Javadoc in the WSDLs becomes the method docs. The WSDLs don't say everything, so the generator corrects them:

```mermaid
flowchart TD
    p["a WSDL parameter or field"] --> d{"its Javadoc says<br/>it accepts null?"}
    d -- yes --> opt["Option, sent as xsi:nil<br/>(nullable_params)"]
    d -- no --> o{"listed in NILLABLE_PARAMS<br/>or OPTIONAL_PARAMS?"}
    o -- yes --> opt2["Option, nil or omitted"]
    o -- no --> req["required, as the WSDL says"]
    op["an operation"] --> r{"name starts with a READ_PREFIXES entry,<br/>or listed in OTHER_READS?"}
    r -- yes --> read["read: allowed on read-only clients"]
    r -- no --> write["write: refused on read-only clients"]
    st["a subtype element<br/>e.g. EnumCustomFieldType.enumId"] --> base["extra field on the base struct,<br/>decoded from the subtype"]
```

## Round-trip tests

Both `spec_examples.rs` suites build an example document from every schema, independently of the generator's type
model, and check that it survives a trip through the generated types. A mapping gap therefore shows up as a test
failure, not as a silently dropped field.
