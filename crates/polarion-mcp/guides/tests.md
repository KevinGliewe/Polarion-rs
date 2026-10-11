# Test management

Test cases are work items (with test steps); test runs record the results of executing them. A test run's
results are its test records, one per executed test case (and iteration).

## Test runs

- `search_test_runs`: Lucene search over test runs (see `queries`), e.g. `status:open`.
- `get_test_run`: one run by `id` (in the default project or `project`); `tests.get-by-uri` by URI. Most other
  commands take the run's URI: `subterra:data-service:objects:/default/MP${TestRun}RUN-1`.
- `test_run_report`: the run's report page (rich text); `tests.set-report` replaces it.
- `tests.custom-field-keys`, `tests.custom-fields`: the custom fields of test runs (project or `global`
  definitions); `tests.custom-fields-of` those of one run.
- `tests.configuration`: the project's test management configuration.

## Templates and creating runs

- Runs are created from a template: `tests.search-templates` finds them (Lucene), `create_test_run` takes the
  new run's `id`, the `template` id and optionally a `title` (the template's otherwise). It returns the URI.
- `update_test_run`: `run` (URI), `title`, `type`, `status`, `group-id`, `query` (the test case query),
  `select-test-cases-by`, `keep-in-history`, `finished-on`, `summary-defect`, and `custom-fields` (typed by the
  run's field definitions).

## Test records (results)

- `test_run_records`: the records of one `test-case` in a run.
- `add_test_record`: `run` and a record: `test-case`, `result` (an id such as `passed`, `failed`, `blocked`),
  `comment`, `executed` (ISO 8601), `executed-by` (user id), `duration` (seconds), `defect` (a work item),
  `iteration`, `parameters` (test parameter values by name) and `step-results` (`[{"result": ...,
  "comment": ...}]`, one per step).
- `tests.update-record` replaces the record at an `index`; `tests.remove-record` removes one.
- `tests.execute` records several results like a manual execution: `records`, a list of records as above.
- Use the result ids the project uses; the records of an existing run show them.

## Test steps

- `test_case_steps`: the steps of a test case (a work item).
- `set_test_steps`: `test-case` and `steps`, rows of cells: `[["Press the pedal", "The car stops"], ...]`,
  one cell per step column; a cell is text or `{"type": "text/html", "content": "..."}`. It replaces all steps.
- `tests.steps-configuration`: the step columns (the project's default, or for a work item type).
- Writing steps writes the test case, so it needs write access to that work item, not to a test run.

## Parameters

- `tests.parameters-library`: the parameters of the project's library; `tests.parameter-names` those a test case
  uses. Record their values with `parameters` in a test record.

## Attachments

- `tests.add-attachment`, `tests.update-attachment`, `tests.delete-attachment`: files of a run (`file-name`,
  `title`, `data` or `data-file`).
- `tests.add-record-attachment` (a record by `record` index) and `tests.add-step-attachment` (`record`, `step`),
  with their `delete-` counterparts. Polarion stores them as `<test case id>_<file name>` and
  `<test case id>_<step>_<file name>`.
- `tests.attachments` lists a run's attachments (with URLs); their content is not available through SOAP.

## Defects

- A record's `defect` links a defect work item to the result.
- `tests.create-summary-defect` creates (or returns) the summary defect of a run from a defect template;
  `tests.update-summary-defect` updates it with statistics.

## Limits on restricted servers

- Creates are refused, so is `tests.execute`.
- A test record names its test case (and defect): they must be write targets too.
- Record and step attachments, and test records themselves, may be added, changed and deleted on a target run.
