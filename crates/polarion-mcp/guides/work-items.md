# Work items

Requirements, tasks, defects, test cases: everything with an id like `MP-42` is a work item. Refer to one as `ID`
in the default project, `PROJECT/ID`, or its URI.

## Reading

- `get_work_item`: all built-in fields, outgoing links, comments, attachments, approvals, work records. Pass
  `fields` (`standard`, `all`, or field ids) and `custom` (custom field keys) to get less, or the custom fields
  you need.
- `query_work_items`, `count_work_items`: Lucene searches (see the `queries` guide).
- `work_item_links`: links in both directions, `outgoing` from the item and `incoming` from others (each link's
  `item` is the other end); `items.back-links` only the incoming ones.
- `work_item_history`: the changes of the work item; `items.revisions` the repository revisions.
- In results, `type`, `status`, `priority`, ... are option ids, `assignees` and `author` user ids,
  `document` the containing document, `plannedIn` plan URIs, `customFields` a `key: value` map.

## Fields and custom fields

- Built-in fields have fixed keys: `title`, `description`, `priority`, `severity`, `resolution`, `dueDate`,
  `initialEstimate`, `remainingEstimate`, `timeSpent`, `assignee`, `categories`, `linkedWorkItems`,
  `hyperlinks`, `module` (the document), `plannedIn`, ...
- Enum fields take option ids. `field_options` lists the options of a field in a project (`type`, `status`,
  `priority`, `severity`, `resolution`, an enum custom field); `items.field-options` lists those one work item
  can take.
- `custom_fields` lists the custom field definitions of a type (key, value type, enum id, multi, required);
  `items.custom-field-keys` the keys of one work item. The section "Project" below lists the keys by type.
- Custom field values are typed by their definitions: strings; numbers for `int`, `float` and `currency`;
  `true`/`false`; option ids for enums (an array for multi-valued fields); dates `YYYY-MM-DD`, date-times ISO
  8601, times `HH:MM:SS`; rich text a string (plain text) or `{"type": "text/html", "content": "..."}`.
  `null` clears a field. A boolean `false` reads back as no value.

## Creating and changing

- `create_work_item`: `type`, `title`, optionally `description`, `priority`, `severity`, `due-date`,
  estimates, `custom-fields`, and `document` (+ `parent`) to create it inside a document. It returns the new
  item and its URI. A type's template work item, if the project configures one, can replace the description
  and clear other fields: read the item back after creating it.
- Fields Polarion's create request cannot carry are arguments of `create_work_item` too: `categories`,
  `assignees`, `approvees`, `links` (`ROLE=ITEM`, `ROLE=ITEM@REVISION` or `{role, item, revision, suspect}`),
  `hyperlinks` (`ROLE=URL`), `planning-constraints` (`CONSTRAINT=DATE`), `planned-in` (plans), `test-steps`,
  `external-links`, `oslc-links` (`ROLE=LABEL=URL`) and `linked-revisions`. They are written right after the
  create, in the same transaction: give here what the project's workflow requires when an item is created (an
  error like "Workflow initialization failed ... required field 'categories' ... is empty" names it). The create
  and these writes succeed or fail together; a failed call leaves no work item behind.
- `commitUncertain`: the writes were sent but the commit got no answer. Look for the item (e.g. by title)
  before you create it again.
- `update_work_item`: `title`, `description`, `priority`, `severity`, `resolution`, `due-date`, estimates,
  `time-spent`, `custom-fields` (`null` clears one) and `clear` (built-in field keys to empty). Fields not given
  stay as they are.
- `items.set-custom-field` sets or clears one custom field; `items.clear-fields` empties fields by key.
- Assignees, categories and approvals of an existing item have commands of their own: `items.add-assignee`,
  `items.add-category`, `items.add-approvee`, `items.set-approval`, and their `remove-` counterparts.
- An update that needs several requests (standard fields, custom fields and cleared fields together) is one
  transaction as well: all or nothing.

## Status: only through the workflow

`update_work_item` has no `status`. A status changes through a workflow action:

1. `items.actions` lists the actions available now (`items.unavailable-actions` the others, with the reason).
2. `items.transition` with the target `status` performs the action that leads there, or
   `items.perform-action` with an action id.

Servers restricted to write targets refuse workflow actions and status and resolution changes.

## Links

- `add_work_item_link` and `remove_work_item_link`: `item`, `role`, `target`. The link goes from `item` to
  `target`; Polarion shows it as a back link on the target. Role ids come from the project (section "Project"
  below), for example `parent` or `relates_to`.
- `items.add-link-at-revision` pins the link to a revision and can mark it suspect.
- Hyperlinks: `items.add-hyperlink` (`url`, `role`) and `items.remove-hyperlink`; the hyperlink roles are the
  options of the field `hyperlinks` (`items.field-options` with key `hyperlinks`).
- Changing a work item can mark the links of other items to it suspect; `items.auto-suspect` does so
  explicitly. Look at its incoming links before changing an item others depend on.

## Comments

- `add_work_item_comment`: `text` (a string is plain text; `{"type": "text/html", "content": "..."}` is rich
  text) and an optional `title`. It returns the comment's URI (`...MP${Comment}{WorkItem}MP-42#KEY`).
- `items.reply-to-comment` (the comment's URI), `items.set-comment-resolved`, `items.set-comment-tags`,
  `items.add-restricted-comment` (visible to some users or roles only).
- `get_work_item` returns the comments.

## Attachments

- `add_work_item_attachment`: `file-name`, `title`, `data` (base64) or, with an exchange folder, `data-file`.
- `get_work_item` lists the attachments; their ids look like `<counter>-<file name>` (`1-spec.pdf`).
- `items.attachment` downloads one: images come back as images, text as text, other files as size and type
  unless saved with `save_as`. `items.update-attachment`, `items.delete-attachment`.

## Descriptions: prefer Markdown

The description is Polarion rich text (HTML). Read and write it as Markdown instead of raw HTML:

- `get_work_item_description_markdown` reads it; `set_work_item_description_markdown` replaces it;
  `append_work_item_description_markdown` adds a paragraph at the end (it reads, extends and writes back, so a
  change someone makes in between is lost).
- Markdown becomes the HTML of Polarion's editor: headings as bold sized text, Polarion's table style, code
  blocks (highlighted when they name a language), `**bold**`, `*italic*`, `~~strike~~`, `++underline++`,
  `==highlight==`, `^sup^`, `~sub~`, inline code, links, images, task lists, alerts (`> [!NOTE]`), formulas
  (`$x$`, `$$x$$`).
- Mermaid code blocks are rendered to SVG, attached as `mermaid-<hash>.svg` and shown as images; an unchanged
  diagram is not uploaded again, and a diagram the new text no longer shows is deleted (unless `keep-orphans`).
  A diagram that cannot be rendered fails the write before anything is written, unless `allow-unrendered` (then it
  stays a code block). Reading gives the mermaid code blocks back.
- Ids with the work item's own prefix (`MP-7` in `MP-42`) become work item links; `id-prefixes` names other
  prefixes (`*` any, `none` no links). Write `MP\-7` for plain text.
- Formatting Markdown cannot express (colors, fonts, alignment, merged cells) is read as inline HTML; keep it
  when you write the text back, and the formatting stays. `drop-html` leaves it out when reading.
- Writing the same text again changes nothing.
- With an exchange folder, `file` (instead of `markdown`) reads the Markdown from a file in that folder and
  attaches the images it names by relative paths (`![](img/a.png)`: PNG, JPEG, GIF, WebP or SVG, in the file's
  folder or below it), so they are shown inline. An image the work item already has an attachment with the same
  content for is not uploaded again; image attachments are never deleted.
- `items.export-description` (`item`, `dir`) saves the description as `<dir>/<ID>.md` in the exchange folder,
  with the images it shows as `<dir>/<ID>/<attachment id>`; `file` writes that Markdown back.
- Use `update_work_item` with `description` only for exact HTML.

## Deleting and moving

`items.delete` deletes a work item; `items.move-to-document` moves it into or within a document.
Servers restricted to write targets refuse both.
