//! `tests.*`: test management.

use polarion::{TestRecord, TestRunUpdate, TestStepResult, Text};
use polarion_commands_derive::{cmd, cmd_args};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ATTACHMENT_COLUMNS, Context, Data, Done, FIELD_COLUMNS, FieldSelection, ItemArg, JsonObject,
    OPTION_COLUMNS, QueryArgs, RECORD_COLUMNS, RUN_COLUMNS, Result, RichText, Run,
};

/// A test run by id (all fields, or only `fields`)
#[cmd(view = object)]
pub struct Get {
    /// Test run id
    #[positional]
    pub id: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
}

impl Run for Get {
    type Output = polarion::TestRun;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let fields = FieldSelection::fields(self.fields.as_ref(), &[]);
        Ok(cx
            .polarion
            .tests()
            .get(
                &cx.project(self.project.as_deref())?,
                &self.id,
                fields.as_ref(),
            )
            .await?)
    }
}

/// A test run by URI.
#[cmd(view = object)]
pub struct GetByUri {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Fields to fetch: `standard`, `all`, or a list of field keys
    pub fields: Option<FieldSelection>,
}

impl Run for GetByUri {
    type Output = polarion::TestRun;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let fields = FieldSelection::fields(self.fields.as_ref(), &[]);
        Ok(cx
            .polarion
            .tests()
            .get_by_uri(&self.run, fields.as_ref())
            .await?)
    }
}

/// Search test runs with a Lucene query.
#[cmd(view = table(RUN_COLUMNS))]
pub struct Search {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for Search {
    type Output = Vec<polarion::TestRun>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.tests().search(&self.search.query(cx)).await?)
    }
}

/// Search test run templates.
#[cmd(view = table(RUN_COLUMNS))]
pub struct SearchTemplates {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for SearchTemplates {
    type Output = Vec<polarion::TestRun>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .search_templates(&self.search.query(cx))
            .await?)
    }
}

/// Records of one test case in a run.
#[cmd(view = table(RECORD_COLUMNS))]
pub struct Records {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Test case work item
    pub test_case: ItemArg,
}

impl Run for Records {
    type Output = Vec<polarion::TestRecord>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .records(&self.run, &self.test_case.resolve(cx)?)
            .await?)
    }
}

/// Search test records across runs (deprecated by Polarion)
#[cmd(view = table(RECORD_COLUMNS))]
pub struct SearchRecords {
    /// Lucene query
    #[positional]
    pub query: String,
    /// Sort order, e.g. `id`, `~updated` (descending)
    pub sort: Option<String>,
    /// Maximum number of results (default 100)
    pub limit: Option<u32>,
}

impl Run for SearchRecords {
    type Output = Vec<polarion::TestRecord>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let sort = self.sort.unwrap_or_default();
        let limit = self.limit.unwrap_or(polarion::DEFAULT_LIMIT);
        Ok(cx
            .polarion
            .tests()
            .search_records(&self.query, &sort, limit)
            .await?)
    }
}

/// Attachments of a run.
#[cmd(view = table(ATTACHMENT_COLUMNS))]
pub struct Attachments {
    /// Test run URI
    #[positional]
    pub run: String,
}

impl Run for Attachments {
    type Output = Vec<polarion::Attachment>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.tests().attachments(&self.run).await?)
    }
}

/// One run attachment (metadata with URL)
#[cmd(view = object)]
pub struct Attachment {
    /// Test run URI
    #[positional]
    pub run: String,
    /// File name
    pub file_name: String,
}

impl Run for Attachment {
    type Output = polarion::Attachment;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .attachment(&self.run, &self.file_name)
            .await?)
    }
}

/// The report page of a run.
#[cmd(view = object)]
pub struct Report {
    /// Test run URI
    #[positional]
    pub run: String,
}

impl Run for Report {
    type Output = polarion::Text;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.tests().report(&self.run).await?)
    }
}

/// Test steps of a test case.
#[cmd(view = object)]
pub struct Steps {
    /// Test case
    #[positional]
    pub test_case: ItemArg,
}

impl Run for Steps {
    type Output = polarion::TestSteps;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .steps(&self.test_case.resolve(cx)?)
            .await?)
    }
}

/// Columns of test steps (project default or for a type)
#[cmd(view = table(OPTION_COLUMNS))]
pub struct StepsConfiguration {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Work item type
    pub r#type: Option<String>,
}

impl Run for StepsConfiguration {
    type Output = Vec<polarion::EnumOption>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let kind = self.r#type;
        Ok(cx
            .polarion
            .tests()
            .steps_configuration(&cx.project(self.project.as_deref())?, kind.as_deref())
            .await?)
    }
}

/// Test management configuration.
#[cmd(view = object)]
pub struct Configuration {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for Configuration {
    type Output = polarion::soap::types::TestsConfiguration;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .configuration(&cx.project(self.project.as_deref())?)
            .await?)
    }
}

/// Parameters of the project library.
#[cmd(view = lines)]
pub struct ParametersLibrary {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for ParametersLibrary {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .parameters_library(&cx.project(self.project.as_deref())?)
            .await?)
    }
}

/// Parameters used in a test case.
#[cmd(view = lines)]
pub struct ParameterNames {
    /// Test case
    #[positional]
    pub test_case: ItemArg,
}

impl Run for ParameterNames {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .parameter_names(&self.test_case.resolve(cx)?)
            .await?)
    }
}

/// Custom fields defined for test runs.
#[cmd(view = table(FIELD_COLUMNS))]
pub struct CustomFields {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Global definitions instead of a project's
    pub global: bool,
}

impl Run for CustomFields {
    type Output = Vec<polarion::FieldDefinition>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .custom_fields(scope(cx, self.project.as_deref(), self.global)?.as_deref())
            .await?)
    }
}

/// One test run custom field definition.
#[cmd(view = object)]
pub struct CustomField {
    /// Field key
    #[positional]
    pub key: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Global definitions instead of a project's
    pub global: bool,
}

impl Run for CustomField {
    type Output = polarion::FieldDefinition;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .custom_field(
                scope(cx, self.project.as_deref(), self.global)?.as_deref(),
                &self.key,
            )
            .await?)
    }
}

/// Keys of test run custom fields.
#[cmd(view = lines)]
pub struct CustomFieldKeys {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Global definitions instead of a project's
    pub global: bool,
}

impl Run for CustomFieldKeys {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .custom_field_keys(scope(cx, self.project.as_deref(), self.global)?.as_deref())
            .await?)
    }
}

/// Custom fields of one run.
#[cmd(view = table(FIELD_COLUMNS))]
pub struct CustomFieldsOf {
    /// Test run URI
    #[positional]
    pub run: String,
}

impl Run for CustomFieldsOf {
    type Output = Vec<polarion::FieldDefinition>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.tests().custom_fields_of(&self.run).await?)
    }
}

/// One custom field of a run.
#[cmd(view = object)]
pub struct CustomFieldOf {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Field key
    pub key: String,
}

impl Run for CustomFieldOf {
    type Output = polarion::FieldDefinition;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .custom_field_of(&self.run, &self.key)
            .await?)
    }
}

/// Create a test run from a template; returns its URI.
#[cmd(write, view = scalar)]
pub struct Create {
    /// Test run id
    #[positional]
    pub id: String,
    /// Template id
    pub template: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Title (template's if omitted)
    pub title: Option<String>,
}

impl Run for Create {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let (project, id, template) =
            (cx.project(self.project.as_deref())?, self.id, self.template);
        let uri = match self.title {
            None => cx.polarion.tests().create(&project, &id, &template).await?,
            Some(title) => {
                cx.polarion
                    .tests()
                    .create_with_title(&project, &id, Some(&title), &template)
                    .await?
            }
        };
        Ok(uri)
    }
}

/// Change fields of a test run.
#[cmd(write, view = done)]
pub struct Update {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Title
    pub title: Option<String>,
    /// Type id
    pub r#type: Option<String>,
    /// Status id
    pub status: Option<String>,
    /// Group id
    pub group_id: Option<String>,
    /// Test case query
    pub query: Option<String>,
    /// Selection mode
    pub select_test_cases_by: Option<String>,
    /// Keep in history
    pub keep_in_history: Option<bool>,
    /// Finish time
    pub finished_on: Option<String>,
    /// Summary defect
    pub summary_defect: Option<ItemArg>,
    /// Custom field values (typed by the run's field definitions)
    pub custom_fields: Option<JsonObject>,
}

impl Run for Update {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let run = self.run;
        let custom = self.custom_fields.unwrap_or_default().0;
        let definitions = if custom.is_empty() {
            Vec::new()
        } else {
            cx.polarion.tests().custom_fields_of(&run).await?
        };
        let update = TestRunUpdate {
            title: self.title,
            kind: self.r#type,
            status: self.status,
            group_id: self.group_id,
            query: self.query,
            select_test_cases_by: self.select_test_cases_by,
            keep_in_history: self.keep_in_history,
            finished_on: self.finished_on,
            summary_defect: self
                .summary_defect
                .as_ref()
                .map(|i| i.resolve(cx))
                .transpose()?,
            custom_fields: crate::values::custom_values(&custom, &definitions)?
                .into_iter()
                .filter_map(|(k, v)| Some((k, v?)))
                .collect(),
        };
        cx.polarion.tests().update(&run, &update).await?;
        Ok(Done)
    }
}

/// Set the report page of a run.
#[cmd(write, view = done)]
pub struct SetReport {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Text: a string (plain text) or {type: text/html, content}
    pub text: RichText,
}

impl Run for SetReport {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .tests()
            .set_report(&self.run, &polarion::Text::from(self.text))
            .await?;
        Ok(Done)
    }
}

/// Add a test record (a test case result) to a run.
#[cmd(write, view = done)]
pub struct AddRecord {
    /// Test run URI
    #[positional]
    pub run: String,
    #[flatten]
    pub record: RecordArgs,
}

impl Run for AddRecord {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let record = self.record.record(cx)?;
        cx.polarion.tests().add_record(&self.run, &record).await?;
        Ok(Done)
    }
}

/// Replace the test record at an index.
#[cmd(write, view = done)]
pub struct UpdateRecord {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Index of the test record in the run
    pub index: u32,
    #[flatten]
    pub record: RecordArgs,
}

impl Run for UpdateRecord {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let record = self.record.record(cx)?;
        cx.polarion
            .tests()
            .update_record(&self.run, self.index, &record)
            .await?;
        Ok(Done)
    }
}

/// Remove the test record at an index.
#[cmd(write, view = done)]
pub struct RemoveRecord {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Index of the test record in the run
    pub record: u32,
}

impl Run for RemoveRecord {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .tests()
            .remove_record(&self.run, self.record)
            .await?;
        Ok(Done)
    }
}

/// Execute test cases: record results like a manual execution.
#[cmd(write, view = done)]
pub struct Execute {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Records: [{test-case, result, comment, ...}] (keys as in add-record; camelCase works too)
    pub records: RecordList,
}

impl Run for Execute {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let records = self
            .records
            .0
            .into_iter()
            .map(|r| r.record(cx))
            .collect::<Result<Vec<_>>>()?;
        cx.polarion.tests().execute(&self.run, &records).await?;
        Ok(Done)
    }
}

/// Set the test steps of a test case (one cell per step column)
#[cmd(write, view = done)]
pub struct SetSteps {
    /// Test case
    #[positional]
    pub test_case: ItemArg,
    /// Steps: [[cell, ...], ...]; cells are text or {type, content}
    pub steps: StepRows,
}

impl Run for SetSteps {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let steps: Vec<Vec<Text>> = self
            .steps
            .0
            .into_iter()
            .map(|row| row.into_iter().map(Text::from).collect())
            .collect();
        cx.polarion
            .tests()
            .set_steps(&self.test_case.resolve(cx)?, &steps)
            .await?;
        Ok(Done)
    }
}

/// Attach a file to a run.
#[cmd(write, view = done)]
pub struct AddAttachment {
    /// Test run URI
    #[positional]
    pub run: String,
    /// File name
    pub file_name: String,
    /// Title
    pub title: String,
    /// Content
    pub data: Data,
}

impl Run for AddAttachment {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .tests()
            .add_attachment(&self.run, &self.file_name, &self.title, &self.data.0)
            .await?;
        Ok(Done)
    }
}

/// Change a run attachment.
#[cmd(write, view = done)]
pub struct UpdateAttachment {
    /// Test run URI
    #[positional]
    pub run: String,
    /// File name
    pub file_name: String,
    /// Title
    pub title: Option<String>,
    /// Content
    pub data: Option<Data>,
}

impl Run for UpdateAttachment {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let (title, data) = (self.title, self.data.map(|d| d.0));
        cx.polarion
            .tests()
            .update_attachment(
                &self.run,
                &self.file_name,
                title.as_deref(),
                data.as_deref(),
            )
            .await?;
        Ok(Done)
    }
}

/// Delete a run attachment.
#[cmd(write, view = done)]
pub struct DeleteAttachment {
    /// Test run URI
    #[positional]
    pub run: String,
    /// File name
    pub file_name: String,
}

impl Run for DeleteAttachment {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .tests()
            .delete_attachment(&self.run, &self.file_name)
            .await?;
        Ok(Done)
    }
}

/// Attach a file to a test record.
#[cmd(write, view = done)]
pub struct AddRecordAttachment {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Index of the test record in the run
    pub record: u32,
    /// File name
    pub file_name: String,
    /// Title
    pub title: String,
    /// Content
    pub data: Data,
}

impl Run for AddRecordAttachment {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .tests()
            .add_record_attachment(
                &self.run,
                self.record,
                &self.file_name,
                &self.title,
                &self.data.0,
            )
            .await?;
        Ok(Done)
    }
}

/// Delete a test record attachment.
#[cmd(write, view = done)]
pub struct DeleteRecordAttachment {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Index of the test record in the run
    pub record: u32,
    /// File name
    pub file_name: String,
}

impl Run for DeleteRecordAttachment {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .tests()
            .delete_record_attachment(&self.run, self.record, &self.file_name)
            .await?;
        Ok(Done)
    }
}

/// Attach a file to a test step result.
#[cmd(write, view = done)]
pub struct AddStepAttachment {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Index of the test record in the run
    pub record: u32,
    /// Step index
    pub step: u32,
    /// File name
    pub file_name: String,
    /// Title
    pub title: String,
    /// Content
    pub data: Data,
}

impl Run for AddStepAttachment {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .tests()
            .add_step_attachment(
                &self.run,
                self.record,
                self.step,
                &self.file_name,
                &self.title,
                &self.data.0,
            )
            .await?;
        Ok(Done)
    }
}

/// Delete a test step attachment.
#[cmd(write, view = done)]
pub struct DeleteStepAttachment {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Index of the test record in the run
    pub record: u32,
    /// Step index
    pub step: u32,
    /// File name
    pub file_name: String,
}

impl Run for DeleteStepAttachment {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .tests()
            .delete_step_attachment(&self.run, self.record, self.step, &self.file_name)
            .await?;
        Ok(Done)
    }
}

/// Create (or return) the summary defect of a run.
#[cmd(write, view = object)]
pub struct CreateSummaryDefect {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Defect template URI
    pub template: String,
}

impl Run for CreateSummaryDefect {
    type Output = polarion::ItemRef;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .create_summary_defect(&self.run, &self.template)
            .await?)
    }
}

/// Create or update the summary defect with statistics.
#[cmd(write, view = object)]
pub struct UpdateSummaryDefect {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Source
    pub source: String,
    /// Failures
    pub failures: u32,
    /// Errors
    pub errors: u32,
    /// Tests
    pub tests: u32,
    /// Defect template URI
    pub template: String,
}

impl Run for UpdateSummaryDefect {
    type Output = polarion::ItemRef;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .tests()
            .update_summary_defect(
                &self.run,
                &self.source,
                self.failures,
                self.errors,
                self.tests,
                &self.template,
            )
            .await?)
    }
}

/// Add a project to the run's project span.
#[cmd(write, view = done)]
pub struct AddProjectToSpan {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Project id
    pub span_project: String,
}

impl Run for AddProjectToSpan {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .tests()
            .add_project_to_span(&self.run, &self.span_project)
            .await?;
        Ok(Done)
    }
}

/// Remove a project from the run's project span.
#[cmd(write, view = done)]
pub struct RemoveProjectFromSpan {
    /// Test run URI
    #[positional]
    pub run: String,
    /// Project id
    pub span_project: String,
}

impl Run for RemoveProjectFromSpan {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .tests()
            .remove_project_from_span(&self.run, &self.span_project)
            .await?;
        Ok(Done)
    }
}

/// The project's definitions, or the global ones.
fn scope(cx: &Context<'_>, project: Option<&str>, global: bool) -> Result<Option<String>> {
    if global {
        Ok(None)
    } else {
        cx.project(project).map(Some)
    }
}

/// One step's result in a test record.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepResult {
    /// Result id, e.g. `passed`
    #[serde(default)]
    pub result: Option<String>,
    /// Comment
    #[serde(default)]
    pub comment: Option<RichText>,
}

/// Step results of a test record. On the command line: a JSON array.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct StepResults(pub Vec<StepResult>);

/// A test record (a test case result), as added to a run or executed.
#[cmd_args]
pub struct RecordArgs {
    /// Test case work item
    #[serde(alias = "testCase")]
    pub test_case: ItemArg,
    /// Result id, e.g. `passed`, `failed`, `blocked`
    pub result: Option<String>,
    /// Comment
    pub comment: Option<RichText>,
    /// Execution time (ISO 8601)
    pub executed: Option<String>,
    /// User id
    #[serde(alias = "executedBy")]
    pub executed_by: Option<String>,
    /// Duration in seconds
    pub duration: Option<f32>,
    /// Defect work item
    pub defect: Option<ItemArg>,
    /// Iteration
    pub iteration: Option<i32>,
    /// Test parameter values by name
    pub parameters: Option<JsonObject>,
    /// Step results: [{result, comment}]
    #[serde(alias = "stepResults")]
    pub step_results: Option<StepResults>,
}

impl RecordArgs {
    fn record(self, cx: &Context<'_>) -> Result<TestRecord> {
        Ok(TestRecord {
            test_case: Some(self.test_case.resolve(cx)?),
            result: self.result,
            comment: self.comment.map(Text::from),
            executed: self.executed,
            executed_by: self.executed_by,
            duration: self.duration,
            defect: self.defect.map(|d| d.resolve(cx)).transpose()?,
            iteration: self.iteration,
            parameters: crate::values::custom_values(&self.parameters.unwrap_or_default().0, &[])?
                .into_iter()
                .filter_map(|(k, v)| Some((k, v?)))
                .collect(),
            step_results: self
                .step_results
                .unwrap_or_default()
                .0
                .into_iter()
                .map(|s| TestStepResult {
                    result: s.result,
                    comment: s.comment.map(Text::from),
                    attachments: Vec::new(),
                })
                .collect(),
            ..TestRecord::default()
        })
    }
}

/// Records for `execute`. On the command line: a JSON array.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct RecordList(pub Vec<RecordArgs>);

/// Test steps: rows of cells. On the command line: a JSON array of arrays.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct StepRows(pub Vec<Vec<RichText>>);

/// `FromStr` for the JSON-only arguments (command line).
macro_rules! from_json_text {
    ($($ty:ty),*) => {$(
        impl std::str::FromStr for $ty {
            type Err = String;
            fn from_str(s: &str) -> std::result::Result<Self, String> {
                serde_json::from_str(s).map_err(|e| format!("expected JSON: {e}"))
            }
        }
    )*};
}
from_json_text!(StepResults, RecordList, StepRows);
