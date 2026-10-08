//! `schema.*`: project configuration.

use polarion::Polarion;
use serde_json::Value;

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{command, out};

const TYPE: Param = Param::req("type", Kind::String, "Work item type id");
const KEY: Param = Param::req(
    "key",
    Kind::String,
    "Field key, e.g. `status` or a custom field id",
);
const ENUM_ID: Param = Param::req("enum-id", Kind::String, "Enumeration id");
const CONTROL: Param = Param::req("control", Kind::String, "Value of the controlling field");
const LANGUAGE: Param = Param::req("language", Kind::String, "Language id");

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "schema",
        "custom-fields",
        false,
        Output::Table(FIELD_COLUMNS),
        "Custom fields defined for a work item type",
        [PROJECT, TYPE],
        custom_fields
    ),
    command!(
        "schema",
        "custom-field",
        false,
        Output::Object,
        "One custom field definition",
        [PROJECT, TYPE, KEY],
        custom_field
    ),
    command!(
        "schema",
        "custom-field-keys",
        false,
        Output::Lines,
        "Keys of the custom fields of a work item type",
        [PROJECT, TYPE],
        custom_field_keys
    ),
    command!(
        "schema",
        "custom-fields-of",
        false,
        Output::Table(FIELD_COLUMNS),
        "Custom field definitions of one work item",
        [ITEM],
        custom_fields_of
    ),
    command!(
        "schema",
        "custom-field-of",
        false,
        Output::Object,
        "One custom field definition of a work item",
        [ITEM, KEY],
        custom_field_of
    ),
    command!(
        "schema",
        "field-options",
        false,
        Output::Table(OPTION_COLUMNS),
        "Options of a field's enumeration (e.g. type, status, priority)",
        [PROJECT, KEY],
        field_options
    ),
    command!(
        "schema",
        "field-option-ids",
        false,
        Output::Lines,
        "Option ids of a field's enumeration",
        [PROJECT, KEY],
        field_option_ids
    ),
    command!(
        "schema",
        "enum-options",
        false,
        Output::Table(OPTION_COLUMNS),
        "Options of an enumeration by id",
        [PROJECT, ENUM_ID],
        enum_options
    ),
    command!(
        "schema",
        "enum-option-ids",
        false,
        Output::Lines,
        "Option ids of an enumeration",
        [PROJECT, ENUM_ID],
        enum_option_ids
    ),
    command!(
        "schema",
        "field-options-with-control",
        false,
        Output::Table(OPTION_COLUMNS),
        "Options of a dependent field given the controlling value",
        [PROJECT, KEY, CONTROL],
        field_options_with_control
    ),
    command!(
        "schema",
        "enum-options-with-control",
        false,
        Output::Table(OPTION_COLUMNS),
        "Options of a dependent enumeration given the controlling value",
        [PROJECT, ENUM_ID, CONTROL],
        enum_options_with_control
    ),
    command!(
        "schema",
        "enum-control-key",
        false,
        Output::Scalar,
        "The field controlling a dependent field",
        [PROJECT, KEY],
        enum_control_key
    ),
    command!(
        "schema",
        "enum-control-key-for-enum",
        false,
        Output::Scalar,
        "The field controlling a dependent enumeration",
        [PROJECT, ENUM_ID],
        enum_control_key_for_enum
    ),
    command!(
        "schema",
        "object-option",
        false,
        Output::Object,
        "The enum option representing an object (e.g. a user)",
        [URI, ENUM_ID],
        object_option
    ),
    command!(
        "schema",
        "object-option-for-key",
        false,
        Output::Object,
        "The option representing an object for a prototype's field",
        [
            URI,
            Param::req("prototype", Kind::String, "Prototype, e.g. `WorkItem`"),
            KEY
        ],
        object_option_for_key
    ),
    command!(
        "schema",
        "initial-action",
        false,
        Output::Object,
        "The workflow action performed when a work item of a type is created",
        [PROJECT, TYPE],
        initial_action
    ),
    command!(
        "schema",
        "categories",
        false,
        Output::Table(OPTION_COLUMNS),
        "Work item categories",
        [PROJECT],
        categories
    ),
    command!(
        "schema",
        "time-points",
        false,
        Output::Table(OPTION_COLUMNS),
        "Time points (milestones)",
        [PROJECT],
        time_points
    ),
    command!(
        "schema",
        "default-language",
        false,
        Output::Object,
        "The default language",
        [PROJECT],
        default_language
    ),
    command!(
        "schema",
        "language",
        false,
        Output::Object,
        "One language definition",
        [PROJECT, LANGUAGE],
        language
    ),
    command!(
        "schema",
        "languages",
        false,
        Output::Object,
        "Language definitions (except the default)",
        [PROJECT],
        languages
    ),
    command!(
        "schema",
        "localized-title-field",
        false,
        Output::Scalar,
        "The field storing a type's title in a language",
        [PROJECT, LANGUAGE, TYPE],
        localized_title_field
    ),
    command!(
        "schema",
        "localized-description-field",
        false,
        Output::Scalar,
        "The field storing a type's description in a language",
        [PROJECT, LANGUAGE, TYPE],
        localized_description_field
    ),
    command!(
        "schema",
        "duration-hours",
        false,
        Output::Scalar,
        "A duration (e.g. `1d 4h`) in hours",
        [Param::pos("duration", Kind::String, "Duration")],
        duration_hours
    ),
    command!(
        "schema",
        "one-day-length",
        false,
        Output::Scalar,
        "The length of one working day",
        [],
        one_day_length
    ),
    command!(
        "schema",
        "history-available",
        false,
        Output::Scalar,
        "Whether history is available",
        [],
        history_available
    ),
];

async fn custom_fields(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .custom_fields(&a.project()?, &a.str("type")?)
        .await?)
}

async fn custom_field(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .custom_field(&a.project()?, &a.str("type")?, &a.str("key")?)
        .await?)
}

async fn custom_field_keys(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .custom_field_keys(&a.project()?, &a.str("type")?)
        .await?)
}

async fn custom_fields_of(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.schema().custom_fields_of(&a.item("item")?).await?)
}

async fn custom_field_of(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .custom_field_of(&a.item("item")?, &a.str("key")?)
        .await?)
}

async fn field_options(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .field_options(&a.project()?, &a.str("key")?)
        .await?)
}

async fn field_option_ids(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .field_option_ids(&a.project()?, &a.str("key")?)
        .await?)
}

async fn enum_options(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .enum_options(&a.project()?, &a.str("enum-id")?)
        .await?)
}

async fn enum_option_ids(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .enum_option_ids(&a.project()?, &a.str("enum-id")?)
        .await?)
}

async fn field_options_with_control(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .field_options_with_control(&a.project()?, &a.str("key")?, &a.str("control")?)
        .await?)
}

async fn enum_options_with_control(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .enum_options_with_control(&a.project()?, &a.str("enum-id")?, &a.str("control")?)
        .await?)
}

async fn enum_control_key(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .enum_control_key(&a.project()?, &a.str("key")?)
        .await?)
}

async fn enum_control_key_for_enum(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .enum_control_key_for_enum(&a.project()?, &a.str("enum-id")?)
        .await?)
}

async fn object_option(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .object_option(&a.str("uri")?, &a.str("enum-id")?)
        .await?)
}

async fn object_option_for_key(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .object_option_for_key(&a.str("uri")?, &a.str("prototype")?, &a.str("key")?)
        .await?)
}

async fn initial_action(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .initial_action(&a.project()?, &a.str("type")?)
        .await?)
}

async fn categories(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.schema().categories(&a.project()?).await?)
}

async fn time_points(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.schema().time_points(&a.project()?).await?)
}

async fn default_language(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.schema().default_language(&a.project()?).await?)
}

async fn language(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .language(&a.project()?, &a.str("language")?)
        .await?)
}

async fn languages(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.schema().languages(&a.project()?).await?)
}

async fn localized_title_field(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .localized_title_field(&a.project()?, &a.str("language")?, &a.str("type")?)
        .await?)
}

async fn localized_description_field(p: &Polarion, a: &Args) -> Result<Value> {
    out(p
        .schema()
        .localized_description_field(&a.project()?, &a.str("language")?, &a.str("type")?)
        .await?)
}

async fn duration_hours(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.schema().duration_hours(&a.str("duration")?).await?)
}

async fn one_day_length(p: &Polarion, _: &Args) -> Result<Value> {
    out(p.schema().one_day_length().await?)
}

async fn history_available(p: &Polarion, _: &Args) -> Result<Value> {
    out(p.schema().history_available().await?)
}
