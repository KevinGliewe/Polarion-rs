# Plans

Plans (releases, iterations, sprints) group work items over time. They are created from plan templates and can
be nested (`parent`).

## Finding and reading plans

- `search_plans`: Lucene search over plans (see `queries`); `plans.search-templates` over plan templates.
- `get_plan`: one plan by `id` (in the default project or `project`); `plans.get-by-uri` by URI. Most other
  commands take the plan's URI: `subterra:data-service:objects:/default/MP${Plan}rel1`. Results carry it as
  `uri`.
- A plan has a name, status, parent, template, description, start and due dates, `startedOn`/`finishedOn`,
  capacity, default estimate, estimation and prioritization fields, color, sort order and custom fields.
- `plan_statistics`: the plan's progress statistics; `plans.report` its page content; `plans.was-started`
  whether it was started.

## Work items in a plan

- A work item's plans are its field `plannedIn` (plan URIs in results). Query them with the field id
  `plannedIn`; `plannedInURIs` is refused by the server.
- `add_plan_items` and `remove_plan_items`: `plan` (URI) and `items` (work items) plan and unplan work items.
- `plans.add-allowed-type`, `plans.remove-allowed-type`: the work item types a plan accepts.

## Creating and changing plans

- `create_plan`: `id`, `name`, `template` (a template id), optionally `parent` (a plan id) and `project`. It
  returns the new plan's URI. `plans.create-template` creates a template.
- `update_plan`: `plan` (URI), `name`, `status`, `description`, `start-date`, `due-date`, `capacity`,
  `default-estimate`, `estimation-field`, `prioritization-field`, `color`, `sort-order`, and `custom-fields`
  (typed from JSON; `{"enum": "id"}` for an enum option). Fields not given stay as they are.
- `plans.set-report` replaces the plan's page content.
- `plans.add-project-to-span`, `plans.remove-project-from-span`: projects the plan spans.
- `plans.delete`: deletes plans by `ids`.

## Limits on restricted servers

- A plan is a write target only by its URI (`...MP${Plan}rel1`).
- Plan items are work items: `add_plan_items` and `remove_plan_items` need every work item to be a target too.
- Creates and deletes are refused.

## Not yet verified against a real server

Plan updates and the other plan writes are tested against a mock server only. Read the plan back after a
change to see what Polarion stored.

## Not available

Plans have no attachment operations in the SOAP API.
