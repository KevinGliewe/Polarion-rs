//! Project configuration: custom field definitions, enumerations, categories, time points.

use crate::client::{Polarion, not_found};
use crate::error::Result;
use crate::model::{Category, EnumOption, FieldDefinition, TimePoint};
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
}
