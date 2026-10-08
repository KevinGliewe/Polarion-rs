//! `tests.*`: test management.

use polarion::{Polarion, TestRecord, TestRunUpdate, TestStepResult, Text};
use serde_json::{Value, json};

use crate::args::Args;
use crate::catalog::*;
use crate::error::{CommandError, Result};
use crate::util::{command, out};

const RUN: Param = Param::pos("run", Kind::String, "Test run URI");
const TEST_CASE: Param = Param::req("test-case", Kind::Item, "Test case work item");
const FILE_NAME: Param = Param::req("file-name", Kind::String, "File name");
const RECORD_INDEX: Param = Param::req(
    "record",
    Kind::Integer,
    "Index of the test record in the run",
);
const GLOBAL: Param = Param::opt(
    "global",
    Kind::Boolean,
    "Global definitions instead of a project's",
);
const RECORD: [Param; 10] = [
    TEST_CASE,
    Param::opt(
        "result",
        Kind::String,
        "Result id, e.g. `passed`, `failed`, `blocked`",
    ),
    Param::opt("comment", Kind::Text, "Comment"),
    Param::opt("executed", Kind::String, "Execution time (ISO 8601)"),
    Param::opt("executed-by", Kind::String, "User id"),
    Param::opt("duration", Kind::Number, "Duration in seconds"),
    Param::opt("defect", Kind::Item, "Defect work item"),
    Param::opt("iteration", Kind::Integer, "Iteration"),
    Param::opt("parameters", Kind::Json, "Test parameter values by name"),
    Param::opt(
        "step-results",
        Kind::Any,
        "Step results: [{result, comment}]",
    ),
];

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "tests",
        "get",
        false,
        Output::Object,
        "A test run by id (all fields, or only `fields`)",
        [
            Param::pos("id", Kind::String, "Test run id"),
            PROJECT,
            FIELDS
        ],
        get
    ),
    command!(
        "tests",
        "get-by-uri",
        false,
        Output::Object,
        "A test run by URI",
        [RUN, FIELDS],
        get_by_uri
    ),
    command!(
        "tests",
        "search",
        false,
        Output::Table(RUN_COLUMNS),
        "Search test runs with a Lucene query",
        [QUERY, PROJECT, SORT, LIMIT, ALL, FIELDS],
        search
    ),
    command!(
        "tests",
        "search-templates",
        false,
        Output::Table(RUN_COLUMNS),
        "Search test run templates",
        [QUERY, PROJECT, SORT, LIMIT, ALL, FIELDS],
        search_templates
    ),
    command!(
        "tests",
        "records",
        false,
        Output::Table(RECORD_COLUMNS),
        "Records of one test case in a run",
        [RUN, TEST_CASE],
        records
    ),
    command!(
        "tests",
        "search-records",
        false,
        Output::Table(RECORD_COLUMNS),
        "Search test records across runs (deprecated by Polarion)",
        [
            Param::pos("query", Kind::String, "Lucene query"),
            SORT,
            LIMIT
        ],
        search_records
    ),
    command!(
        "tests",
        "attachments",
        false,
        Output::Table(ATTACHMENT_COLUMNS),
        "Attachments of a run",
        [RUN],
        attachments
    ),
    command!(
        "tests",
        "attachment",
        false,
        Output::Object,
        "One run attachment (metadata with URL)",
        [RUN, FILE_NAME],
        attachment
    ),
    command!(
        "tests",
        "report",
        false,
        Output::Object,
        "The report page of a run",
        [RUN],
        report
    ),
    command!(
        "tests",
        "steps",
        false,
        Output::Object,
        "Test steps of a test case",
        [Param::pos("test-case", Kind::Item, "Test case")],
        steps
    ),
    command!(
        "tests",
        "steps-configuration",
        false,
        Output::Table(OPTION_COLUMNS),
        "Columns of test steps (project default or for a type)",
        [PROJECT, Param::opt("type", Kind::String, "Work item type")],
        steps_configuration
    ),
    command!(
        "tests",
        "configuration",
        false,
        Output::Object,
        "Test management configuration",
        [PROJECT],
        configuration
    ),
    command!(
        "tests",
        "parameters-library",
        false,
        Output::Lines,
        "Parameters of the project library",
        [PROJECT],
        parameters_library
    ),
    command!(
        "tests",
        "parameter-names",
        false,
        Output::Lines,
        "Parameters used in a test case",
        [Param::pos("test-case", Kind::Item, "Test case")],
        parameter_names
    ),
    command!(
        "tests",
        "custom-fields",
        false,
        Output::Table(FIELD_COLUMNS),
        "Custom fields defined for test runs",
        [PROJECT, GLOBAL],
        custom_fields
    ),
    command!(
        "tests",
        "custom-field",
        false,
        Output::Object,
        "One test run custom field definition",
        [
            Param::pos("key", Kind::String, "Field key"),
            PROJECT,
            GLOBAL
        ],
        custom_field
    ),
    command!(
        "tests",
        "custom-field-keys",
        false,
        Output::Lines,
        "Keys of test run custom fields",
        [PROJECT, GLOBAL],
        custom_field_keys
    ),
    command!(
        "tests",
        "custom-fields-of",
        false,
        Output::Table(FIELD_COLUMNS),
        "Custom fields of one run",
        [RUN],
        custom_fields_of
    ),
    command!(
        "tests",
        "custom-field-of",
        false,
        Output::Object,
        "One custom field of a run",
        [RUN, Param::req("key", Kind::String, "Field key")],
        custom_field_of
    ),
    command!(
        "tests",
        "create",
        true,
        Output::Scalar,
        "Create a test run from a template; returns its URI",
        [
            Param::pos("id", Kind::String, "Test run id"),
            Param::req("template", Kind::String, "Template id"),
            PROJECT,
            Param::opt("title", Kind::String, "Title (template's if omitted)")
        ],
        create
    ),
    command!(
        "tests",
        "update",
        true,
        Output::Done,
        "Change fields of a test run",
        [
            RUN,
            Param::opt("title", Kind::String, "Title"),
            Param::opt("type", Kind::String, "Type id"),
            Param::opt("status", Kind::String, "Status id"),
            Param::opt("group-id", Kind::String, "Group id"),
            Param::opt("query", Kind::String, "Test case query"),
            Param::opt("select-test-cases-by", Kind::String, "Selection mode"),
            Param::opt("keep-in-history", Kind::Boolean, "Keep in history"),
            Param::opt("finished-on", Kind::String, "Finish time"),
            Param::opt("summary-defect", Kind::Item, "Summary defect"),
            Param::opt(
                "custom-fields",
                Kind::Json,
                "Custom field values (typed by the run's field definitions)"
            ),
        ],
        update
    ),
    command!(
        "tests",
        "set-report",
        true,
        Output::Done,
        "Set the report page of a run",
        [RUN, TEXT],
        set_report
    ),
    command!(
        "tests",
        "add-record",
        true,
        Output::Done,
        "Add a test record (a test case result) to a run",
        [
            RUN, RECORD[0], RECORD[1], RECORD[2], RECORD[3], RECORD[4], RECORD[5], RECORD[6],
            RECORD[7], RECORD[8], RECORD[9]
        ],
        add_record
    ),
    command!(
        "tests",
        "update-record",
        true,
        Output::Done,
        "Replace the test record at an index",
        [
            RUN,
            RECORD_INDEX,
            RECORD[0],
            RECORD[1],
            RECORD[2],
            RECORD[3],
            RECORD[4],
            RECORD[5],
            RECORD[6],
            RECORD[7],
            RECORD[8],
            RECORD[9]
        ],
        update_record
    ),
    command!(
        "tests",
        "remove-record",
        true,
        Output::Done,
        "Remove the test record at an index",
        [RUN, RECORD_INDEX],
        remove_record
    ),
    command!(
        "tests",
        "execute",
        true,
        Output::Done,
        "Execute test cases: record results like a manual execution",
        [
            RUN,
            Param::req(
                "records",
                Kind::Any,
                "Records: [{testCase, result, comment, ...}] (keys as in add-record)"
            )
        ],
        execute
    ),
    command!(
        "tests",
        "set-steps",
        true,
        Output::Done,
        "Set the test steps of a test case (one cell per step column)",
        [
            Param::pos("test-case", Kind::Item, "Test case"),
            Param::req(
                "steps",
                Kind::Any,
                "Steps: [[cell, ...], ...]; cells are text or {type, content}"
            )
        ],
        set_steps
    ),
    command!(
        "tests",
        "add-attachment",
        true,
        Output::Done,
        "Attach a file to a run",
        [
            RUN,
            FILE_NAME,
            Param::req("title", Kind::String, "Title"),
            Param::req("data", Kind::Bytes, "Content")
        ],
        add_attachment
    ),
    command!(
        "tests",
        "update-attachment",
        true,
        Output::Done,
        "Change a run attachment",
        [
            RUN,
            FILE_NAME,
            Param::opt("title", Kind::String, "Title"),
            Param::opt("data", Kind::Bytes, "Content")
        ],
        update_attachment
    ),
    command!(
        "tests",
        "delete-attachment",
        true,
        Output::Done,
        "Delete a run attachment",
        [RUN, FILE_NAME],
        delete_attachment
    ),
    command!(
        "tests",
        "add-record-attachment",
        true,
        Output::Done,
        "Attach a file to a test record",
        [
            RUN,
            RECORD_INDEX,
            FILE_NAME,
            Param::req("title", Kind::String, "Title"),
            Param::req("data", Kind::Bytes, "Content")
        ],
        add_record_attachment
    ),
    command!(
        "tests",
        "delete-record-attachment",
        true,
        Output::Done,
        "Delete a test record attachment",
        [RUN, RECORD_INDEX, FILE_NAME],
        delete_record_attachment
    ),
    command!(
        "tests",
        "add-step-attachment",
        true,
        Output::Done,
        "Attach a file to a test step result",
        [
            RUN,
            RECORD_INDEX,
            Param::req("step", Kind::Integer, "Step index"),
            FILE_NAME,
            Param::req("title", Kind::String, "Title"),
            Param::req("data", Kind::Bytes, "Content")
        ],
        add_step_attachment
    ),
    command!(
        "tests",
        "delete-step-attachment",
        true,
        Output::Done,
        "Delete a test step attachment",
        [
            RUN,
            RECORD_INDEX,
            Param::req("step", Kind::Integer, "Step index"),
            FILE_NAME
        ],
        delete_step_attachment
    ),
    command!(
        "tests",
        "create-summary-defect",
        true,
        Output::Object,
        "Create (or return) the summary defect of a run",
        [
            RUN,
            Param::req("template", Kind::String, "Defect template URI")
        ],
        create_summary_defect
    ),
    command!(
        "tests",
        "update-summary-defect",
        true,
        Output::Object,
        "Create or update the summary defect with statistics",
        [
            RUN,
            Param::req("source", Kind::String, "Source"),
            Param::req("failures", Kind::Integer, "Failures"),
            Param::req("errors", Kind::Integer, "Errors"),
            Param::req("tests", Kind::Integer, "Tests"),
            Param::req("template", Kind::String, "Defect template URI")
        ],
        update_summary_defect
    ),
    command!(
        "tests",
        "add-project-to-span",
        true,
        Output::Done,
        "Add a project to the run's project span",
        [RUN, Param::req("span-project", Kind::String, "Project id")],
        add_project_to_span
    ),
    command!(
        "tests",
        "remove-project-from-span",
        true,
        Output::Done,
        "Remove a project from the run's project span",
        [RUN, Param::req("span-project", Kind::String, "Project id")],
        remove_project_from_span
    ),
];

fn scope(a: &Args) -> Result<Option<String>> {
    if a.bool("global")? {
        Ok(None)
    } else {
        a.project().map(Some)
    }
}

/// A test record from the record parameters.
fn record(a: &Args) -> Result<TestRecord> {
    let step_results = match a.get("step-results") {
        None => Vec::new(),
        Some(Value::Array(steps)) => steps
            .iter()
            .map(|s| {
                let step = Args::from_value(s.clone(), None)?;
                Ok(TestStepResult {
                    result: step.opt_str("result")?,
                    comment: step.opt_text("comment")?,
                    attachments: Vec::new(),
                })
            })
            .collect::<Result<_>>()?,
        Some(other) => {
            return Err(CommandError::invalid(format!(
                "`step-results`: expected [{{result, comment}}], got {other}"
            )));
        }
    };
    Ok(TestRecord {
        test_case: Some(a.item("test-case")?),
        result: a.opt_str("result")?,
        comment: a.opt_text("comment")?,
        executed: a.opt_str("executed")?,
        executed_by: a.opt_str("executed-by")?,
        duration: a.opt_f32("duration")?,
        defect: a.opt_item("defect")?,
        iteration: a
            .opt_i64("iteration")?
            .map(|i| i32::try_from(i).unwrap_or(i32::MAX)),
        parameters: a
            .custom_values("parameters", &[])?
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect(),
        step_results,
        ..TestRecord::default()
    })
}

async fn get(p: &Polarion, a: &Args) -> Result<Value> {
    let fields = a.opt_fields()?;
    out(p
        .tests()
        .get(&a.project()?, &a.str("id")?, fields.as_ref())
        .await?)
}

async fn get_by_uri(p: &Polarion, a: &Args) -> Result<Value> {
    let fields = a.opt_fields()?;
    out(p
        .tests()
        .get_by_uri(&a.str("run")?, fields.as_ref())
        .await?)
}

async fn search(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.tests().search(&a.query()?).await?)
}

async fn search_templates(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.tests().search_templates(&a.query()?).await?)
}

async fn records(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .tests()
        .records(&a.str("run")?, &a.item("test-case")?)
        .await?)
}

async fn search_records(p: &Polarion, a: &Args) -> Result<Value> {
    let sort = a.opt_str("sort")?.unwrap_or_default();
    let limit = a.opt_u32("limit")?.unwrap_or(polarion::DEFAULT_LIMIT);
    out(p
        .tests()
        .search_records(&a.str("query")?, &sort, limit)
        .await?)
}

async fn attachments(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.tests().attachments(&a.str("run")?).await?)
}

async fn attachment(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .tests()
        .attachment(&a.str("run")?, &a.str("file-name")?)
        .await?)
}

async fn report(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.tests().report(&a.str("run")?).await?)
}

async fn steps(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.tests().steps(&a.item("test-case")?).await?)
}

async fn steps_configuration(p: &Polarion, a: &Args) -> Result<Value> {
    let kind = a.opt_str("type")?;
    out(p
        .tests()
        .steps_configuration(&a.project()?, kind.as_deref())
        .await?)
}

async fn configuration(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.tests().configuration(&a.project()?).await?)
}

async fn parameters_library(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.tests().parameters_library(&a.project()?).await?)
}

async fn parameter_names(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.tests().parameter_names(&a.item("test-case")?).await?)
}

async fn custom_fields(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.tests().custom_fields(scope(a)?.as_deref()).await?)
}

async fn custom_field(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .tests()
        .custom_field(scope(a)?.as_deref(), &a.str("key")?)
        .await?)
}

async fn custom_field_keys(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.tests().custom_field_keys(scope(a)?.as_deref()).await?)
}

async fn custom_fields_of(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.tests().custom_fields_of(&a.str("run")?).await?)
}

async fn custom_field_of(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .tests()
        .custom_field_of(&a.str("run")?, &a.str("key")?)
        .await?)
}

async fn create(p: &Polarion, a: &Args) -> Result<Value> {
    let (project, id, template) = (a.project()?, a.str("id")?, a.str("template")?);
    let uri = match a.opt_str("title")? {
        None => p.tests().create(&project, &id, &template).await?,
        Some(title) => {
            p.tests()
                .create_with_title(&project, &id, Some(&title), &template)
                .await?
        }
    };
    out(uri)
}

async fn update(p: &Polarion, a: &Args) -> Result<Value> {
    let run = a.str("run")?;
    let definitions = if a.object("custom-fields")?.is_empty() {
        Vec::new()
    } else {
        p.tests().custom_fields_of(&run).await?
    };
    let update = TestRunUpdate {
        title: a.opt_str("title")?,
        kind: a.opt_str("type")?,
        status: a.opt_str("status")?,
        group_id: a.opt_str("group-id")?,
        query: a.opt_str("query")?,
        select_test_cases_by: a.opt_str("select-test-cases-by")?,
        keep_in_history: a.opt_bool("keep-in-history")?,
        finished_on: a.opt_str("finished-on")?,
        summary_defect: a.opt_item("summary-defect")?,
        custom_fields: a
            .custom_values("custom-fields", &definitions)?
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect(),
    };
    p.tests().update(&run, &update).await?;
    Ok(Value::Null)
}

async fn set_report(p: &Polarion, a: &Args) -> Result<Value> {
    p.tests()
        .set_report(&a.str("run")?, &a.text("text")?)
        .await?;
    Ok(Value::Null)
}

async fn add_record(p: &Polarion, a: &Args) -> Result<Value> {
    p.tests().add_record(&a.str("run")?, &record(a)?).await?;
    Ok(Value::Null)
}

async fn update_record(p: &Polarion, a: &Args) -> Result<Value> {
    p.tests()
        .update_record(&a.str("run")?, a.u32("record")?, &record(a)?)
        .await?;
    Ok(Value::Null)
}

async fn remove_record(p: &Polarion, a: &Args) -> Result<Value> {
    p.tests()
        .remove_record(&a.str("run")?, a.u32("record")?)
        .await?;
    Ok(Value::Null)
}

async fn execute(p: &Polarion, a: &Args) -> Result<Value> {
    let Some(Value::Array(list)) = a.get("records") else {
        return Err(CommandError::invalid(
            "`records`: expected an array of records",
        ));
    };
    let records: Vec<TestRecord> = list
        .iter()
        .map(|r| {
            // Accept both `test-case` and `testCase` style keys.
            let mut r = r.clone();
            if let Some(map) = r.as_object_mut() {
                for (camel, kebab) in [
                    ("testCase", "test-case"),
                    ("executedBy", "executed-by"),
                    ("stepResults", "step-results"),
                ] {
                    if let Some(v) = map.remove(camel) {
                        map.insert(kebab.into(), v);
                    }
                }
            }
            record(&Args::from_value(r, a.opt_project()?)?)
        })
        .collect::<Result<_>>()?;
    p.tests().execute(&a.str("run")?, &records).await?;
    Ok(Value::Null)
}

async fn set_steps(p: &Polarion, a: &Args) -> Result<Value> {
    let bad = || CommandError::invalid("`steps`: expected [[cell, ...], ...]");
    let Some(Value::Array(rows)) = a.get("steps") else {
        return Err(bad());
    };
    let steps: Vec<Vec<Text>> = rows
        .iter()
        .map(|row| {
            let Value::Array(cells) = row else {
                return Err(bad());
            };
            cells
                .iter()
                .map(|cell| {
                    let wrapper = Args::from_value(json!({ "cell": cell }), None)?;
                    wrapper.text("cell")
                })
                .collect()
        })
        .collect::<Result<_>>()?;
    p.tests().set_steps(&a.item("test-case")?, &steps).await?;
    Ok(Value::Null)
}

async fn add_attachment(p: &Polarion, a: &Args) -> Result<Value> {
    p.tests()
        .add_attachment(
            &a.str("run")?,
            &a.str("file-name")?,
            &a.str("title")?,
            &a.bytes("data")?,
        )
        .await?;
    Ok(Value::Null)
}

async fn update_attachment(p: &Polarion, a: &Args) -> Result<Value> {
    let (title, data) = (a.opt_str("title")?, a.opt_bytes("data")?);
    p.tests()
        .update_attachment(
            &a.str("run")?,
            &a.str("file-name")?,
            title.as_deref(),
            data.as_deref(),
        )
        .await?;
    Ok(Value::Null)
}

async fn delete_attachment(p: &Polarion, a: &Args) -> Result<Value> {
    p.tests()
        .delete_attachment(&a.str("run")?, &a.str("file-name")?)
        .await?;
    Ok(Value::Null)
}

async fn add_record_attachment(p: &Polarion, a: &Args) -> Result<Value> {
    p.tests()
        .add_record_attachment(
            &a.str("run")?,
            a.u32("record")?,
            &a.str("file-name")?,
            &a.str("title")?,
            &a.bytes("data")?,
        )
        .await?;
    Ok(Value::Null)
}

async fn delete_record_attachment(p: &Polarion, a: &Args) -> Result<Value> {
    p.tests()
        .delete_record_attachment(&a.str("run")?, a.u32("record")?, &a.str("file-name")?)
        .await?;
    Ok(Value::Null)
}

async fn add_step_attachment(p: &Polarion, a: &Args) -> Result<Value> {
    p.tests()
        .add_step_attachment(
            &a.str("run")?,
            a.u32("record")?,
            a.u32("step")?,
            &a.str("file-name")?,
            &a.str("title")?,
            &a.bytes("data")?,
        )
        .await?;
    Ok(Value::Null)
}

async fn delete_step_attachment(p: &Polarion, a: &Args) -> Result<Value> {
    p.tests()
        .delete_step_attachment(
            &a.str("run")?,
            a.u32("record")?,
            a.u32("step")?,
            &a.str("file-name")?,
        )
        .await?;
    Ok(Value::Null)
}

async fn create_summary_defect(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .tests()
        .create_summary_defect(&a.str("run")?, &a.str("template")?)
        .await?)
}

async fn update_summary_defect(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .tests()
        .update_summary_defect(
            &a.str("run")?,
            &a.str("source")?,
            a.u32("failures")?,
            a.u32("errors")?,
            a.u32("tests")?,
            &a.str("template")?,
        )
        .await?)
}

async fn add_project_to_span(p: &Polarion, a: &Args) -> Result<Value> {
    p.tests()
        .add_project_to_span(&a.str("run")?, &a.str("span-project")?)
        .await?;
    Ok(Value::Null)
}

async fn remove_project_from_span(p: &Polarion, a: &Args) -> Result<Value> {
    p.tests()
        .remove_project_from_span(&a.str("run")?, &a.str("span-project")?)
        .await?;
    Ok(Value::Null)
}
