//! Project configuration: custom field definitions, enumerations, categories, time points,
//! languages, workflow and time settings.

use crate::client::{Polarion, not_found};
use crate::convert::option_id;
use crate::error::Result;
use crate::model::{Category, EnumOption, FieldDefinition, TimePoint, WorkflowAction};
use crate::uri::ItemRef;

/// Schema and configuration: [`Polarion::schema`].
#[derive(Debug, Clone, Copy)]
pub struct Schema<'a> {
    polarion: &'a Polarion,
}

impl<'a> Schema<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    fn tracker(&self) -> polarion_soap::services::TrackerService<'a> {
        self.polarion.soap().tracker()
    }

    /// Custom fields defined for a work item type in a project.
    pub async fn custom_fields(
        &self,
        project: &str,
        work_item_type: &str,
    ) -> Result<Vec<FieldDefinition>> {
        let fields = self
            .tracker()
            .get_defined_custom_field_types(project, work_item_type)
            .await?;
        Ok(fields.into_iter().map(FieldDefinition::from).collect())
    }

    /// One custom field definition.
    pub async fn custom_field(
        &self,
        project: &str,
        work_item_type: &str,
        key: &str,
    ) -> Result<FieldDefinition> {
        let field = self
            .tracker()
            .get_defined_custom_field_type(project, work_item_type, key)
            .await?;
        if field.id.is_none() {
            return Err(not_found(
                "custom field",
                format!("{project}/{work_item_type}/{key}"),
            ));
        }
        Ok(field.into())
    }

    /// The custom fields of one work item (as defined for its type).
    pub async fn custom_fields_of(&self, item: &ItemRef) -> Result<Vec<FieldDefinition>> {
        let fields = self.tracker().get_custom_field_types(&item.uri).await?;
        Ok(fields.into_iter().map(FieldDefinition::from).collect())
    }

    /// All options of a work item field's enumeration in a project, e.g. for `type`,
    /// `status`, `priority`, `severity`, `resolution` or an enum custom field.
    pub async fn field_options(&self, project: &str, key: &str) -> Result<Vec<EnumOption>> {
        let options = self
            .tracker()
            .get_all_enum_options_for_key(project, key)
            .await?;
        Ok(options.into_iter().map(EnumOption::from).collect())
    }

    /// All options of an enumeration by its id (see [`FieldDefinition::enum_id`]).
    pub async fn enum_options(&self, project: &str, enum_id: &str) -> Result<Vec<EnumOption>> {
        let options = self
            .tracker()
            .get_all_enum_options_for_id(project, enum_id)
            .await?;
        Ok(options.into_iter().map(EnumOption::from).collect())
    }

    pub async fn categories(&self, project: &str) -> Result<Vec<Category>> {
        let categories = self.tracker().get_categories(project).await?;
        Ok(categories.into_iter().map(Category::from).collect())
    }

    pub async fn time_points(&self, project: &str) -> Result<Vec<TimePoint>> {
        let points = self.tracker().get_timepoints(project).await?;
        Ok(points.into_iter().map(TimePoint::from).collect())
    }

    /// The keys of the custom fields defined for a work item type.
    pub async fn custom_field_keys(
        &self,
        project: &str,
        work_item_type: &str,
    ) -> Result<Vec<String>> {
        Ok(self
            .tracker()
            .get_defined_custom_field_keys(project, work_item_type)
            .await?)
    }

    /// One custom field definition as it applies to a work item.
    pub async fn custom_field_of(&self, item: &ItemRef, key: &str) -> Result<FieldDefinition> {
        let field = self.tracker().get_custom_field_type(&item.uri, key).await?;
        if field.id.is_none() {
            return Err(not_found("custom field", format!("{item}/{key}")));
        }
        Ok(field.into())
    }

    /// Option ids of a work item field's enumeration (cheaper than [`Schema::field_options`]).
    pub async fn field_option_ids(&self, project: &str, key: &str) -> Result<Vec<String>> {
        let ids = self
            .tracker()
            .get_all_enum_option_ids_for_key(project, key)
            .await?;
        Ok(ids.into_iter().filter_map(|o| o.id).collect())
    }

    /// Option ids of an enumeration by its id.
    pub async fn enum_option_ids(&self, project: &str, enum_id: &str) -> Result<Vec<String>> {
        let ids = self
            .tracker()
            .get_all_enum_option_ids_for_id(project, enum_id)
            .await?;
        Ok(ids.into_iter().filter_map(|o| o.id).collect())
    }

    /// Options of a dependent field given the value of the field controlling it
    /// (see [`Schema::enum_control_key`]).
    pub async fn field_options_with_control(
        &self,
        project: &str,
        key: &str,
        control_value: &str,
    ) -> Result<Vec<EnumOption>> {
        let options = self
            .tracker()
            .get_enum_options_for_key_with_control(project, key, control_value)
            .await?;
        Ok(options.into_iter().map(EnumOption::from).collect())
    }

    /// Options of a dependent enumeration (by enum id) given the controlling value.
    pub async fn enum_options_with_control(
        &self,
        project: &str,
        enum_id: &str,
        control_value: &str,
    ) -> Result<Vec<EnumOption>> {
        let options = self
            .tracker()
            .get_enum_options_for_id_with_control(project, enum_id, control_value)
            .await?;
        Ok(options.into_iter().map(EnumOption::from).collect())
    }

    /// The key of the field that controls a dependent field's options.
    pub async fn enum_control_key(&self, project: &str, key: &str) -> Result<String> {
        Ok(self
            .tracker()
            .get_enum_control_key_for_key(project, key)
            .await?)
    }

    /// The key of the field that controls a dependent enumeration (by enum id).
    pub async fn enum_control_key_for_enum(&self, project: &str, enum_id: &str) -> Result<String> {
        Ok(self
            .tracker()
            .get_enum_control_key_for_id(project, enum_id)
            .await?)
    }

    /// The option of an enumeration (by enum id) that represents an object, e.g. a user.
    pub async fn object_option(&self, object_uri: &str, enum_id: &str) -> Result<EnumOption> {
        let o = self
            .tracker()
            .get_enum_option_from_object_uri_for_id(object_uri, enum_id)
            .await?;
        Ok(o.into())
    }

    /// The option that represents an object, for a field of a prototype (e.g. `WorkItem`).
    pub async fn object_option_for_key(
        &self,
        object_uri: &str,
        prototype: &str,
        key: &str,
    ) -> Result<EnumOption> {
        let o = self
            .tracker()
            .get_enum_option_from_object_uri_for_key(object_uri, prototype, key)
            .await?;
        Ok(o.into())
    }

    /// The workflow action performed when a work item of the type is created, if any.
    pub async fn initial_action(
        &self,
        project: &str,
        work_item_type: &str,
    ) -> Result<Option<WorkflowAction>> {
        let a = self
            .tracker()
            .get_initial_workflow_action_for_project_and_type(project, &option_id(work_item_type))
            .await?;
        Ok((a.action_name.is_some() || a.native_action_id.is_some()).then(|| a.into()))
    }

    // ---- languages

    pub async fn default_language(
        &self,
        project: &str,
    ) -> Result<polarion_soap::types::LanguageDefinition> {
        Ok(self
            .tracker()
            .get_default_language_definition(project)
            .await?)
    }

    pub async fn language(
        &self,
        project: &str,
        language: &str,
    ) -> Result<polarion_soap::types::LanguageDefinition> {
        Ok(self
            .tracker()
            .get_language_definition(project, language)
            .await?)
    }

    /// All language definitions except the default one.
    pub async fn languages(
        &self,
        project: &str,
    ) -> Result<Vec<polarion_soap::types::LanguageDefinition>> {
        Ok(self.tracker().get_language_definitions(project).await?)
    }

    /// The field that stores a work item type's title in a language.
    pub async fn localized_title_field(
        &self,
        project: &str,
        language: &str,
        work_item_type: &str,
    ) -> Result<String> {
        Ok(self
            .tracker()
            .get_localized_work_item_title_field(project, language, &option_id(work_item_type))
            .await?)
    }

    /// The field that stores a work item type's description in a language.
    pub async fn localized_description_field(
        &self,
        project: &str,
        language: &str,
        work_item_type: &str,
    ) -> Result<String> {
        Ok(self
            .tracker()
            .get_localized_work_item_description_field(
                project,
                language,
                &option_id(work_item_type),
            )
            .await?)
    }

    // ---- time and history settings

    /// A duration (e.g. `1d 4h`) in hours, according to the server's working-day length.
    pub async fn duration_hours(&self, duration: &str) -> Result<f32> {
        Ok(self.tracker().get_duration_hours(duration).await?)
    }

    /// The length of one working day.
    pub async fn one_day_length(&self) -> Result<i64> {
        Ok(self.tracker().get_one_day_length().await?)
    }

    /// Whether work item history is available on the server.
    pub async fn history_available(&self) -> Result<bool> {
        Ok(self.tracker().is_history_available().await?)
    }
}
