# Documents

Polarion documents (live documents, "modules" in the SOAP API) hold a body of rich text and, embedded in it,
work items in document order. They live in spaces (folders) of a project.

## Spaces, locations and URIs

- A document is `SPACE/NAME` in the default project or `PROJECT/SPACE/NAME`, e.g. `Specs/SRS`. The default
  space is `_default` (`_default/Notes`).
- Its URI separates space and name with `#`:
  `subterra:data-service:objects:/default/MP${Module}{moduleFolder}Specs#SRS`. A URI with `Specs/SRS` does
  not resolve. Results carry the `uri`; reuse it.
- `documents.spaces` lists the spaces of a project, `documents.sub-folders` the folders under a location of the
  documents folder.

## Reading

- `list_documents`: the documents of a project, or of one `space`, sorted by space and name, without bodies.
  With `space` and `full`, all fields including the bodies.
- `get_document`: one document's fields (title, type, status, author, structure link role, allowed types,
  outline numbering, comments, custom fields, `derivedFrom`); with `body`, the body too (Polarion HTML, can be
  large).
- `document_work_items`: the work items of a document in document order; `parent` for the items under one item,
  `deep: false` for one level only, `fields` and `custom` as in queries. `documents.work-item-refs` returns
  references only.
- `search_documents`: Lucene search over documents (see `queries`), without bodies unless `fields` asks.
- `documents.actions`: workflow actions available for the document.

## Work items in a document

- Create a work item inside a document with `create_work_item` and `document` (plus `parent` for its place in
  the structure). `items.move-to-document` moves an existing one into or within a document.
- The structure (which item is under which) is made of links with the document's structure link role, usually
  `parent`.
- A work item's `document` field (field id `module`) names its document; `outlineNumber` its number there.
- Change the work items themselves with the work item tools (see `work-items`), not through the body.

## Creating and changing documents

- `create_document`: `name`, optionally `space` (default `_default`), `title`, `allowed-types` (the work item
  types the document may hold), `structure-link-role` (default `parent`) and initial `content` (HTML).
- `update_document`: `title`, `type`, `status`, `content` (the whole body), `structure-link-role`,
  `outline-numbering`, `custom-fields` (typed from JSON; `{"enum": "id"}` for an enum option).
- `documents.update-title-heading` sets the first title heading of the body.
- Status changes go through the workflow: `documents.transition` (target status) or `documents.perform-action`.
- `documents.reuse` creates a new document from an existing one; `documents.update-derived` brings a derived
  document up to a revision of its source.
- `documents.delete` deletes the document.

## The body and its limits

- The body is Polarion HTML in which work items appear as placeholders. `content` in `update_document` replaces
  the whole body: read it first (`get_document` with `body`) and change only what you mean to.
- Changing the body of a document that holds work items can change those work items (their placeholders are
  part of it). Servers restricted to write targets therefore refuse body and outline numbering changes of such
  documents, and never change the structure link role or write a body with work item placeholders.
- There is no Markdown for document bodies; Markdown is for work item descriptions only.

## Comments

- `add_document_comment`: `text` (a string is plain text, or `{"type": "text/html", "content": "..."}`) and
  optionally `about`, a work item of the document the comment refers to. It returns the comment's URI
  (`...MP${ModuleComment}{Module}{moduleFolder}Specs#SRS#KEY`).
- `documents.reply-to-comment` answers one. `get_document` returns the comments.
- Restricted servers refuse comments that refer to a work item.

## PDF export

- `export_document_pdf`: `document`, optionally `paper-size` (default A4), `orientation` (default portrait),
  `fit-to-page-width`, `bookmarks`, `header-footer`, `repeat-table-headers`.
- The result is file content: its size and type, or with an exchange folder saved there with `save_as`
  (`"srs.pdf"`). The PDF is never returned as base64.

## Not available

Documents have no attachment operations in the SOAP API.
