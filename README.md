# Polarion-rs

Rust tooling for [Siemens Polarion ALM](https://polarion.plm.automation.siemens.com/): a command-line tool, an MCP
server for AI agents, and the libraries behind them.

**Start here:**

| I want to ... | Go to |
|---|---|
| query and edit Polarion from a terminal or script | [polarion (command line)](#polarion-command-line) |
| let an AI agent (Claude, Copilot, Codex) work with Polarion | [polarion mcp (MCP server)](#polarion-mcp-mcp-server), setup guide in [examples/mcp](examples/mcp/README.md) |
| call Polarion from Rust | [polarion (library)](#polarion) |
| convert Markdown ↔ Polarion rich text | [polarion-markdown](#polarion-markdown) |
| understand or change the code | [docs/](docs/README.md) (architecture, with diagrams) |

> **Safe by default:** both the CLI and the MCP server are **read-only** until writes are enabled for named
> projects or objects (see [Writes](#writes)).

## Contents

- [Polarion-rs](#polarion-rs)
  - [Contents](#contents)
  - [Crates](#crates)
  - [polarion (command line)](#polarion-command-line)
    - [Install](#install)
    - [Configure](#configure)
    - [Use](#use)
    - [Enabling writes](#enabling-writes)
    - [Arguments](#arguments)
    - [Markdown descriptions](#markdown-descriptions)
    - [Output and exit codes](#output-and-exit-codes)
  - [polarion mcp (MCP server)](#polarion-mcp-mcp-server)
    - [Install](#install-1)
    - [Connect a client](#connect-a-client)
    - [Tools](#tools)
    - [Guides first](#guides-first)
    - [Choosing tools](#choosing-tools)
    - [Results and errors](#results-and-errors)
    - [Attachments](#attachments)
    - [Shared server over HTTP](#shared-server-over-http)
    - [Docker](#docker)
  - [polarion-commands](#polarion-commands)
  - [polarion](#polarion)
    - [Writes](#writes)
      - [What has been checked against a real server](#what-has-been-checked-against-a-real-server)
  - [polarion-markdown](#polarion-markdown)
  - [polarion-soap](#polarion-soap)
  - [Code generation](#code-generation)
  - [License](#license)

## Crates

| Crate | Status | Purpose |
|---|---|---|
| [`polarion-cli`](crates/polarion-cli) | usable (writes mock-tested only) | The one binary, `polarion`: a command-line tool covering the whole library, with JSON output for automation, and the MCP server as `polarion mcp` |
| [`polarion-mcp`](crates/polarion-mcp) | usable (writes mock-tested only) | The MCP server (stdio or Streamable HTTP) so AI agents can work with Polarion; a library that `polarion mcp` runs, without a binary of its own |
| [`polarion`](crates/polarion) | usable (writes mock-tested only) | Convenient, JSON-serializable API over the whole SOAP API, by area, on top of `polarion-soap` |
| [`polarion-markdown`](crates/polarion-markdown) | usable (Polarion's handling of the HTML not yet verified) | Markdown ↔ Polarion rich text (HTML) in both directions, mermaid diagrams rendered to SVG; no I/O |
| [`polarion-commands`](crates/polarion-commands) | usable | Every library method as a typed command DTO (serde, JSON Schema, clap); shared by the CLI and the MCP server |
| [`polarion-commands-derive`](crates/polarion-commands-derive) | internal | `#[cmd]`, `#[cmd_args]` and `registry!`: the code generation behind `polarion-commands` |
| [`polarion-soap`](crates/polarion-soap) | usable | Async client for the Polarion SOAP web services, generated from the WSDLs |

How the crates fit together, how a call flows through them, and how the code is generated and tested:
[docs/](docs/README.md).

## polarion (command line)

### Install

```sh
cargo install polarion-cli                  # from crates.io
cargo install --path crates/polarion-cli    # or from a checkout
```

Or download the ready-built binary. Every [GitHub release](https://github.com/KevinGliewe/Polarion-rs/releases)
has one file per platform, the executable itself (nothing to unpack), and the same address always gives the newest
release:

```sh
# Linux (x86_64 or aarch64); the musl build is static and runs on every distribution
sudo curl -fL https://github.com/KevinGliewe/Polarion-rs/releases/latest/download/polarion-rs-$(uname -m)-unknown-linux-musl \
  -o /usr/local/bin/polarion && sudo chmod +x /usr/local/bin/polarion

# macOS, Apple silicon (Intel: x86_64-apple-darwin)
sudo curl -fL https://github.com/KevinGliewe/Polarion-rs/releases/latest/download/polarion-rs-aarch64-apple-darwin \
  -o /usr/local/bin/polarion && sudo chmod +x /usr/local/bin/polarion
```

```powershell
# Windows (ARM64: aarch64-pc-windows-msvc); put the folder on PATH
curl.exe -fL https://github.com/KevinGliewe/Polarion-rs/releases/latest/download/polarion-rs-x86_64-pc-windows-msvc.exe -o polarion.exe
```

| Platform | Files (`polarion-rs-<target>`) |
|---|---|
| Linux | `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` (static), `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` (glibc) |
| macOS | `aarch64-apple-darwin` (Apple silicon), `x86_64-apple-darwin` (Intel) |
| Windows | `x86_64-pc-windows-msvc.exe`, `aarch64-pc-windows-msvc.exe` |

- `-f` makes `curl` fail on a wrong name instead of saving the error page as the binary.
- Each file has a SHA-256 checksum next to it (`<file>.sha256`): download both and run `sha256sum -c <file>.sha256`.
- A specific version: `.../releases/download/v0.4.0/<file>` instead of `.../releases/latest/download/<file>`.
- The binaries are not signed. A browser download on macOS is quarantined (`xattr -d com.apple.quarantine polarion`
  removes that; `curl` does not set it), and Windows may ask before the first start.
- The licenses are [MIT](LICENSE-MIT) and [Apache-2.0](LICENSE-APACHE).

**Upgrading from 0.3:** there is one binary now. The MCP server is `polarion mcp`; the executable `polarion-mcp`
and the release archives are gone.

- In MCP client configurations, replace the command `polarion-mcp` with `polarion` and the argument `mcp`
  (`"command": "polarion", "args": ["mcp"]`); `--http` and `--config` follow `mcp`. The environment variables are
  unchanged.
- Remove an old `polarion-mcp` from your `PATH` (`cargo uninstall polarion-mcp`), or clients keep starting it.
  `cargo install polarion-mcp` no longer installs a program: use `cargo install polarion-cli`.
- The Docker images keep their names and arguments.
- `POLARION_INSECURE` and `POLARION_ALLOW_GLOBAL_WRITES` are now read the same way by every command: `1`, `true`,
  `yes`, `on` (in any case) switch them on, an empty value is off, and anything unclear is an error instead of "off".

### Configure

Settings come from the environment. Credentials come only from there, never from flags.

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

### Use

Commands are `polarion <area> <command>`, one per library method (about 250).

- `polarion commands [AREA]` lists them.
- `--help` on any command shows its arguments.

```sh
polarion commands items
polarion projects get
polarion items query "type:requirement AND status:open" --sort ~updated --limit 20
polarion items query "type:requirement" --fields id,title,description --custom asil --json
polarion items count "type:requirement"
polarion items get MP-42                    # or OtherProject/OP-7, or a URI
polarion items links MP-42                  # outgoing and incoming
polarion items history MP-42
polarion items description-markdown MP-42 > desc.md
polarion items set-description-markdown MP-42 --markdown @desc.md   # a write: see below
polarion items create --type task --title "Brakes" --categories software --links parent=MP-42   # one transaction
polarion documents list [--space Specs]
polarion documents get Specs/SRS [--body]
polarion documents work-items Specs/SRS
polarion schema custom-fields --type requirement
polarion tests search "status:open"
polarion plans statistics Release-1
polarion users get jdoe
polarion items attachment MP-42 --attachment 1-spec.pdf --out spec.pdf
```

### Enabling writes

Commands marked `write` in `polarion commands` are refused before anything is sent, unless one of these allows them:

| Setting | Flag | Allows |
|---|---|---|
| `POLARION_WRITE_PROJECTS` | `--write-projects` | writes to the listed projects |
| `POLARION_ALLOW_GLOBAL_WRITES=1` | | in addition, writes not tied to a project (users, global folders) |
| `POLARION_WRITE_TARGETS` | `--write-targets` | writes to the listed objects only; overrides the two above |

Each run ends its server session.

**Write targets** in detail:

- **Set but empty**, `POLARION_WRITE_TARGETS` allows no writes at all.
- **Entries**, comma-separated (`\,` is a comma inside a name):
  - work items: `ID` or `PROJECT/ID` (work item ids are `PREFIX-NUMBER`)
  - documents: `doc:SPACE/NAME` or `doc:PROJECT/SPACE/NAME`
  - test runs: `run:ID` or `run:PROJECT/ID`
  - anything else: a URI
- **Project of an entry without one:** `POLARION_PROJECT` as it is in the environment. `-p` does not move them;
  entries of `--write-targets` are in the project of `-p`.
- **Allowed for the targets:** their fields, attachments, work records, comments and test records, including
  deleting attachments, work records and test records.
- **Refused even for the targets:** creates, deletes of whole objects, workflow actions, status and resolution
  changes, a document's structure link role, and the body of a document that holds work items.
- **Named objects:** every object a write names (link targets, test cases, defects, plan items) must be a target
  too.
- **Not prevented:** what the server does in response, such as suspect links on work items that link to a target,
  and notifications. Check a work item's back links before making it a target.

### Arguments

| Kind | Forms |
|---|---|
| Work items | `ID` (in the default project), `PROJECT/ID` or a URI |
| Documents | `SPACE/NAME`, `PROJECT/SPACE/NAME` or a URI |
| Structured arguments (custom fields, test records) | JSON, e.g. `--custom-fields '{"asil": "D"}'` |
| Rich text | a string, or `{"type": "text/html", "content": "..."}` |
| Text (`--markdown`) | the text (it may start with `-`, as a list does), `@FILE`, `@-` (standard input), or `@@...` (text that starts with `@`) |
| Binary | base64 or `@FILE` |
| Paths (`--file`, `--dir`) | a file or folder; relative paths are relative to the working directory |

- Files and standard input are read as UTF-8, or as UTF-16 with a byte order mark.
- Binary results and Markdown go to `--out FILE`.

### Markdown descriptions

- `items description-markdown` prints the description as Markdown.
- `items set-description-markdown` and `items append-description-markdown` write it.
- **Mermaid blocks** become SVG attachments shown as images, and are read back as mermaid blocks. An unchanged
  diagram is not uploaded again.
- **Images from files:** `--file doc.md` (instead of `--markdown`) reads the Markdown from a file and attaches the
  images it names by relative paths (`![](img/a.png)`), so the description shows them inline. They must be PNG,
  JPEG, GIF, WebP or SVG files in the folder of the Markdown file or below it. An image the work item already has
  an attachment with the same content for is not uploaded again. Image attachments are never deleted.
- **Export:** `items export-description ITEM --dir out` writes `out/ITEM.md` and the images the description shows
  as `out/ITEM/<attachment id>`. `items set-description-markdown ITEM --file out/ITEM.md` writes it back.

  ```sh
  polarion items export-description MP-42 --dir out
  # edit out/MP-42.md, add or replace images next to it
  polarion items set-description-markdown MP-42 --file out/MP-42.md
  ```
- **Work item ids become links** when they have the work item's own prefix: `MP-7` in the description of `MP-42`,
  but not `UTF-8`. `--id-prefixes MP,OP` names the prefixes, `'*'` allows any, `none` turns links off.
- **Formatting Markdown cannot express** is read as HTML, so writing the Markdown back keeps it. `--drop-html`
  leaves it out.
- A diagram attachment whose delete failed (a warning) is not deleted by later writes either.

<details>
<summary>Rendering time and the font cache</summary>

Rendering a diagram takes milliseconds, a large one seconds. The first render of a process loads the system fonts,
unless the font is already in the cache; after loading, it is copied there. The cache is
`$XDG_CACHE_HOME/mmdr/font-cache`, else `$HOME/.cache/mmdr/font-cache`; with neither set there is no cache.

</details>

Details: [docs/markdown.md](docs/markdown.md).

### Output and exit codes

- **Text output** is for people: tables for lists, an indented `key: value` view for single objects, rich text with
  HTML tags stripped, Markdown as it is.
- **`--json`** prints the library's JSON form: pretty on a terminal, one line when piped.
  - Errors go to stderr; in JSON mode as `{"error": {"code": "notFound", "message": "..."}}`.
  - `polarion --json commands` includes the JSON Schema of every command's arguments.
- **Exit codes:** 0 success, 1 error, 2 usage error, 3 not found.
- **Limits:** queries return at most 100 results unless `--limit N` or `--all`. A note on stderr says when results
  were cut off.

## polarion mcp (MCP server)

`polarion mcp` is an [MCP](https://modelcontextprotocol.io) server that lets AI agents work with Polarion. It is
part of the `polarion` binary.

- **Step-by-step setup** on Windows, from read-only to full write access, with configurations for Claude Code,
  Claude Desktop, VS Code and Codex: [examples/mcp](examples/mcp/README.md).
- **Transports:** stdio (default) or Streamable HTTP (`--http`, see [below](#shared-server-over-http)).
- **Built on** the official Rust SDK ([`rmcp`](https://github.com/modelcontextprotocol/rust-sdk)), which negotiates
  the protocol version (up to `2026-07-28`).
- **Configured** by the same environment variables as the other [commands](#configure), or by the same global
  options (`polarion --url ... mcp`); `--json` and `--out` do not apply to it.

### Install

It comes with `polarion`: see [Install](#install). `polarion mcp --help` describes the options, the config file and
the environment variables.

### Connect a client

Claude Code:

```sh
claude mcp add polarion -e POLARION_URL=https://polarion.example.com/polarion \
  -e POLARION_TOKEN=<token> -e POLARION_PROJECT=MyProject -- polarion mcp
```

Claude Desktop and other clients (`claude_desktop_config.json`, `.mcp.json`):

```json
{
  "mcpServers": {
    "polarion": {
      "command": "polarion",
      "args": ["mcp"],
      "env": {
        "POLARION_URL": "https://polarion.example.com/polarion",
        "POLARION_TOKEN": "<token>",
        "POLARION_PROJECT": "MyProject"
      }
    }
  }
}
```

### Tools

**Everyday reads**, each a tool of its own:

| Area | Tools |
|---|---|
| Work items | `get_work_item`, `query_work_items`, `count_work_items`, `work_item_links`, `work_item_history`, `get_work_item_description_markdown` |
| Documents | `list_documents`, `get_document`, `document_work_items`, `search_documents`, `export_document_pdf` |
| Schema | `custom_fields`, `field_options` |
| Test runs | `search_test_runs`, `get_test_run`, `test_case_steps`, `test_run_records`, `test_run_report` |
| Plans | `search_plans`, `get_plan`, `plan_statistics` |
| Wiki | `query_wiki_pages`, `get_wiki_page` |
| Project and user | `get_project`, `get_user` |

**Everyday writes**, listed only when writes are enabled (`POLARION_WRITE_PROJECTS`, `POLARION_ALLOW_GLOBAL_WRITES`
or `POLARION_WRITE_TARGETS`):

| Area | Tools |
|---|---|
| Work items | `create_work_item`, `update_work_item`, `set_work_item_description_markdown`, `append_work_item_description_markdown`, `add_work_item_comment`, `add_work_item_link`, `remove_work_item_link`, `add_work_item_attachment` |
| Documents | `create_document`, `update_document`, `add_document_comment` |
| Test runs | `create_test_run`, `update_test_run`, `add_test_record`, `set_test_steps` (the steps of a test case, a work item) |
| Plans | `create_plan`, `update_plan`, `add_plan_items`, `remove_plan_items` |

- A read-only server does not list the write tools at all. `polarion_call` still reaches those commands, and they
  are refused.
- When the write settings cannot reach an area, its tools are listed and refused, and the instructions say which.
  That happens with global writes alone, or with write targets of other kinds only (plans are written only by a
  plan's URI).

**Guides for agents:** `polarion_guide(area)` returns a short Markdown guide to one area: `overview`, `queries`,
`work-items`, `documents`, `tests`, `plans` or `wiki`.

- A section read from the project follows the guide: its work item types and statuses; for work items and queries
  the link roles; for work items the custom field keys by type. A read that fails becomes a note.
- The guides are in [`crates/polarion-mcp/guides/`](crates/polarion-mcp/guides).
- They are also MCP resources (`polarion://guide/<area>`, `text/markdown`, without the project section).

**Generic tools that reach every command:**

- `polarion_commands`: areas, or the commands of one area
- `polarion_describe`: a command's argument schema
- `polarion_call`: run a command

**Totals:**

| Server | Tools |
|---|---|
| read-only (default) | 29: 25 reads, `polarion_guide`, 3 generic tools |
| writes enabled | 48: the 29 above plus 19 writes |

They are listed in this order: reads, writes, `polarion_guide`, generic tools. A config file picks more dedicated
tools, or fewer (see [Choosing tools](#choosing-tools)).

### Guides first

By default (`require_guides = "write"`), a server that may write refuses an area's writes until `polarion_guide`
was called for that area.

- This applies to their own tools and to `polarion_call` alike.
- The tool error `error (guideRequired): ...` names the guide. Nothing is sent to Polarion.
- **Which guide:** work item commands need `work-items`. Documents, tests, plans and wiki need their own guide.
  Every other area (schema, projects, users, ...) needs `overview`.
- **Other settings:** `"all"` requires the guide before every command of an area, reads included. `"none"` turns
  the gate off.
- It comes on top of the write settings, never instead of them.
- **Memory:** the guides read are remembered per server process: over stdio for the one client, over HTTP for all
  clients together.
- `POLARION_MCP_REQUIRE_GUIDES` (`write`, `all`, `none`) wins over the file. Disabling `polarion_guide` turns the
  default gate off.

### Choosing tools

Every tool definition costs the agent context:

| Selection | Size of `tools/list` |
|---|---|
| default, read-only | about 72 KB (53 KB of it output schemas, 19 KB everything else) |
| default, writes enabled | about 94 KB |
| `all` (about 240 tools) | about 288 KB; 121 KB without output schemas |
| `all`, writes enabled (258 tools) | about 309 KB; 139 KB without output schemas |

An optional TOML file, given with `--config FILE` or `POLARION_MCP_CONFIG=FILE`, picks what is offered. Every key
is optional:

```toml
# Commands with a tool of their own. Default: "default", the everyday reads above, and the everyday writes when
# writes are enabled; "all": every command
tools = ["default", "documents.*"]
# Switched off everywhere: no tool, not listed by polarion_commands, refused by polarion_describe and
# polarion_call. Generic tools can be left out by name.
disable = ["*.delete", "security.*", "polarion_describe"]
# outputSchema on tools and structuredContent on results (default: true)
output_schemas = false
# Commands refused until polarion_guide was called for their area: "write" (default), "all" or "none"
require_guides = "all"
```

- **Selectors:** command names (`items.get`, `items.*`, `*.delete`), tool names (`get_work_item`,
  `items_add_hyperlink`), or `default`/`all`. `*` is a wildcard.
- **Tool names:** commands with a curated tool keep its name; the others are named `area_command`
  (`items_add_hyperlink`).
- **Curated writes** are listed only when writes are enabled, whatever selects them.
- **`POLARION_MCP_TOOLS`** takes the same selectors, comma-separated (`POLARION_MCP_TOOLS=all`), and wins over the
  file's `tools`.
- **Mistakes stop the server** with an error: a selector that matches nothing, an unknown key, an unreadable file.
- **Disabling is a real restriction**, not just a shorter list, so it doubles as a deny list.

### Results and errors

- **Results** are the library's JSON, both as text and as structured content.
  - Every tool that returns data declares an `outputSchema`, generated from the command's result type. Its result
    carries `structuredContent` that matches it (unless `output_schemas = false`).
  - Results that are not JSON objects (lists, counts, flags) are wrapped as `{"result": ...}`, which every protocol
    version accepts.
  - `polarion_describe` shows a command's output schema too.
- **Failures**, refused writes included, are tool errors with a message such as `error (readOnly): ...`.
- **Write access:** the server is read-only unless `POLARION_WRITE_PROJECTS` or `POLARION_WRITE_TARGETS` is set
  (see [Enabling writes](#enabling-writes)).
  - It tells the agent what it may write when it connects.
  - The targets are resolved once, at startup, in `POLARION_PROJECT`.

### Attachments

Work item attachments (`items.attachment`) and PDF exports (`documents.export-pdf`) never reach the agent as
base64:

| Content | Comes back as |
|---|---|
| images (PNG, JPEG, GIF, WebP) up to 4 MB | image content the model can look at |
| UTF-8 text up to 100 KB | text |
| anything else | its size and type |

For real files, set an exchange folder, `POLARION_MCP_FILES=/path/to/dir` (off by default):

- **Downloads** take `save_as: "spec.pdf"` and are written to that folder; the result says where.
- **Uploads** (`items.add-attachment`, `tests.add-attachment`, ...) take `data-file: "notes.txt"` instead of base64
  `data`, and read the file from that folder.
- **Only plain file names** inside the folder are accepted: no paths, no `..`, no symbolic links.
- **Where it is:** with stdio the folder is on your machine, so the agent can open what it saved. Over HTTP it is
  on the server and shared by all clients.

Limits of Polarion's SOAP API: test run attachments can be listed, but their content is not available. Documents,
wiki pages and plans have no attachment operations at all.

### Shared server over HTTP

`polarion mcp --http ADDR` (or `POLARION_MCP_HTTP=ADDR`) serves MCP's Streamable HTTP transport at
`http://ADDR/mcp`, for any number of clients, all using the server's Polarion account.

Start the server:

```sh
export POLARION_URL=https://polarion.example.com/polarion POLARION_TOKEN=<service account token>
export POLARION_MCP_AUTH_TOKEN=$(openssl rand -hex 32)   # clients send it as a bearer token
polarion mcp --http 127.0.0.1:8787
```

Connect Claude Code:

```sh
claude mcp add --transport http polarion https://mcp.example.com/mcp \
  --header "Authorization: Bearer $POLARION_MCP_AUTH_TOKEN"
```

A client that cannot set headers gives the token in the URL instead: `https://mcp.example.com/mcp/<token>`
(percent-encoded, e.g. `%20` for a space; a trailing `/` is fine).

Or another client:

```json
{
  "mcpServers": {
    "polarion": {
      "type": "http",
      "url": "https://mcp.example.com/mcp",
      "headers": {"Authorization": "Bearer ${POLARION_MCP_AUTH_TOKEN}"}
    }
  }
}
```

**Access:**

- `POLARION_MCP_AUTH_TOKEN` (at least 16 characters) is required. Clients send it as `Authorization: Bearer
  <token>` to `/mcp`, or as part of the URL, `/mcp/<token>`. Requests without it get 401.
- **Token in the URL:** it ends up wherever URLs are kept: proxy and access logs, the client's configuration,
  browser history. Prefer the header where the client supports it, and keep such logs private.
- **No authentication:** `POLARION_MCP_ALLOW_UNAUTHENTICATED=1` (without `POLARION_MCP_AUTH_TOKEN`; both together
  are refused) accepts every request to `/mcp`. Everyone who reaches the port acts as the Polarion account, with
  its write scope; the startup line says `NO AUTHENTICATION`. Only for a port nobody else reaches (loopback, a
  private Docker network); the Host and Origin checks below stay.
- **Browsers:** requests with an `Origin` header are refused unless the origin is listed, with scheme and port, in
  `POLARION_MCP_ALLOWED_ORIGINS` (e.g. `http://localhost:3000`).
- **Host names:** the server answers only to the `Host` names in `POLARION_MCP_ALLOWED_HOSTS` (default:
  `localhost`, `127.0.0.1`, `::1`). Behind a reverse proxy, list its public name.
- **Plain HTTP only:** bind it to localhost, and put a TLS reverse proxy in front for anything else.

**One identity:**

- Everyone with the token (or, without authentication, everyone who reaches the port) acts as the service account. Polarion's history shows that account, not the
  person.
- Give it the narrowest Polarion role that works (read-only for production).
- Enable writes with `POLARION_WRITE_PROJECTS` only where they are meant to happen.

**One Polarion session** for all clients:

- It is logged in again when it expires, and ended on Ctrl-C/SIGTERM.
- Explicit transactions (`session.begin-transaction`, `commit`, `rollback`) and `session.check` are therefore
  refused over HTTP. Use stdio for them.
- Commands of several writes (a create with categories, assignees or links, an update of several fields) still
  apply them as one transaction (a description with new diagrams: the uploads, then the update and deletes): the library opens a Polarion session of its own
  for each such call (one more login), so it never mixes with other clients' calls.

**Stateless:**

- No `Mcp-Session-Id` sessions and no server-initiated SSE streams. Every request is answered with plain JSON.
- OAuth is not included, so claude.ai web connectors cannot use it.

### Docker

The [`Dockerfile`](Dockerfile) builds two images, both on Debian slim with the one binary `polarion`, run as a
non-root user:

| Image | Target | Runs |
|---|---|---|
| `polarion-mcp` | `mcp` (the default) | `polarion mcp` with the container's arguments: the [HTTP server](#shared-server-over-http) on port 8787 |
| `polarion-cli` | `cli` | `polarion` with the container's arguments (`--help` without any) |

Its stages:

- **build:** both release binaries.
- **test:** the workspace's tests. The images are only built when they pass. `--target build` or `--target test`
  stops early.
- **runtime:** the base of both images, with the entry point that applies the profile (below).

```sh
docker build -t polarion-mcp .
docker run --rm -p 127.0.0.1:8787:8787 \
  -e POLARION_URL=https://polarion.example.com/polarion \
  -e POLARION_TOKEN=<service account token> \
  -e POLARION_PROJECT=MYPROJECT \
  -e POLARION_WRITE_PROJECTS=MYPROJECT \
  -e POLARION_MCP_AUTH_TOKEN=<16+ characters> \
  polarion-mcp

docker build --target cli -t polarion-cli .
docker run --rm -e POLARION_URL -e POLARION_TOKEN -e POLARION_PROJECT polarion-cli items get MP-42
# Files named in arguments (@desc.md, --out) are relative to /work:
docker run --rm -v "$PWD:/work" --user "$(id -u):$(id -g)" -e POLARION_URL -e POLARION_TOKEN \
  polarion-cli items description-markdown MP-42 --out desc.md
```

**Profiles:** each image has the configurations of a few scenarios built in. `POLARION_PROFILE` selects one:

| Image | `POLARION_PROFILE` | Tools / access |
|---|---|---|
| `polarion-mcp` | `full-access` (default) | the tools of [`full-access.toml`](examples/mcp/config/full-access.toml), `POLARION_ALLOW_GLOBAL_WRITES=1` |
| | `every-tool` | every command as a tool ([`every-tool.toml`](examples/mcp/config/every-tool.toml)), `POLARION_ALLOW_GLOBAL_WRITES=1` |
| | `no-deletes` | no deletes, no user accounts ([`no-deletes.toml`](examples/mcp/config/no-deletes.toml)); no global writes |
| | `read-only` | [`read-only.toml`](examples/mcp/config/read-only.toml); the write scope is removed, and write options given as arguments are refused |
| `polarion-cli` | none (default) | the environment as given (read-only without a write scope) |
| | `full-access` | `POLARION_ALLOW_GLOBAL_WRITES=1` |
| | `read-only` | the write scope is removed from the environment (write flags given as arguments still apply) |

- A profile only sets what the container was not given (given empty counts as given):
  `-e POLARION_ALLOW_GLOBAL_WRITES=0` or `-e POLARION_MCP_CONFIG=...` still win. Only `read-only` takes something away (`POLARION_WRITE_PROJECTS`,
  `POLARION_WRITE_TARGETS`, `POLARION_ALLOW_GLOBAL_WRITES`).
- The projects that may be written are always `POLARION_WRITE_PROJECTS`, set at run time; there is no "every
  project" wildcard.
- The profiles are shell fragments in `/etc/polarion/profiles` ([`docker/profiles`](docker/profiles)); the MCP
  tool selections are in `/etc/polarion/mcp`. A profile of your own: mount it and give its path,
  `-v ./mine.sh:/etc/polarion/mine.sh -e POLARION_PROFILE=/etc/polarion/mine.sh`; or mount a config file and set
  `POLARION_MCP_CONFIG`. An unknown profile stops the container (exit code 64) with the list of the built-in ones.
- [`docker/check-images.sh`](docker/check-images.sh) checks built images without a Polarion server.

**Docker Hub:** [`.github/workflows/docker.yml`](.github/workflows/docker.yml) builds both images, checks them, and
pushes `polarion-cli` and `polarion-mcp`: `edge` and `sha-<commit>` from `main`; `1.2.3`, `1.2`, `1` and `latest`
from a semver tag (a pre-release only its version). See [Releasing](docs/releasing.md#docker-images).

**Base images:** from Docker Hub. Where it is blocked or rate-limited, `--build-arg BASE_REGISTRY=...` with
`docker build` (the variable `BASE_REGISTRY` in the workflow) names a mirror of the official images, e.g.
`mirror.gcr.io/library`. `RUST_IMAGE` and `RUNTIME_IMAGE` replace a base image whole (their defaults are built from
`BASE_REGISTRY`, `RUST_VERSION` and `DEBIAN_RELEASE`); keep both on the same Debian release.

**compose:** [`compose.yaml`](compose.yaml) runs `polarion-mcp` with the variables it lists, taken from the host's
environment (or a `.env` file next to it). It builds the image from the `Dockerfile` (target `mcp`):

```sh
export POLARION_URL=https://polarion.example.com/polarion POLARION_TOKEN=<service account token>
export POLARION_PROJECT=MYPROJECT
export POLARION_MCP_AUTH_TOKEN=$(openssl rand -hex 32)
docker compose up -d --build
```

- **Token:** `POLARION_MCP_AUTH_TOKEN`, or `POLARION_MCP_ALLOW_UNAUTHENTICATED=1` instead; with neither (or both) the
  server does not start.
- **Build arguments:** `BASE_REGISTRY`, `RUST_VERSION`, `DEBIAN_RELEASE`, `RUST_IMAGE` and `RUNTIME_IMAGE` come from
  the host or the `.env` file too (`docker compose --env-file FILE` names another file); those not set keep the
  Dockerfile's defaults.
- **A published image instead:** `POLARION_MCP_IMAGE=<namespace>/polarion-mcp:latest`, then `docker compose pull`
  before `docker compose up -d` (without the pull, compose builds the image under that name).

- **Passed through by name:** only the variables listed under `environment:`; one not set on the host is not set in
  the container. Add the others you need there (`POLARION_WRITE_PROJECTS`, `POLARION_PROFILE`,
  `POLARION_MCP_ALLOWED_HOSTS`, ...).
- **Port:** `127.0.0.1:8787`; `POLARION_MCP_PORT` changes the host port.

**Who can reach it:**

- Inside the container the server listens on `0.0.0.0:8787`, so that Docker's port forwarding reaches it. What is
  reachable from outside is decided by `-p`:

  | `docker run` option | Who can connect |
  |---|---|
  | `-p 127.0.0.1:8787:8787` | Only programs on the Docker host |
  | `-p 8787:8787` | Anyone who can reach the host on that port. On Linux, Docker's port rules bypass host firewalls such as ufw |
  | no `-p` | Only containers on the same Docker network |

- The [access checks](#shared-server-over-http) still apply: every request needs the token (in the header or the
  URL; unless `POLARION_MCP_ALLOW_UNAUTHENTICATED=1`), and the server
  answers only to the `Host` names in `POLARION_MCP_ALLOWED_HOSTS` (default: `localhost`, `127.0.0.1`, `::1`). A
  request to `http://192.168.1.50:8787/mcp` is refused until that name is listed.
- **Remote access:** the token, and with it write access to Polarion, would cross the network in plain HTTP.
  Publish the port only on `127.0.0.1` (or a Docker network), put a TLS reverse proxy in front, and set
  `POLARION_MCP_ALLOWED_HOSTS` to the proxy's public name.

## polarion-commands

Every library method as a command: a DTO with typed arguments and a typed result.

- **Generated from it** by the macros: serde, the JSON Schema of the arguments and of the result, the clap arguments
  and the command catalog.
- **Built on that catalog:** the CLI and the MCP server.
- **From Rust:** commands can be called typed, or by name with JSON arguments. See
  [docs/commands.md](docs/commands.md).

## polarion

The library the CLI and MCP server build on.

- **SOAP, not REST:** it uses the SOAP services, so it works with ordinary user permissions (see
  [polarion-soap](#polarion-soap) for why).
- **Compact JSON:** it returns flat domain types that serialize to compact JSON.
  - Enum options, users and linked work items are reduced to their ids.
  - Custom fields become a `key → value` map.
  - Empty fields are left out.
- **The whole SOAP API:** every one of the 302 operations is reachable through a library method.
  - Variants such as `…WithFields`, `…Limited`, `…InBaseline` or `…BySQL` are options of one method.
  - `tests/coverage.rs` enforces this. The few operations it excludes (login/logout, deprecated duplicates) are
    listed there with the reason.

Operations are grouped by area:

| Handle | What it covers |
|---|---|
| `items()` | **Reads:** get (by id/URI, fields, revision), Lucene queries (limit, baseline, revision), SQL, count, back links, history, revisions, workflow actions and transitions, field options, custom fields, attachments, localized texts. **Writes:** create, update, delete, move, comments (incl. restricted visibility), links of every kind, assignees, categories, approvals, attachments, work records, planning constraints. **Markdown** (feature `markdown`): read, set and append descriptions, with mermaid diagrams as SVG attachments |
| `documents()` | **Reads:** spaces, list, get (with or without body), Lucene/SQL queries (baseline), work items in document order. **Writes:** create, update, delete, comments, reuse and derived documents, title heading, workflow. PDF export |
| `schema()` | Custom field definitions and keys, enum options (by field, enum id, control value, object), control keys, initial workflow actions, categories, time points, languages, duration settings |
| `tests()` | **Reads:** test runs and templates (get, search), records, steps, step configuration, test configuration, parameters, custom field definitions, report page. **Writes:** create, update, records, execute, steps, attachments, summary defects, project span |
| `plans()` | **Reads:** plans and templates (get, search), statistics, report page. **Writes:** create, update, delete, plan items, allowed types, project span |
| `wiki()` | Spaces, pages, Lucene/SQL queries (baseline) |
| `baselines()`, `revisions()` | Baselines (query, create, update, delete); revisions (get, query, linked work items) |
| `activities()` | Activity streams (all, project, group), sources, comments |
| `folders()` | Document & page folders, project or global |
| `projects()`, `users()` | Projects and project groups; users (get, list, avatar, create, update) |
| `security()` | Permission checks, roles, location of objects, token owner, licenses, account vault |
| `Polarion` | Login/logout, session check, transactions (`begin_transaction`, `commit`, `rollback`); writes of several requests run as one transaction in a session of their own on their own |

Result types:

- Most results are flat domain types (`WorkItem`, `Document`, `TestRun`, `Plan`, ...).
- Configuration-like results (test configuration, licenses, builds, activities, folders, ...) are the generated
  SOAP types. They implement `serde::Serialize` with the `serde` feature of `polarion-soap`, which this crate
  enables.

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

The raw SOAP client stays available through `Polarion::soap()`. To try the library against a real server
(read-only, about twenty small requests, prints the JSON of each result):

```sh
POLARION_URL=https://polarion.example.com/polarion POLARION_TOKEN=... POLARION_PROJECT=MyProject \
  cargo run -p polarion --example smoke
```

### Writes

Writes are enabled per project, or only for listed objects.

**Per project:**

- Every write method checks the target's project against the allowlist. Outside it, the method fails with
  `Error::WriteNotAllowed` before anything is sent.
- A client built with `Polarion::new` or `Polarion::from_client` writes nothing.
- Writes not tied to a project additionally need `.allow_global_writes(true)`: creating and updating users, global
  folders, comments on activities, the account vault.
- `Polarion::soap()` is the raw client and is not checked against the allowlist. In restricted mode its writes pass
  the write filter.
- **Several requests, one transaction:** a write that needs more than one request (a create with follow-ups such
  as categories, assignees or links; a document, test run or plan create with its fields; an update of standard
  and custom fields) runs them as one transaction, all or nothing, in a Polarion session of its own that the
  library opens and ends. A Markdown description with new diagrams takes two: the uploads, then the update and
  the deletes (Polarion lists an attachment uploaded in a transaction only once it is committed). A commit without an answer is
  `Error::CommitUncertain`. A call dropped after it started still finishes (commit or rollback, then logout). See
  [Writes of several requests](docs/library.md#writes-of-several-requests).

```rust
use polarion::{Auth, FieldValue, ItemRef, NewLink, NewWorkItem, Polarion, WorkItemUpdate};

let polarion = Polarion::builder("https://polarion.example.com/polarion", Auth::Token("<token>".into()))
    .allow_writes(["Sandbox"])
    .build()?;
let items = polarion.items();

let mut new = NewWorkItem::new("Sandbox", "requirement", "Brakes must work");
new.custom_fields.insert("asil".into(), FieldValue::Enum("D".into()));
// Fields createWorkItem cannot carry, e.g. what the project's workflow requires on create:
// sent right after the create, as one transaction in a Polarion session of its own.
new.categories = vec!["software".into()];
new.links = vec![NewLink::new("parent", ItemRef::new("Sandbox", "SB-1"))];
let item = items.create(&new).await?;

items.update(&item, &WorkItemUpdate { title: Some("Brakes must always work".into()), ..Default::default() }).await?;
items.add_link(&item, "relates_to", &ItemRef::new("Sandbox", "SB-1")).await?;
items.transition(&item, "reviewed").await?;   // status changes go through workflow actions
```

**Restricted to write targets (restricted mode):**

- **Turned on by** either of:
  - `.restrict_writes([uri, ...])`: writes to those objects only, by URI (e.g. the `uri` of fetched work items,
    documents and test runs);
  - `.allow_creates(project, CreateBudget { .. })`: a fixed number of creates in one project.
- **Cannot be widened:** `allow_writes` and `allow_global_writes` have no effect on it.
- **Named objects:** every object a write names must be a target.
- **Always refused:** writes that may change other objects or cannot be undone (workflow actions, deletes of whole
  objects, moves, reuse, test executions, ...), and `status`/`resolution` changes.
- **Documents:** a document's structure link role and work item macros are never written; its body only when the
  document holds no work items.
- **Ask first:** `Polarion::write_allowed(uri)` tells, without sending anything, whether an object may be written.
- **Write filter:** on top of the library's guards, the SOAP client gets a filter that parses every write request
  and refuses any that names an object other than a target. That covers requests sent through `Polarion::soap()`
  and writes repeated after a session renewal.
- **Not prevented:** what the server does in response to a write to a target (suspect links on work items that link
  to it, notifications).
- **More:** `PolarionBuilder::restrict_writes` and [the write guard](docs/library.md#the-write-guard). The
  [live suite](docs/testing.md#the-live-suite-fixtures-on-a-real-server) uses restricted mode to run the writes
  against a real server, on its own fixtures only.

**Helpers:**

- **Markdown descriptions:** with the feature `markdown` (plus `mermaid` to render diagrams, `highlight` for
  highlighted code), `items().description_markdown`, `set_description_markdown` and `append_description_markdown`
  read and write descriptions as Markdown through [`polarion-markdown`](#polarion-markdown). The crate is
  re-exported as `polarion::polarion_markdown`. `MarkdownWriteOptions::images` attaches images of the Markdown
  (given as bytes) and shows them inline; `description_markdown_export` returns the Markdown with the images the
  description shows.
- **Typed custom fields:** `FieldValue::coerce(&json, &field_definition)` types JSON input (from a CLI or an agent)
  for a custom field, using its definition from `schema()`.

#### What has been checked against a real server

<details>
<summary><b>Reads</b>: found and confirmed against a production server (smoke example and live read tier)</summary>

The smoke example (`cargo run -p polarion --example smoke`) and the live read tier confirmed:

- **Document URIs** separate space and name with `#` (`...PROJECT${Module}{moduleFolder}SPACE#NAME`); the server
  cannot resolve `SPACE/NAME`. Work item URIs are `...PROJECT${WorkItem}ID`.
- **Field keys** are Polarion's field ids: `module` and `plannedIn` for work items, `linkedPages` for wiki pages.
  The server refuses the SOAP element names `moduleURI`, `plannedInURIs` and `linkedPageURIs`. The key
  `customFields` alone returns no custom fields; they need `customFields.<key>`.
- **Test run URIs** are `...PROJECT${TestRun}ID` (`uri::test_run_uri`). Comments and work records have URIs of
  their own (`uri::owner_of` reads their owner):
  - `...PROJECT${Comment}{WorkItem}ID#KEY`
  - `...PROJECT${WorkRecord}{WorkItem}ID#KEY`
  - `...PROJECT${ModuleComment}{Module}{moduleFolder}SPACE#NAME#KEY`
- **Session renewal:** after the server ended the session (`endSession`), the next read fails, `hasSubject` reports
  the session gone, and the client logs in again and repeats the read.
- **Queries** with date ranges (`updated:[20261009 TO 99991231]`, also `TO *`), and `getRevisions` for documents
  and test runs.

</details>

<details>
<summary><b>Writes</b>: confirmed by the live suite on its fixtures (live write tier, CLI and MCP tiers, bootstrap)</summary>

The live write tier, the CLI and MCP tiers and the bootstrap (see [docs/testing.md](docs/testing.md)) confirmed:

- **`createWorkItem`** stores the custom fields given at creation (boolean, date, enum, float, string, rich text).
  But the type's template work item, if the project configures one, replaces the description and can clear other
  fields (here an `int`).
- **`setCustomField`** with every value type the project has (string, rich text as `types:Text`, boolean, enum,
  date, `xsd:float`, `int`), and `setFieldsNull` clearing a custom field by its key. A boolean `false` reads back
  as no value.
- **`updateWorkItem`** (title, description), hyperlinks, categories, links between work items (and the target's back
  link), assignees and approvees, localized titles, transactions (a rollback discards the change, a commit keeps
  it).
- **Attachments:** `getWorkItemByIdsWithFields` with `attachments` lists a just-created attachment with `id`
  (`<counter>-<fileName>`) and `fileName`; `getAttachment` returns it unchanged; update and delete.
- **Markdown descriptions:** Polarion keeps the HTML written through the API exactly. Checked: a heading, bold,
  italic, `text-decoration` spans, highlight, `<sup>`/`<sub>`, inline code, formula spans, a link, a work item link
  span, a list, an alert, a table, highlighted code as `<pre class="language-…">` with styled spans, a
  `workitemimg:` image. The uploaded SVG comes back unchanged, and the description reads back as the same Markdown.
  Writing the same text again sends nothing. A removed diagram's attachment is pruned.
- **Mermaid diagrams** (SVG attachments shown through `workitemimg:`) display in the browser and in PDF and Word
  exports (checked by hand).
- **Test management:** test steps (`setTestSteps`), test records (`addTestRecordToTestRun`,
  `updateTestRecordAtIndex`, and removing one with an empty record), test run fields and report, run, record and
  step attachments. The server stores record and step attachments as `<test case id>_<file name>` and
  `<test case id>_<step>_<file name>`.
- **`updateModule`** with a changed body (the document's work items unchanged), **`updateTestRun`** with a changed
  title.
- **Comments:** `addComment` on a work item and a document, a reply, `setResolvedComment`, `setCommentTags`, with
  URIs `...${Comment}{WorkItem}ID#KEY` and `...${ModuleComment}{Module}{moduleFolder}SPACE#NAME#KEY`.
- **Write targets:** the CLI and the MCP server with `POLARION_WRITE_TARGETS`. Writes to the target go through;
  writes to anything else are refused before they are sent.

</details>

**Not yet verified against a real server.** These are tested against a mock server only:

- Currency custom fields sent as `xsd:decimal` (the project has none), multi-user custom fields, planning
  constraints (their options could not be read).
- Test records naming the executing user, by URI (`subterra:data-service:objects:/default/${User}ID`). Work records
  name the user the same way; one was booked by hand against a real server, but restricted mode treats the user as
  a write target, so the live suite (restricted to its fixtures) skips them.
- Update operations for plans, and every write the live suite never makes (workflow actions, deletes, document
  structure, plans, baselines, users).
- A write repeated after a session renewal: only after a SOAP fault (the server refused it), never after an HTTP
  error, and through the write filter again (a create beyond the budget is not repeated).
- Creates with follow-ups for every kind (work items, documents, test runs, plans): the live suite creates nothing.
  A task with a category was created by hand (begin transaction, `createWorkItem`, `addCategory`, commit in one
  session) against a real server whose workflow requires the category. The live suite covers updates of several
  requests in a session of their own.
- Images of Markdown descriptions: attaching the images of a Markdown file (`--file`,
  `MarkdownWriteOptions::images`), reusing an attachment with the same content, and
  `items export-description`.
- A commit without an answer (`CommitUncertain`), and the cleanup of a session of its own after a cancelled call
  (it finishes in a spawned task) or a runtime shutdown (the server times it out).

## polarion-markdown

Markdown ↔ Polarion rich text, without I/O. The mapping, the lossy cases and the security measures:
[docs/markdown.md](docs/markdown.md).

**Markdown → HTML:** `to_html(markdown, &ToHtmlOptions)` writes the inline-styled HTML of Polarion's rich text
editor:

- headings as sized bold spans
- Polarion's table styles
- `PREFIX-123` ids as work item links (`WorkItemIds::Any`, `Prefixes([...])` or `Off`)
- `$…$` formulas
- highlighted code (feature `highlight`)

**HTML → Markdown:** `to_markdown(html, &ToMarkdownOptions)` reads such HTML back. Formatting Markdown cannot express
stays HTML (`RawHtml::Keep`, the default) or is dropped (`RawHtml::Drop`).

**Mermaid blocks** (`mermaid` module; rendering needs the feature `mermaid`):

- **Naming:** `mermaid::diagrams(markdown)` lists them, each named `mermaid-<hash>.svg` after a hash of its
  normalized source, so an unchanged diagram keeps its attachment.
- **Rendering:** `mermaid::render_svg` renders one locally (mermaid-rs-renderer), with the source embedded in the
  SVG.
- **Round trip:** that embedded source is how `mermaid::source_of` and `to_markdown_with_diagrams` restore the
  block. `to_html_with_diagrams` shows a diagram as an image of its attachment.
- **Behaviour:** rendering is synchronous, and its text measurement goes through a mutex all renders of a process
  share. Its panics are caught (and printed by the panic hook). Its layouts differ from mermaid.js.

**Helpers** the library's description methods use: `append_html`, `text_to_html`, `diagram_refs`,
`attachment_file_name`, and `attachment_refs` (every attachment an HTML shows).

## polarion-soap

Everything here is built on Polarion's SOAP web services, not its REST API:

- **Permissions:** SOAP follows the normal project permissions. The REST API needs extra role permissions that by
  default only `admin` and `project_admin` have; every other user gets `403 Forbidden`.
- **Coverage:** SOAP covers things the 2410 REST API lacks, such as listing documents, SQL queries, baselines and
  wiki pages.

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

| Feature | What it does |
|---|---|
| Coverage | All 302 operations of the 7 services (Session, Project, Tracker, TestManagement, Planning, Security, Builder), with Javadoc from the WSDLs as method docs |
| Session | Logs in lazily (token or user/password), shares one session across services and concurrent calls, and logs in again when the session expires (except inside a transaction). A failed read is then repeated; a failed write only when the server answered with a fault, never after an HTTP error (it may have been applied). `ClientBuilder` is `Clone`: each built client has a session of its own; `transaction_only` clients log in once and write only inside their transaction |
| Untyped values | Custom field values and other untyped elements are `AnyValue`, decoded from their `xsi:type`; unknown types are kept as raw XML |
| Errors | SOAP faults surface with their code and message (`Error::Fault`) |
| Read-only mode | `read_only(true)` refuses every operation that is not a read before anything is sent |
| Write filter | `write_filter(Arc::new(\|operation, body\| ...))` checks the request of every operation that is not a read before it is sent (and again before a repeat), and refuses it with `Error::WriteFiltered` when the filter says no. The `polarion` library's restricted mode uses it |

How the session, renewal and read-only mode work: [docs/soap-client.md](docs/soap-client.md).

To try it against a real server (read-only, at most seven small requests, each reported as OK or FAIL):

```sh
POLARION_URL=https://polarion.example.com/polarion POLARION_TOKEN=... POLARION_PROJECT=MyProject \
  cargo run -p polarion-soap --example soap_smoke
```

## Code generation

- **Source:** the SOAP client is generated from `spec/soap/*.wsdl` (Polarion 2410) by the generator in
  [`xtask/`](xtask).
- **Committed:** the generated code is checked in, and CI fails if it is stale.
- **More:** how the generator works and how to target another Polarion version:
  [docs/code-generation.md](docs/code-generation.md).

```sh
cargo xtask codegen          # regenerate
cargo xtask codegen --check  # verify it is up to date
```

Tests, including the opt-in live suite, which writes only to its own fixtures: [docs/testing.md](docs/testing.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
