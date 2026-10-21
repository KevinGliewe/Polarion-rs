//! Test management: test runs, test records, test steps.

use polarion_soap::services::TestManagementService;
use polarion_soap::types as soap;

use crate::client::{CreateKind, Polarion, not_found};
use crate::convert::{option_id, soap_custom_fields, soap_test_record, soap_text};
use crate::error::{Error, Result};
use crate::model::{Attachment, EnumOption, FieldDefinition, TestRecord, TestRun, TestSteps, Text};
use crate::query::{Fields, Query};
use crate::uri::{self, ItemRef};
use crate::write::{NewTestRun, TestRunUpdate};

fn runs(list: Vec<soap::TestRun>) -> Vec<TestRun> {
    list.into_iter()
        .filter(|r| r.unresolvable != Some(true))
        .map(TestRun::from)
        .collect()
}

fn found(run: soap::TestRun, id: impl std::fmt::Display) -> Result<TestRun> {
    if run.unresolvable == Some(true) || (run.uri.is_none() && run.id.is_none()) {
        return Err(not_found("test run", id));
    }
    Ok(run.into())
}

fn index(i: u32) -> i32 {
    i32::try_from(i).unwrap_or(i32::MAX)
}

/// Test management: [`Polarion::tests`]. Test runs are addressed by URI ([`TestRun::uri`]);
/// writes check the project in that URI against the write allowlist, or the URI against the
/// write targets of a restricted client.
#[derive(Debug, Clone, Copy)]
pub struct TestRuns<'a> {
    polarion: &'a Polarion,
}

impl<'a> TestRuns<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    fn service(&self) -> TestManagementService<'a> {
        self.polarion.soap().test_management()
    }

    /// A write to the test run itself (its records and attachments included).
    fn writable_run(
        &self,
        operation: &'static str,
        run_uri: &str,
    ) -> Result<TestManagementService<'a>> {
        self.polarion.check_write_target(operation, run_uri)?;
        Ok(self.service())
    }

    /// A write that may change other objects or cannot be undone: refused in restricted mode.
    fn unbounded_run(
        &self,
        operation: &'static str,
        run_uri: &str,
    ) -> Result<TestManagementService<'a>> {
        self.polarion.check_unbounded_write(operation)?;
        self.polarion.check_write_uri(operation, run_uri)?;
        Ok(self.service())
    }

    /// A write of a test record to the test run: the test case, the defect and the user who
    /// executed it are named too (restricted mode: all of them must be write targets).
    fn writable_record(
        &self,
        operation: &'static str,
        run_uri: &str,
        record: &TestRecord,
    ) -> Result<TestManagementService<'a>> {
        let user = record.executed_by.as_deref().map(uri::user_uri);
        let named: Vec<&str> = [
            record.test_case.as_ref().map(|t| t.uri.as_str()),
            record.defect.as_ref().map(|d| d.uri.as_str()),
            user.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect();
        self.polarion
            .check_write_targets(operation, run_uri, &named)?;
        Ok(self.service())
    }

    // ---------------------------------------------------------------- reads

    /// A test run by project and id: all fields (`fields` `None`) or only `fields`.
    pub async fn get(&self, project: &str, id: &str, fields: Option<&Fields>) -> Result<TestRun> {
        let s = self.service();
        let run = match fields {
            None => s.get_test_run_by_id(project, id).await?,
            Some(f) => {
                s.get_test_run_by_id_with_fields(project, id, f.keys())
                    .await?
            }
        };
        found(run, format!("{project}/{id}"))
    }

    pub async fn get_by_uri(&self, uri: &str, fields: Option<&Fields>) -> Result<TestRun> {
        let s = self.service();
        let run = match fields {
            None => s.get_test_run_by_uri(uri).await?,
            Some(f) => s.get_test_run_by_uri_with_fields(uri, f.keys()).await?,
        };
        found(run, uri)
    }

    /// Test runs matching a Lucene query (all fields unless [`Query::fields`]).
    pub async fn search(&self, query: &Query) -> Result<Vec<TestRun>> {
        let s = self.service();
        let (q, sort, limit) = (query.lucene(), query.sort_key(), query.results_limit());
        Ok(runs(match (query.field_list(), query.limit_value()) {
            (None, None) => s.search_test_runs(&q, sort).await?,
            (None, Some(_)) => s.search_test_runs_limited(&q, sort, limit).await?,
            (Some(f), None) => s.search_test_runs_with_fields(&q, sort, f.keys()).await?,
            (Some(f), Some(_)) => {
                s.search_test_runs_with_fields_limited(&q, sort, f.keys(), limit)
                    .await?
            }
        }))
    }

    /// Test run templates matching a Lucene query.
    pub async fn search_templates(&self, query: &Query) -> Result<Vec<TestRun>> {
        let s = self.service();
        let (q, sort, limit) = (query.lucene(), query.sort_key(), query.results_limit());
        Ok(runs(match (query.field_list(), query.limit_value()) {
            (None, None) => s.search_test_run_templates(&q, sort).await?,
            (None, Some(_)) => s.search_test_run_templates_limited(&q, sort, limit).await?,
            (Some(f), None) => {
                s.search_test_run_templates_with_fields(&q, sort, f.keys())
                    .await?
            }
            (Some(f), Some(_)) => {
                s.search_test_run_templates_with_fields_limited(&q, sort, f.keys(), limit)
                    .await?
            }
        }))
    }

    /// The records of one test case in a test run.
    pub async fn records(&self, run_uri: &str, test_case: &ItemRef) -> Result<Vec<TestRecord>> {
        let records = self
            .service()
            .get_test_case_records(run_uri, &test_case.uri)
            .await?;
        Ok(records.into_iter().map(TestRecord::from).collect())
    }

    /// Test records across test runs matching a query. (Deprecated by Polarion; there is no
    /// replacement in the SOAP API.)
    pub async fn search_records(
        &self,
        query: &str,
        sort: &str,
        limit: u32,
    ) -> Result<Vec<TestRecord>> {
        #[allow(deprecated)]
        let records = self
            .service()
            .search_test_records(query, sort, index(limit))
            .await?;
        Ok(records.into_iter().map(TestRecord::from).collect())
    }

    pub async fn attachments(&self, run_uri: &str) -> Result<Vec<Attachment>> {
        let list = self.service().get_test_run_attachments(run_uri).await?;
        Ok(list.into_iter().map(Attachment::from).collect())
    }

    /// Attachment metadata (with a download URL) by file name.
    pub async fn attachment(&self, run_uri: &str, file_name: &str) -> Result<Attachment> {
        let a = self
            .service()
            .get_test_run_attachment(run_uri, file_name)
            .await?;
        Ok(a.into())
    }

    /// The report page content of a test run.
    pub async fn report(&self, run_uri: &str) -> Result<Text> {
        Ok(self
            .service()
            .get_wiki_content_for_test_run(run_uri)
            .await?
            .into())
    }

    /// The test steps of a test case.
    pub async fn steps(&self, test_case: &ItemRef) -> Result<TestSteps> {
        Ok(self.service().get_test_steps(&test_case.uri).await?.into())
    }

    /// The columns of test steps, for a work item type or the project default.
    pub async fn steps_configuration(
        &self,
        project: &str,
        work_item_type: Option<&str>,
    ) -> Result<Vec<EnumOption>> {
        let s = self.service();
        let options = match work_item_type {
            None => s.get_test_steps_configuration(project).await?,
            Some(t) => {
                s.get_test_steps_configuration_by_work_item_type(project, t)
                    .await?
            }
        };
        Ok(options.into_iter().map(EnumOption::from).collect())
    }

    /// The project's test management configuration.
    pub async fn configuration(&self, project: &str) -> Result<soap::TestsConfiguration> {
        Ok(self.service().get_tests_configuration(project).await?)
    }

    /// Names of the parameters in the project's test parameter library.
    pub async fn parameters_library(&self, project: &str) -> Result<Vec<String>> {
        let library = self.service().get_test_parameters_library(project).await?;
        Ok(library
            .parameter_definitions
            .into_iter()
            .flatten()
            .filter_map(|d| d.name)
            .collect())
    }

    /// Names of the test parameters used in a test case.
    pub async fn parameter_names(&self, test_case: &ItemRef) -> Result<Vec<String>> {
        Ok(self
            .service()
            .get_test_case_parameter_names(&test_case.uri)
            .await?)
    }

    /// Custom fields defined for test runs (`project` `None`: global definitions).
    pub async fn custom_fields(&self, project: Option<&str>) -> Result<Vec<FieldDefinition>> {
        let list = self
            .service()
            .get_defined_test_run_custom_field_types(project)
            .await?;
        Ok(list.into_iter().map(FieldDefinition::from).collect())
    }

    pub async fn custom_field(&self, project: Option<&str>, key: &str) -> Result<FieldDefinition> {
        let f = self
            .service()
            .get_defined_test_run_custom_field_type(project, key)
            .await?;
        if f.id.is_none() {
            return Err(not_found("test run custom field", key));
        }
        Ok(f.into())
    }

    pub async fn custom_field_keys(&self, project: Option<&str>) -> Result<Vec<String>> {
        Ok(self
            .service()
            .get_defined_test_run_custom_field_keys(project)
            .await?)
    }

    /// The custom fields of one test run.
    pub async fn custom_fields_of(&self, run_uri: &str) -> Result<Vec<FieldDefinition>> {
        let list = self
            .service()
            .get_test_run_custom_field_types(run_uri)
            .await?;
        Ok(list.into_iter().map(FieldDefinition::from).collect())
    }

    pub async fn custom_field_of(&self, run_uri: &str, key: &str) -> Result<FieldDefinition> {
        let f = self
            .service()
            .get_test_run_custom_field_type(run_uri, key)
            .await?;
        if f.id.is_none() {
            return Err(not_found("test run custom field", key));
        }
        Ok(f.into())
    }

    // --------------------------------------------------------------- writes

    /// Create a test run from a template; returns its URI.
    ///
    /// With `fields` ([`NewTestRun`]), the create and the `updateTestRun` that writes them run as
    /// one transaction in a Polarion session of its own (all or nothing).
    pub async fn create(&self, new: &NewTestRun) -> Result<String> {
        let operation = if new.title.is_some() {
            "createTestRunWithTitle"
        } else {
            "createTestRun"
        };
        if new.fields.title.is_some() {
            return Err(Error::Invalid(
                "a new test run's title is `title`, not `fields.title`".into(),
            ));
        }
        // Before the budget is taken: a refused create takes nothing.
        let keys = new
            .fields
            .custom_fields
            .keys()
            .map(String::as_str)
            .chain(new.fields.status.as_ref().map(|_| "status"));
        self.polarion.check_field_keys(operation, keys)?;
        if let Some(defect) = &new.fields.summary_defect
            && self.polarion.writes_restricted()
        {
            self.polarion
                .check_write_target("updateTestRun", &defect.uri)?;
        }
        self.polarion
            .check_create(operation, &new.project, CreateKind::TestRun)?;
        let new = new.clone();
        let writes = 1 + usize::from(!new.fields.is_empty());
        self.polarion
            .atomic(operation, writes, move |p| async move {
                // The guard ran above: the request is sent as it is.
                let s = p.soap().test_management();
                let uri = match &new.title {
                    Some(title) => {
                        s.create_test_run_with_title(
                            &new.project,
                            &new.id,
                            Some(title),
                            &new.template,
                        )
                        .await?
                    }
                    None => {
                        s.create_test_run(&new.project, &new.id, &new.template)
                            .await?
                    }
                };
                if !new.fields.is_empty() {
                    p.grant_created(CreateKind::TestRun, &uri)?;
                    p.tests().update(&uri, &new.fields).await?;
                }
                Ok(uri)
            })
            .await
    }

    pub async fn update(&self, run_uri: &str, update: &TestRunUpdate) -> Result<()> {
        let defect = update.summary_defect.as_ref().map(|d| d.uri.as_str());
        self.polarion
            .check_write_targets("updateTestRun", run_uri, defect.as_slice())?;
        let keys = update
            .custom_fields
            .keys()
            .map(String::as_str)
            .chain(update.status.as_ref().map(|_| "status"));
        self.polarion.check_field_keys("updateTestRun", keys)?;
        let s = self.service();
        let run = soap::TestRun {
            uri: Some(run_uri.to_owned()),
            title: update.title.clone(),
            type_: update.kind.as_deref().map(option_id),
            status: update.status.as_deref().map(option_id),
            group_id: update.group_id.clone(),
            query: update.query.clone(),
            select_test_cases_by: update.select_test_cases_by.as_deref().map(option_id),
            keep_in_history: update.keep_in_history,
            finished_on: update.finished_on.clone(),
            summary_defect_uri: update.summary_defect.as_ref().map(|d| d.uri.clone()),
            custom_fields: soap_custom_fields(&update.custom_fields)?,
            ..soap::TestRun::default()
        };
        Ok(s.update_test_run(&run).await?)
    }

    /// Set the report page content.
    pub async fn set_report(&self, run_uri: &str, content: &Text) -> Result<()> {
        let s = self.writable_run("updateWikiContentForTestRun", run_uri)?;
        Ok(
            s.update_wiki_content_for_test_run(run_uri, &soap_text(content))
                .await?,
        )
    }

    /// Add a test record (a test case result) to a test run.
    pub async fn add_record(&self, run_uri: &str, record: &TestRecord) -> Result<()> {
        let s = self.writable_record("addTestRecordToTestRun", run_uri, record)?;
        let record = soap_test_record(record)?;
        Ok(s.add_test_record_to_test_run(run_uri, &record).await?)
    }

    /// Replace the test record at `index`.
    pub async fn update_record(
        &self,
        run_uri: &str,
        index_: u32,
        record: &TestRecord,
    ) -> Result<()> {
        let s = self.writable_record("updateTestRecordAtIndex", run_uri, record)?;
        let record = soap_test_record(record)?;
        Ok(
            s.update_test_record_at_index(run_uri, index(index_), Some(&record))
                .await?,
        )
    }

    /// Remove the test record at `index`.
    pub async fn remove_record(&self, run_uri: &str, index_: u32) -> Result<()> {
        let s = self.writable_run("updateTestRecordAtIndex", run_uri)?;
        Ok(s.update_test_record_at_index(run_uri, index(index_), None)
            .await?)
    }

    /// Execute test cases: records results like a manual execution (status, defects, ...).
    pub async fn execute(&self, run_uri: &str, records: &[TestRecord]) -> Result<()> {
        let s = self.unbounded_run("executeTest", run_uri)?;
        let records: Vec<soap::TestRecord> = records
            .iter()
            .map(soap_test_record)
            .collect::<Result<_>>()?;
        Ok(s.execute_test(run_uri, &records).await?)
    }

    /// Set the test steps of a test case (replacing existing ones). Each step needs one
    /// cell per column of [`TestRuns::steps_configuration`].
    pub async fn set_steps(&self, test_case: &ItemRef, steps: &[Vec<Text>]) -> Result<()> {
        self.polarion
            .check_write_target("setTestSteps", &test_case.uri)?;
        let steps: Vec<soap::TestStep> = steps
            .iter()
            .map(|cells| soap::TestStep {
                values: Some(cells.iter().map(soap_text).collect()),
            })
            .collect();
        Ok(self
            .service()
            .set_test_steps(&test_case.uri, &steps)
            .await?)
    }

    pub async fn add_attachment(
        &self,
        run_uri: &str,
        file_name: &str,
        title: &str,
        data: &[u8],
    ) -> Result<()> {
        let s = self.writable_run("addAttachmentToTestRun", run_uri)?;
        Ok(
            s.add_attachment_to_test_run(run_uri, file_name, title, data)
                .await?,
        )
    }

    /// Change an attachment's title or content (`None` leaves it unchanged).
    pub async fn update_attachment(
        &self,
        run_uri: &str,
        file_name: &str,
        title: Option<&str>,
        data: Option<&[u8]>,
    ) -> Result<()> {
        let s = self.writable_run("updateTestRunAttachment", run_uri)?;
        Ok(
            s.update_test_run_attachment(run_uri, file_name, title, data)
                .await?,
        )
    }

    pub async fn delete_attachment(&self, run_uri: &str, file_name: &str) -> Result<()> {
        let s = self.writable_run("deleteTestRunAttachment", run_uri)?;
        Ok(s.delete_test_run_attachment(run_uri, file_name).await?)
    }

    pub async fn add_record_attachment(
        &self,
        run_uri: &str,
        record_index: u32,
        file_name: &str,
        title: &str,
        data: &[u8],
    ) -> Result<()> {
        let s = self.writable_run("addAttachmentToTestRecord", run_uri)?;
        Ok(
            s.add_attachment_to_test_record(run_uri, index(record_index), file_name, title, data)
                .await?,
        )
    }

    pub async fn delete_record_attachment(
        &self,
        run_uri: &str,
        record_index: u32,
        file_name: &str,
    ) -> Result<()> {
        let s = self.writable_run("deleteAttachmentFromTestRecord", run_uri)?;
        Ok(
            s.delete_attachment_from_test_record(run_uri, index(record_index), file_name)
                .await?,
        )
    }

    pub async fn add_step_attachment(
        &self,
        run_uri: &str,
        record_index: u32,
        step_index: u32,
        file_name: &str,
        title: &str,
        data: &[u8],
    ) -> Result<()> {
        let s = self.writable_run("addAttachmentToTestStep", run_uri)?;
        Ok(s.add_attachment_to_test_step(
            run_uri,
            index(record_index),
            index(step_index),
            file_name,
            title,
            data,
        )
        .await?)
    }

    pub async fn delete_step_attachment(
        &self,
        run_uri: &str,
        record_index: u32,
        step_index: u32,
        file_name: &str,
    ) -> Result<()> {
        let s = self.writable_run("deleteAttachmentFromTestStep", run_uri)?;
        Ok(s.delete_attachment_from_test_step(
            run_uri,
            index(record_index),
            index(step_index),
            file_name,
        )
        .await?)
    }

    /// Create (or return the existing) summary defect of a test run from a defect template.
    pub async fn create_summary_defect(
        &self,
        run_uri: &str,
        defect_template_uri: &str,
    ) -> Result<ItemRef> {
        let s = self.unbounded_run("createSummaryDefect", run_uri)?;
        let uri = s
            .create_summary_defect(run_uri, defect_template_uri)
            .await?;
        Ok(ItemRef::from_uri(uri))
    }

    /// Create or update the summary defect with failure statistics.
    pub async fn update_summary_defect(
        &self,
        run_uri: &str,
        source: &str,
        failures: u32,
        errors: u32,
        tests: u32,
        defect_template_uri: &str,
    ) -> Result<ItemRef> {
        let s = self.unbounded_run("updateSummaryDefect", run_uri)?;
        let uri = s
            .update_summary_defect(
                run_uri,
                source,
                index(failures),
                index(errors),
                index(tests),
                defect_template_uri,
            )
            .await?;
        Ok(ItemRef::from_uri(uri))
    }

    pub async fn add_project_to_span(&self, run_uri: &str, project: &str) -> Result<()> {
        let s = self.unbounded_run("addProjectToProjectSpan", run_uri)?;
        Ok(s.add_project_to_project_span(run_uri, project).await?)
    }

    pub async fn remove_project_from_span(&self, run_uri: &str, project: &str) -> Result<()> {
        let s = self.unbounded_run("removeProjectFromProjectSpan", run_uri)?;
        Ok(s.remove_project_from_project_span(run_uri, project).await?)
    }
}
