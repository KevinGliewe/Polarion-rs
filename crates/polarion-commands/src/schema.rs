//! `schema.*`: project configuration.

use polarion_commands_derive::cmd;

use crate::{Context, FIELD_COLUMNS, ItemArg, OPTION_COLUMNS, Result, Run};

/// Custom fields defined for a work item type.
#[cmd(view = table(FIELD_COLUMNS))]
pub struct CustomFields {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Work item type id
    pub r#type: String,
}

impl Run for CustomFields {
    type Output = Vec<polarion::FieldDefinition>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .custom_fields(&cx.project(self.project.as_deref())?, &self.r#type)
            .await?)
    }
}

/// One custom field definition.
#[cmd(view = object)]
pub struct CustomField {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Work item type id
    pub r#type: String,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
}

impl Run for CustomField {
    type Output = polarion::FieldDefinition;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .custom_field(
                &cx.project(self.project.as_deref())?,
                &self.r#type,
                &self.key,
            )
            .await?)
    }
}

/// Keys of the custom fields of a work item type.
#[cmd(view = lines)]
pub struct CustomFieldKeys {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Work item type id
    pub r#type: String,
}

impl Run for CustomFieldKeys {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .custom_field_keys(&cx.project(self.project.as_deref())?, &self.r#type)
            .await?)
    }
}

/// Custom field definitions of one work item.
#[cmd(view = table(FIELD_COLUMNS))]
pub struct CustomFieldsOf {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
}

impl Run for CustomFieldsOf {
    type Output = Vec<polarion::FieldDefinition>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .custom_fields_of(&self.item.resolve(cx)?)
            .await?)
    }
}

/// One custom field definition of a work item.
#[cmd(view = object)]
pub struct CustomFieldOf {
    /// Work item: ID (in the default project), PROJECT/ID, or URI
    #[positional]
    pub item: ItemArg,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
}

impl Run for CustomFieldOf {
    type Output = polarion::FieldDefinition;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .custom_field_of(&self.item.resolve(cx)?, &self.key)
            .await?)
    }
}

/// Options of a field's enumeration (e.g. type, status, priority)
#[cmd(view = table(OPTION_COLUMNS))]
pub struct FieldOptions {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
}

impl Run for FieldOptions {
    type Output = Vec<polarion::EnumOption>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .field_options(&cx.project(self.project.as_deref())?, &self.key)
            .await?)
    }
}

/// Option ids of a field's enumeration.
#[cmd(view = lines)]
pub struct FieldOptionIds {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
}

impl Run for FieldOptionIds {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .field_option_ids(&cx.project(self.project.as_deref())?, &self.key)
            .await?)
    }
}

/// Options of an enumeration by id.
#[cmd(view = table(OPTION_COLUMNS))]
pub struct EnumOptions {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Enumeration id
    pub enum_id: String,
}

impl Run for EnumOptions {
    type Output = Vec<polarion::EnumOption>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .enum_options(&cx.project(self.project.as_deref())?, &self.enum_id)
            .await?)
    }
}

/// Option ids of an enumeration.
#[cmd(view = lines)]
pub struct EnumOptionIds {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Enumeration id
    pub enum_id: String,
}

impl Run for EnumOptionIds {
    type Output = Vec<String>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .enum_option_ids(&cx.project(self.project.as_deref())?, &self.enum_id)
            .await?)
    }
}

/// Options of a dependent field given the controlling value.
#[cmd(view = table(OPTION_COLUMNS))]
pub struct FieldOptionsWithControl {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
    /// Value of the controlling field
    pub control: String,
}

impl Run for FieldOptionsWithControl {
    type Output = Vec<polarion::EnumOption>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .field_options_with_control(
                &cx.project(self.project.as_deref())?,
                &self.key,
                &self.control,
            )
            .await?)
    }
}

/// Options of a dependent enumeration given the controlling value.
#[cmd(view = table(OPTION_COLUMNS))]
pub struct EnumOptionsWithControl {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Enumeration id
    pub enum_id: String,
    /// Value of the controlling field
    pub control: String,
}

impl Run for EnumOptionsWithControl {
    type Output = Vec<polarion::EnumOption>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .enum_options_with_control(
                &cx.project(self.project.as_deref())?,
                &self.enum_id,
                &self.control,
            )
            .await?)
    }
}

/// The field controlling a dependent field.
#[cmd(view = scalar)]
pub struct EnumControlKey {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
}

impl Run for EnumControlKey {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .enum_control_key(&cx.project(self.project.as_deref())?, &self.key)
            .await?)
    }
}

/// The field controlling a dependent enumeration.
#[cmd(view = scalar)]
pub struct EnumControlKeyForEnum {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Enumeration id
    pub enum_id: String,
}

impl Run for EnumControlKeyForEnum {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .enum_control_key_for_enum(&cx.project(self.project.as_deref())?, &self.enum_id)
            .await?)
    }
}

/// The enum option representing an object (e.g. a user)
#[cmd(view = object)]
pub struct ObjectOption {
    /// Object URI
    #[positional]
    pub uri: String,
    /// Enumeration id
    pub enum_id: String,
}

impl Run for ObjectOption {
    type Output = polarion::EnumOption;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .object_option(&self.uri, &self.enum_id)
            .await?)
    }
}

/// The option representing an object for a prototype's field.
#[cmd(view = object)]
pub struct ObjectOptionForKey {
    /// Object URI
    #[positional]
    pub uri: String,
    /// Prototype, e.g. `WorkItem`
    pub prototype: String,
    /// Field key, e.g. `status` or a custom field id
    pub key: String,
}

impl Run for ObjectOptionForKey {
    type Output = polarion::EnumOption;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .object_option_for_key(&self.uri, &self.prototype, &self.key)
            .await?)
    }
}

/// The workflow action performed when a work item of a type is created.
#[cmd(view = object)]
pub struct InitialAction {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Work item type id
    pub r#type: String,
}

impl Run for InitialAction {
    type Output = Option<polarion::WorkflowAction>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .initial_action(&cx.project(self.project.as_deref())?, &self.r#type)
            .await?)
    }
}

/// Work item categories.
#[cmd(view = table(OPTION_COLUMNS))]
pub struct Categories {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for Categories {
    type Output = Vec<polarion::Category>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .categories(&cx.project(self.project.as_deref())?)
            .await?)
    }
}

/// Time points (milestones)
#[cmd(view = table(OPTION_COLUMNS))]
pub struct TimePoints {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for TimePoints {
    type Output = Vec<polarion::TimePoint>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .time_points(&cx.project(self.project.as_deref())?)
            .await?)
    }
}

/// The default language.
#[cmd(view = object)]
pub struct DefaultLanguage {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for DefaultLanguage {
    type Output = polarion::soap::types::LanguageDefinition;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .default_language(&cx.project(self.project.as_deref())?)
            .await?)
    }
}

/// One language definition.
#[cmd(view = object)]
pub struct Language {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Language id
    pub language: String,
}

impl Run for Language {
    type Output = polarion::soap::types::LanguageDefinition;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .language(&cx.project(self.project.as_deref())?, &self.language)
            .await?)
    }
}

/// Language definitions (except the default)
#[cmd(view = object)]
pub struct Languages {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for Languages {
    type Output = Vec<polarion::soap::types::LanguageDefinition>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .languages(&cx.project(self.project.as_deref())?)
            .await?)
    }
}

/// The field storing a type's title in a language.
#[cmd(view = scalar)]
pub struct LocalizedTitleField {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Language id
    pub language: String,
    /// Work item type id
    pub r#type: String,
}

impl Run for LocalizedTitleField {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .localized_title_field(
                &cx.project(self.project.as_deref())?,
                &self.language,
                &self.r#type,
            )
            .await?)
    }
}

/// The field storing a type's description in a language.
#[cmd(view = scalar)]
pub struct LocalizedDescriptionField {
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Language id
    pub language: String,
    /// Work item type id
    pub r#type: String,
}

impl Run for LocalizedDescriptionField {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .schema()
            .localized_description_field(
                &cx.project(self.project.as_deref())?,
                &self.language,
                &self.r#type,
            )
            .await?)
    }
}

/// A duration (e.g. `1d 4h`) in hours.
#[cmd(view = scalar)]
pub struct DurationHours {
    /// Duration
    #[positional]
    pub duration: String,
}

impl Run for DurationHours {
    type Output = f32;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.schema().duration_hours(&self.duration).await?)
    }
}

/// The length of one working day.
#[cmd(view = scalar)]
pub struct OneDayLength {}

impl Run for OneDayLength {
    type Output = i64;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.schema().one_day_length().await?)
    }
}

/// Whether history is available.
#[cmd(view = scalar)]
pub struct HistoryAvailable {}

impl Run for HistoryAvailable {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.schema().history_available().await?)
    }
}
