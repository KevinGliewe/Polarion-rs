//! `plans.*`: planning.

use polarion::{NewPlan, PlanUpdate};
use polarion_commands_derive::{cmd, cmd_args};

use crate::{Context, Done, ItemArg, JsonObject, PLAN_COLUMNS, QueryArgs, Result, RichText, Run};

/// A plan by id.
#[cmd(view = object)]
pub struct Get {
    /// Plan id
    #[positional]
    pub id: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for Get {
    type Output = polarion::Plan;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .plans()
            .get(&cx.project(self.project.as_deref())?, &self.id)
            .await?)
    }
}

/// A plan by URI.
#[cmd(view = object)]
pub struct GetByUri {
    /// Plan URI
    #[positional]
    pub plan: String,
}

impl Run for GetByUri {
    type Output = polarion::Plan;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.plans().get_by_uri(&self.plan).await?)
    }
}

/// Search plans with a Lucene query.
#[cmd(view = table(PLAN_COLUMNS))]
pub struct Search {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for Search {
    type Output = Vec<polarion::Plan>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.plans().search(&self.search.query(cx)).await?)
    }
}

/// Search plan templates.
#[cmd(view = table(PLAN_COLUMNS))]
pub struct SearchTemplates {
    #[flatten]
    pub search: QueryArgs,
}

impl Run for SearchTemplates {
    type Output = Vec<polarion::Plan>;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx
            .polarion
            .plans()
            .search_templates(&self.search.query(cx))
            .await?)
    }
}

/// Progress statistics of a plan.
#[cmd(view = object)]
pub struct Statistics {
    /// Plan URI
    #[positional]
    pub plan: String,
}

impl Run for Statistics {
    type Output = polarion::soap::types::PlanStatistics;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.plans().statistics(&self.plan).await?)
    }
}

/// The page content of a plan.
#[cmd(view = object)]
pub struct Report {
    /// Plan URI
    #[positional]
    pub plan: String,
}

impl Run for Report {
    type Output = polarion::Text;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.plans().report(&self.plan).await?)
    }
}

/// Whether a plan was started.
#[cmd(view = scalar)]
pub struct WasStarted {
    /// Plan URI
    #[positional]
    pub plan: String,
}

impl Run for WasStarted {
    type Output = bool;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        Ok(cx.polarion.plans().was_started(&self.plan).await?)
    }
}

/// The fields of a plan other than its name, shared by `create` and `update`.
#[cmd_args]
pub struct PlanFields {
    /// Status id
    pub status: Option<String>,
    /// Description
    pub description: Option<RichText>,
    /// Start date
    pub start_date: Option<String>,
    /// Due date
    pub due_date: Option<String>,
    /// Capacity
    pub capacity: Option<f32>,
    /// Default estimate
    pub default_estimate: Option<f32>,
    /// Estimation field
    pub estimation_field: Option<String>,
    /// Prioritization field
    pub prioritization_field: Option<String>,
    /// Color
    pub color: Option<String>,
    /// Sort order
    pub sort_order: Option<i32>,
    /// Custom field values (typed from JSON; use {"enum": id} for enums)
    pub custom_fields: Option<JsonObject>,
}

impl PlanFields {
    /// The update these fields make, with `name`.
    fn into_update(self, name: Option<String>) -> Result<PlanUpdate> {
        Ok(PlanUpdate {
            name,
            status: self.status,
            description: self.description.map(polarion::Text::from),
            start_date: self.start_date,
            due_date: self.due_date,
            capacity: self.capacity,
            default_estimate: self.default_estimate,
            estimation_field: self.estimation_field,
            prioritization_field: self.prioritization_field,
            color: self.color,
            sort_order: self.sort_order,
            custom_fields: crate::values::custom_values(
                &self.custom_fields.unwrap_or_default().0,
                &[],
            )?
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect(),
        })
    }
}

/// Create a plan from a template; returns its URI. Fields a workflow may require are written in
/// the same transaction, right after the create.
#[cmd(write, view = scalar)]
pub struct Create {
    /// Plan id
    #[positional]
    pub id: String,
    /// Name
    pub name: String,
    /// Template id
    pub template: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Parent plan id
    pub parent: Option<String>,
    #[flatten]
    pub fields: PlanFields,
}

impl Run for Create {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let new = NewPlan {
            parent: self.parent,
            fields: self.fields.into_update(None)?,
            ..NewPlan::new(
                cx.project(self.project.as_deref())?,
                self.id,
                self.name,
                self.template,
            )
        };
        Ok(cx.polarion.plans().create(&new).await?)
    }
}

/// Create a plan template; returns its URI.
#[cmd(write, view = scalar)]
pub struct CreateTemplate {
    /// Template id
    #[positional]
    pub id: String,
    /// Name
    pub name: String,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
    /// Base template id
    pub template: Option<String>,
}

impl Run for CreateTemplate {
    type Output = String;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let template = self.template;
        Ok(cx
            .polarion
            .plans()
            .create_template(
                &cx.project(self.project.as_deref())?,
                &self.id,
                &self.name,
                template.as_deref(),
            )
            .await?)
    }
}

/// Change fields of a plan.
#[cmd(write, view = done)]
pub struct Update {
    /// Plan URI
    #[positional]
    pub plan: String,
    /// Name
    pub name: Option<String>,
    #[flatten]
    pub fields: PlanFields,
}

impl Run for Update {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        let update = self.fields.into_update(self.name)?;
        cx.polarion.plans().update(&self.plan, &update).await?;
        Ok(Done)
    }
}

/// Set the page content of a plan.
#[cmd(write, view = done)]
pub struct SetReport {
    /// Plan URI
    #[positional]
    pub plan: String,
    /// Text: a string (plain text) or {type: text/html, content}
    pub text: RichText,
}

impl Run for SetReport {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .plans()
            .set_report(&self.plan, &polarion::Text::from(self.text))
            .await?;
        Ok(Done)
    }
}

/// Delete plans by id.
#[cmd(write, view = done)]
pub struct Delete {
    /// Plan ids
    #[positional]
    pub ids: Vec<String>,
    /// Project id (default: POLARION_PROJECT or -p)
    pub project: Option<String>,
}

impl Run for Delete {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .plans()
            .delete(&cx.project(self.project.as_deref())?, &self.ids)
            .await?;
        Ok(Done)
    }
}

/// Plan work items.
#[cmd(write, view = done)]
pub struct AddItems {
    /// Plan URI
    #[positional]
    pub plan: String,
    /// Work items
    pub items: Vec<ItemArg>,
}

impl Run for AddItems {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .plans()
            .add_items(&self.plan, &ItemArg::resolve_all(&self.items, cx)?)
            .await?;
        Ok(Done)
    }
}

/// Unplan work items.
#[cmd(write, view = done)]
pub struct RemoveItems {
    /// Plan URI
    #[positional]
    pub plan: String,
    /// Work items
    pub items: Vec<ItemArg>,
}

impl Run for RemoveItems {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .plans()
            .remove_items(&self.plan, &ItemArg::resolve_all(&self.items, cx)?)
            .await?;
        Ok(Done)
    }
}

/// Allow a work item type.
#[cmd(write, view = done)]
pub struct AddAllowedType {
    /// Plan URI
    #[positional]
    pub plan: String,
    /// Work item type id
    pub r#type: String,
}

impl Run for AddAllowedType {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .plans()
            .add_allowed_type(&self.plan, &self.r#type)
            .await?;
        Ok(Done)
    }
}

/// Disallow a work item type.
#[cmd(write, view = done)]
pub struct RemoveAllowedType {
    /// Plan URI
    #[positional]
    pub plan: String,
    /// Work item type id
    pub r#type: String,
}

impl Run for RemoveAllowedType {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .plans()
            .remove_allowed_type(&self.plan, &self.r#type)
            .await?;
        Ok(Done)
    }
}

/// Add a project to the plan's span.
#[cmd(write, view = done)]
pub struct AddProjectToSpan {
    /// Plan URI
    #[positional]
    pub plan: String,
    /// Project id
    pub span_project: String,
}

impl Run for AddProjectToSpan {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .plans()
            .add_project_to_span(&self.plan, &self.span_project)
            .await?;
        Ok(Done)
    }
}

/// Remove a project from the plan's span.
#[cmd(write, view = done)]
pub struct RemoveProjectFromSpan {
    /// Plan URI
    #[positional]
    pub plan: String,
    /// Project id
    pub span_project: String,
}

impl Run for RemoveProjectFromSpan {
    type Output = Done;

    async fn run(self, cx: &Context<'_>) -> Result<Self::Output> {
        cx.polarion
            .plans()
            .remove_project_from_span(&self.plan, &self.span_project)
            .await?;
        Ok(Done)
    }
}
