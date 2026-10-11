# Wiki

Wiki pages live in spaces of a project. Through the SOAP API they can be read and searched, not written: this
server has no command that changes a wiki page.

## Spaces and pages

- `wiki.spaces`: the wiki spaces of a project (the default project unless `project` is given).
- `wiki.pages`: the pages of a `space` with all fields, or only `fields`; `wiki.page-uris` their URIs only.
- `get_wiki_page`: one page by its `uri` (all fields, or only `fields`). Take the URI from `wiki.page-uris`,
  `wiki.pages` or a search.

## Searching

- `query_wiki_pages`: Lucene search over wiki pages (see `queries`); `wiki.query-uris` returns URIs only.
- `wiki.query-sql` and `wiki.query-uris-sql`: SQL (`SELECT ... FROM ...`).
- Searches take `baseline` to search the pages as they were in a baseline revision.

## Fields

- Searches without `fields` return every field but the page content: id, title, type, space (`spaceId`),
  page name and location, author, created, updated, updatedBy, attachments (listed only) and `linkedPages`.
- Field ids are Polarion's: `linkedPages`, not the SOAP element name `linkedPageURIs`, which the server
  refuses.
- Ask for the content only for the pages you read; it can be large.

## Not available

- No writes: pages cannot be created, changed or deleted through this server.
- Wiki pages have no attachment operations in the SOAP API (their attachments are only listed).
- There is no Markdown conversion for wiki pages; their content comes back as Polarion stores it.
