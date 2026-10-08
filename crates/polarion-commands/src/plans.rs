//! `plans.*`: planning.

use polarion::{PlanUpdate, Polarion};
use serde_json::Value;

use crate::args::Args;
use crate::catalog::*;
use crate::error::Result;
use crate::util::{command, out};

const PLAN: Param = Param::pos("plan", Kind::String, "Plan URI");
const TYPE: Param = Param::req("type", Kind::String, "Work item type id");
const SPAN_PROJECT: Param = Param::req("span-project", Kind::String, "Project id");

pub(crate) const COMMANDS: &[Command] = &[
    command!(
        "plans",
        "get",
        false,
        Output::Object,
        "A plan by id",
        [Param::pos("id", Kind::String, "Plan id"), PROJECT],
        get
    ),
    command!(
        "plans",
        "get-by-uri",
        false,
        Output::Object,
        "A plan by URI",
        [PLAN],
        get_by_uri
    ),
    command!(
        "plans",
        "search",
        false,
        Output::Table(PLAN_COLUMNS),
        "Search plans with a Lucene query",
        [QUERY, PROJECT, SORT, LIMIT, ALL, FIELDS],
        search
    ),
    command!(
        "plans",
        "search-templates",
        false,
        Output::Table(PLAN_COLUMNS),
        "Search plan templates",
        [QUERY, PROJECT, SORT, LIMIT, ALL, FIELDS],
        search_templates
    ),
    command!(
        "plans",
        "statistics",
        false,
        Output::Object,
        "Progress statistics of a plan",
        [PLAN],
        statistics
    ),
    command!(
        "plans",
        "report",
        false,
        Output::Object,
        "The page content of a plan",
        [PLAN],
        report
    ),
    command!(
        "plans",
        "was-started",
        false,
        Output::Scalar,
        "Whether a plan was started",
        [PLAN],
        was_started
    ),
    command!(
        "plans",
        "create",
        true,
        Output::Scalar,
        "Create a plan from a template; returns its URI",
        [
            Param::pos("id", Kind::String, "Plan id"),
            Param::req("name", Kind::String, "Name"),
            Param::req("template", Kind::String, "Template id"),
            PROJECT,
            Param::opt("parent", Kind::String, "Parent plan id")
        ],
        create
    ),
    command!(
        "plans",
        "create-template",
        true,
        Output::Scalar,
        "Create a plan template; returns its URI",
        [
            Param::pos("id", Kind::String, "Template id"),
            Param::req("name", Kind::String, "Name"),
            PROJECT,
            Param::opt("template", Kind::String, "Base template id")
        ],
        create_template
    ),
    command!(
        "plans",
        "update",
        true,
        Output::Done,
        "Change fields of a plan",
        [
            PLAN,
            Param::opt("name", Kind::String, "Name"),
            Param::opt("status", Kind::String, "Status id"),
            Param::opt("description", Kind::Text, "Description"),
            Param::opt("start-date", Kind::String, "Start date"),
            Param::opt("due-date", Kind::String, "Due date"),
            Param::opt("capacity", Kind::Number, "Capacity"),
            Param::opt("default-estimate", Kind::Number, "Default estimate"),
            Param::opt("estimation-field", Kind::String, "Estimation field"),
            Param::opt("prioritization-field", Kind::String, "Prioritization field"),
            Param::opt("color", Kind::String, "Color"),
            Param::opt("sort-order", Kind::Integer, "Sort order"),
            Param::opt(
                "custom-fields",
                Kind::Json,
                "Custom field values (typed from JSON; use {\"enum\": id} for enums)"
            ),
        ],
        update
    ),
    command!(
        "plans",
        "set-report",
        true,
        Output::Done,
        "Set the page content of a plan",
        [PLAN, TEXT],
        set_report
    ),
    command!(
        "plans",
        "delete",
        true,
        Output::Done,
        "Delete plans by id",
        [Param::pos("ids", Kind::StringList, "Plan ids"), PROJECT],
        delete
    ),
    command!(
        "plans",
        "add-items",
        true,
        Output::Done,
        "Plan work items",
        [PLAN, Param::req("items", Kind::ItemList, "Work items")],
        add_items
    ),
    command!(
        "plans",
        "remove-items",
        true,
        Output::Done,
        "Unplan work items",
        [PLAN, Param::req("items", Kind::ItemList, "Work items")],
        remove_items
    ),
    command!(
        "plans",
        "add-allowed-type",
        true,
        Output::Done,
        "Allow a work item type",
        [PLAN, TYPE],
        add_allowed_type
    ),
    command!(
        "plans",
        "remove-allowed-type",
        true,
        Output::Done,
        "Disallow a work item type",
        [PLAN, TYPE],
        remove_allowed_type
    ),
    command!(
        "plans",
        "add-project-to-span",
        true,
        Output::Done,
        "Add a project to the plan's span",
        [PLAN, SPAN_PROJECT],
        add_project_to_span
    ),
    command!(
        "plans",
        "remove-project-from-span",
        true,
        Output::Done,
        "Remove a project from the plan's span",
        [PLAN, SPAN_PROJECT],
        remove_project_from_span
    ),
];

async fn get(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.plans().get(&a.project()?, &a.str("id")?).await?)
}

async fn get_by_uri(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.plans().get_by_uri(&a.str("plan")?).await?)
}

async fn search(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.plans().search(&a.query()?).await?)
}

async fn search_templates(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.plans().search_templates(&a.query()?).await?)
}

async fn statistics(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.plans().statistics(&a.str("plan")?).await?)
}

async fn report(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.plans().report(&a.str("plan")?).await?)
}

async fn was_started(p: &Polarion, a: &Args) -> Result<Value> {
    out(p.plans().was_started(&a.str("plan")?).await?)
}

async fn create(p: &Polarion, a: &Args) -> Result<Value> {
    let parent = a.opt_str("parent")?;
    out(p
        .plans()
        .create(
            &a.project()?,
            &a.str("id")?,
            &a.str("name")?,
            parent.as_deref(),
            &a.str("template")?,
        )
        .await?)
}

async fn create_template(p: &Polarion, a: &Args) -> Result<Value> {
    let template = a.opt_str("template")?;
    out(p
        .plans()
        .create_template(
            &a.project()?,
            &a.str("id")?,
            &a.str("name")?,
            template.as_deref(),
        )
        .await?)
}

async fn update(p: &Polarion, a: &Args) -> Result<Value> {
    let update = PlanUpdate {
        name: a.opt_str("name")?,
        status: a.opt_str("status")?,
        description: a.opt_text("description")?,
        start_date: a.opt_str("start-date")?,
        due_date: a.opt_str("due-date")?,
        capacity: a.opt_f32("capacity")?,
        default_estimate: a.opt_f32("default-estimate")?,
        estimation_field: a.opt_str("estimation-field")?,
        prioritization_field: a.opt_str("prioritization-field")?,
        color: a.opt_str("color")?,
        sort_order: a
            .opt_i64("sort-order")?
            .map(|i| i32::try_from(i).unwrap_or(i32::MAX)),
        custom_fields: a
            .custom_values("custom-fields", &[])?
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect(),
    };
    p.plans().update(&a.str("plan")?, &update).await?;
    Ok(Value::Null)
}

async fn set_report(p: &Polarion, a: &Args) -> Result<Value> {
    p.plans()
        .set_report(&a.str("plan")?, &a.text("text")?)
        .await?;
    Ok(Value::Null)
}

async fn delete(p: &Polarion, a: &Args) -> Result<Value> {
    p.plans().delete(&a.project()?, &a.list("ids")?).await?;
    Ok(Value::Null)
}

async fn add_items(p: &Polarion, a: &Args) -> Result<Value> {
    p.plans()
        .add_items(&a.str("plan")?, &a.items("items")?)
        .await?;
    Ok(Value::Null)
}

async fn remove_items(p: &Polarion, a: &Args) -> Result<Value> {
    p.plans()
        .remove_items(&a.str("plan")?, &a.items("items")?)
        .await?;
    Ok(Value::Null)
}

async fn add_allowed_type(p: &Polarion, a: &Args) -> Result<Value> {
    p.plans()
        .add_allowed_type(&a.str("plan")?, &a.str("type")?)
        .await?;
    Ok(Value::Null)
}

async fn remove_allowed_type(p: &Polarion, a: &Args) -> Result<Value> {
    p.plans()
        .remove_allowed_type(&a.str("plan")?, &a.str("type")?)
        .await?;
    Ok(Value::Null)
}

async fn add_project_to_span(p: &Polarion, a: &Args) -> Result<Value> {
    p.plans()
        .add_project_to_span(&a.str("plan")?, &a.str("span-project")?)
        .await?;
    Ok(Value::Null)
}

async fn remove_project_from_span(p: &Polarion, a: &Args) -> Result<Value> {
    p.plans()
        .remove_project_from_span(&a.str("plan")?, &a.str("span-project")?)
        .await?;
    Ok(Value::Null)
}
