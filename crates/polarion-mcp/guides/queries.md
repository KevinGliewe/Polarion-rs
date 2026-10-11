# Queries

Searches take a query in Polarion's Lucene syntax. The tools: `query_work_items`, `count_work_items`,
`search_documents`, `search_test_runs`, `search_plans` and `query_wiki_pages`; through `polarion_call` also the
variants that return only references (`items.query-refs`, `documents.query-refs`, `wiki.query-uris`), the
template searches (`tests.search-templates`, `plans.search-templates`) and SQL (`items.query-sql`,
`documents.query-sql`, `wiki.query-sql`: `SELECT ... FROM ...`).

## Syntax

- `field:value` matches a field: `type:requirement`, `status:open`. Several terms are combined with `AND`, `OR`
  and `NOT` (upper case) and grouped with parentheses:
  `type:requirement AND (status:open OR status:draft) AND NOT severity:minor`.
- Values with spaces or special characters go in double quotes: `title:"brake pedal"`.
- Ranges use square brackets and `TO`; dates are written `YYYYMMDD`:
  `updated:[20260101 TO 20261231]`, open-ended `created:[20260101 TO *]`.
- `HAS_VALUE:field` matches items whose field is set, e.g. `type:defect AND NOT HAS_VALUE:resolution`.
- An empty query matches everything (in the project, see below).

The values are ids, not display names: type ids, status ids, user ids. The section "Project" at the end of this
guide lists the type and status ids of the project; `field_options` lists the options of any enum field
(`priority`, `severity`, `resolution`, an enum custom field).

## The project

The server restricts every query to the default project or to the command's `project` argument: it sends
`project.id:PROJECT AND (your query)`. Do not add `project.id` yourself. Without a default project and without
`project`, a query searches every project the account can read.

## Sorting

`sort` takes field ids, `~` in front for descending: `~updated` (newest first), `status id` (by status, then
id). The default is `id`. `~` means "descending" only in `sort`; inside a query it is Lucene's fuzzy operator.

## How many results

- At most 100 results by default. `limit: N` changes the limit; `all: true` returns every match (mind the load
  on the server for large projects).
- When exactly `limit` results come back there may be more: count first with `count_work_items`, or raise the
  limit.
- `count_work_items` takes the same query and returns the number only.

## Which fields come back

- Work item searches return the standard fields unless `fields` says otherwise: id, title, type, status,
  priority, severity, resolution, author, assignee, created, updated, dueDate, outlineNumber, project and module
  (the document).
- `fields` takes `standard`, `all` (every built-in field, description and links included) or a list of field
  ids, e.g. `["id", "title", "description", "linkedWorkItems"]`.
- Field ids are Polarion's ids: `module` (the document) and `plannedIn` for work items, `linkedPages` for wiki
  pages. The SOAP element names `moduleURI`, `plannedInURIs` and `linkedPageURIs` are refused by the server.
- Custom fields come back only when named: `custom: ["asil", "reviewed"]` (sent as `customFields.asil`, ...).
  The key `customFields` alone returns none of them, not even with `fields: "all"`.
- Fewer fields make faster queries; fetch the description only when you need it.

## Baselines and revisions

- `baseline` searches the index as it was in a baseline revision (work items, documents and wiki pages).
- `revision` (work items only) searches the current state but returns the work items as they were in that
  revision.

## Pitfalls

- Lower-case `and`, `or`, `not` are search terms, not operators.
- Dates in ranges are `YYYYMMDD`, without dashes; field values in writes are `YYYY-MM-DD`.
- A query names field ids and option ids: use the option's id from `field_options` (or the section
  "Project"), not its display name.
- Custom fields have to be requested (`custom`) to appear in the results, even when the query filters on them.
- A single work item by id is cheaper with `get_work_item` than with a query.
- Results stop silently at the limit; a list of exactly 100 items is a hint that there are more.

## Examples

| Goal | Query and options |
|---|---|
| Open requirements, newest first | `type:requirement AND status:open`, `sort: "~updated"` |
| Changed this year | `updated:[20260101 TO *]` |
| Defects without a resolution | `type:defect AND NOT HAS_VALUE:resolution` |
| How many tasks | `count_work_items` with `type:task` |
| With a custom field | `type:requirement`, `custom: ["asil"]` |
| Test runs still open | `search_test_runs` with `status:open` |
