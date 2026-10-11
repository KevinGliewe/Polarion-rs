# Polarion MCP server: overview

This server reaches one Siemens Polarion ALM server through its SOAP web services, with the permissions of the
account it runs as. Every Polarion operation is a command named `area.name` (`items.get`, `documents.export-pdf`).

## Tools

- **Curated tools** for everyday work, by area: work items (`get_work_item`, `query_work_items`,
  `count_work_items`, `work_item_links`, `work_item_history`, `get_work_item_description_markdown`), documents
  (`list_documents`, `get_document`, `document_work_items`, `search_documents`, `export_document_pdf`), schema
  (`custom_fields`, `field_options`), test runs (`search_test_runs`, `get_test_run`, `test_case_steps`,
  `test_run_records`, `test_run_report`), plans (`search_plans`, `get_plan`, `plan_statistics`), wiki
  (`query_wiki_pages`, `get_wiki_page`), `get_project` and `get_user`. With writes enabled, the everyday writes
  have tools too (`create_work_item`, `update_work_item`, `set_work_item_description_markdown`,
  `create_document`, `add_test_record`, `create_plan`, ...).
- **Generic tools:**
  - `polarion_guide(area)`: these guides, with a section read from the project (types, statuses, link roles).
  - `polarion_commands`: the areas, or with `area` the commands of one area (name, summary, `write`).
  - `polarion_describe(command)`: a command's description, write flag and input and output schemas.
  - `polarion_call(command, args)`: runs any command, e.g.
    `{"command": "items.back-links", "args": {"item": "MP-42"}}`.
- A command without a curated tool may have a tool named `area_name` (`items_add_hyperlink`) if the server lists
  it; `polarion_call` reaches every command that is not disabled.
- Argument names are kebab-case: `custom-fields`, `test-case`, `file-name`, `due-date`.

## Areas and guides

| Commands | Guide |
|---|---|
| `items.*` | `work-items` |
| `documents.*` | `documents` |
| `tests.*` | `tests` |
| `plans.*` | `plans` |
| `wiki.*` | `wiki` |
| `schema`, `projects`, `users`, `baselines`, `revisions`, `activities`, `folders`, `security`, `builds`, `session` | `overview` |

`queries` explains the Lucene query syntax every search tool takes.

## Referring to objects

- Work items: `ID` in the default project (`MP-42`), `PROJECT/ID` (`OP/OP-7`), or a URI
  (`subterra:data-service:objects:/default/MP${WorkItem}MP-42`).
- Documents: `SPACE/NAME` in the default project, `PROJECT/SPACE/NAME`, or a URI. In URIs space and name are
  separated by `#`: `...MP${Module}{moduleFolder}Specs#SRS`.
- Test runs and plans: most commands take the URI (`...MP${TestRun}RUN-1`, `...MP${Plan}rel1`); `get_test_run`
  and `get_plan` take an id. Results carry the `uri` of each object; reuse it.
- Commands that take `project` use the default project of the server when it is omitted.

## Results

- The library's JSON: flat, camelCase, empty fields left out. Enum options, users and linked work items are
  reduced to their ids; custom fields are a `customFields` map.
- Tools with an output schema also return structured content. Results that are not objects (lists, counts,
  flags) are wrapped as `{"result": ...}`.
- Queries return at most 100 results unless `limit` or `all` is given (see `queries`).
- File content (attachments, PDF exports) never comes back as base64: images as image content, UTF-8 text as
  text, anything else as its size and type. With an exchange folder, `save_as` saves a download there and
  `data-file` uploads a file from there (plain file names only).

## Writes and their limits

- The server is read-only unless it was started with write projects (`POLARION_WRITE_PROJECTS`), global writes
  (`POLARION_ALLOW_GLOBAL_WRITES`, for writes not tied to a project) or write targets
  (`POLARION_WRITE_TARGETS`). The instructions sent at connection say which applies.
- Commands that modify data are marked `write` (`polarion_commands`, `polarion_describe`) and their tools carry
  `readOnlyHint: false`. A refused write sends nothing to Polarion.
- **Restricted mode** (write targets): only the listed objects may be written, and every object a write names
  (link targets, test cases, defects, plan items) must be a target too. Refused even for the targets: creates,
  deletes of whole objects, workflow actions, status and resolution changes, a document's structure link role,
  and the body of a document that holds work items. Allowed: their fields, attachments, work records, comments
  and test records. What Polarion does in response (suspect links on work items that link to a target,
  notifications) is not prevented.
- **Guides first:** this server may refuse an area's writes, or all its commands, until that area's guide was
  read (`error (guideRequired)`, naming the area). Call `polarion_guide` with that area, then repeat the call.

## Errors

Failures are tool results with `isError` and the text `error (code): message`:

| Code | Meaning |
|---|---|
| `readOnly` | the write scope does not cover the write; nothing was sent. Do not look for another way around it |
| `guideRequired` | read the named guide with `polarion_guide`, then repeat the call |
| `invalid` | bad arguments (unknown argument, wrong type, unknown or disabled command) |
| `notFound` | the object was not found |
| `fault` | Polarion refused the request; the message is Polarion's |
| `login`, `sessionExpired`, `http`, `config` | the connection or the server's settings: report them to the user |

- A disabled command is refused by every tool ("is disabled on this server") and not listed by
  `polarion_commands`.
- Over HTTP all clients share one Polarion session, so transactions (`session.begin-transaction`, `commit`,
  `rollback`) are refused there.

## Habits that work

- Look at the project's configuration before writing: the section "Project" of a guide, `field_options` (types,
  statuses, priorities, enum fields), `custom_fields` (the custom fields of a type).
- Fetch only the fields you need (`fields`, `custom`) and count before fetching large result sets.
- Check `work_item_links` (incoming links too) before changing a work item others depend on.
