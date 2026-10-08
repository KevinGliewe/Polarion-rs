//! Planning: plans (releases, iterations) and plan templates.

use polarion_soap::services::PlanningService;
use polarion_soap::types as soap;

use crate::client::{Polarion, not_found};
use crate::convert::{option_id, soap_custom_fields, soap_text};
use crate::error::Result;
use crate::model::{Plan, Text};
use crate::query::Query;
use crate::uri::ItemRef;
use crate::write::PlanUpdate;

fn plans(list: Vec<soap::Plan>) -> Vec<Plan> {
    list.into_iter()
        .filter(|p| p.unresolvable != Some(true))
        .map(Plan::from)
        .collect()
}

fn found(plan: soap::Plan, id: impl std::fmt::Display) -> Result<Plan> {
    if plan.unresolvable == Some(true) || (plan.uri.is_none() && plan.id.is_none()) {
        return Err(not_found("plan", id));
    }
    Ok(plan.into())
}

/// Plans: [`Polarion::plans`]. Plans are addressed by URI ([`Plan::uri`]) or by project and id;
/// writes check the project against the write allowlist.
#[derive(Debug, Clone, Copy)]
pub struct Plans<'a> {
    polarion: &'a Polarion,
}

impl<'a> Plans<'a> {
    pub(crate) fn new(polarion: &'a Polarion) -> Self {
        Self { polarion }
    }

    fn service(&self) -> PlanningService<'a> {
        self.polarion.soap().planning()
    }

    fn writable(&self, operation: &'static str, plan_uri: &str) -> Result<PlanningService<'a>> {
        self.polarion.check_write_uri(operation, plan_uri)?;
        Ok(self.service())
    }

    // ---------------------------------------------------------------- reads

    pub async fn get(&self, project: &str, id: &str) -> Result<Plan> {
        found(
            self.service().get_plan_by_id(project, id).await?,
            format!("{project}/{id}"),
        )
    }

    pub async fn get_by_uri(&self, uri: &str) -> Result<Plan> {
        found(self.service().get_plan_by_uri(uri).await?, uri)
    }

    /// Plans matching a Lucene query (all fields unless [`Query::fields`]).
    pub async fn search(&self, query: &Query) -> Result<Vec<Plan>> {
        let s = self.service();
        let (q, sort, limit) = (query.lucene_opt(), query.sort_key(), query.results_limit());
        Ok(plans(match query.field_list() {
            None => s.search_plans(q.as_deref(), sort, limit).await?,
            Some(f) => {
                s.search_plans_with_fields(q.as_deref(), sort, limit, f.keys())
                    .await?
            }
        }))
    }

    /// Plan templates matching a Lucene query.
    pub async fn search_templates(&self, query: &Query) -> Result<Vec<Plan>> {
        let s = self.service();
        let (q, sort, limit) = (query.lucene_opt(), query.sort_key(), query.results_limit());
        Ok(plans(match query.field_list() {
            None => s.search_plan_templates(q.as_deref(), sort, limit).await?,
            Some(f) => {
                s.search_plan_templates_with_fields(q.as_deref(), sort, limit, f.keys())
                    .await?
            }
        }))
    }

    /// Progress statistics of a plan.
    pub async fn statistics(&self, uri: &str) -> Result<soap::PlanStatistics> {
        Ok(self.service().get_plan_statistics(uri).await?)
    }

    /// The plan's page content.
    pub async fn report(&self, uri: &str) -> Result<Text> {
        Ok(self.service().get_plan_wiki_content(uri).await?.into())
    }

    pub async fn was_started(&self, uri: &str) -> Result<bool> {
        Ok(self.service().was_plan_started(uri).await?)
    }

    // --------------------------------------------------------------- writes

    /// Create a plan from a template, optionally under a parent plan (by id); returns its URI.
    pub async fn create(
        &self,
        project: &str,
        id: &str,
        name: &str,
        parent: Option<&str>,
        template: &str,
    ) -> Result<String> {
        self.polarion.check_write("createPlan", Some(project))?;
        Ok(self
            .service()
            .create_plan(project, name, id, parent, template)
            .await?)
    }

    /// Create a plan template, optionally based on another template (by id); returns its URI.
    pub async fn create_template(
        &self,
        project: &str,
        id: &str,
        name: &str,
        template: Option<&str>,
    ) -> Result<String> {
        self.polarion
            .check_write("createPlanTemplate", Some(project))?;
        Ok(self
            .service()
            .create_plan_template(project, name, id, template)
            .await?)
    }

    pub async fn update(&self, uri: &str, update: &PlanUpdate) -> Result<()> {
        let s = self.writable("updatePlan", uri)?;
        let plan = soap::Plan {
            uri: Some(uri.to_owned()),
            name: update.name.clone(),
            status: update.status.as_deref().map(option_id),
            description: update.description.as_ref().map(soap_text),
            start_date: update.start_date.clone(),
            due_date: update.due_date.clone(),
            capacity: update.capacity,
            default_estimate: update.default_estimate,
            estimation_field: update.estimation_field.clone(),
            prioritization_field: update.prioritization_field.clone(),
            color: update.color.clone(),
            sort_order: update.sort_order,
            custom_fields: soap_custom_fields(&update.custom_fields)?,
            ..soap::Plan::default()
        };
        Ok(s.update_plan(&plan).await?)
    }

    /// Set the plan's page content.
    pub async fn set_report(&self, uri: &str, content: &Text) -> Result<()> {
        let s = self.writable("setPlanWikiContent", uri)?;
        Ok(s.set_plan_wiki_content(uri, &soap_text(content)).await?)
    }

    /// Delete plans of a project by id.
    pub async fn delete(&self, project: &str, ids: &[String]) -> Result<()> {
        self.polarion.check_write("deletePlans", Some(project))?;
        Ok(self.service().delete_plans(project, ids).await?)
    }

    /// Plan work items.
    pub async fn add_items(&self, uri: &str, items: &[ItemRef]) -> Result<()> {
        let s = self.writable("addPlanItems", uri)?;
        let uris: Vec<String> = items.iter().map(|i| i.uri.clone()).collect();
        Ok(s.add_plan_items(uri, &uris).await?)
    }

    pub async fn remove_items(&self, uri: &str, items: &[ItemRef]) -> Result<()> {
        let s = self.writable("removePlanItems", uri)?;
        let uris: Vec<String> = items.iter().map(|i| i.uri.clone()).collect();
        Ok(s.remove_plan_items(uri, &uris).await?)
    }

    /// Allow a work item type in the plan.
    pub async fn add_allowed_type(&self, uri: &str, work_item_type: &str) -> Result<()> {
        let s = self.writable("addPlanAllowedType", uri)?;
        Ok(s.add_plan_allowed_type(uri, &option_id(work_item_type))
            .await?)
    }

    pub async fn remove_allowed_type(&self, uri: &str, work_item_type: &str) -> Result<()> {
        let s = self.writable("removePlanAllowedType", uri)?;
        Ok(s.remove_plan_allowed_type(uri, &option_id(work_item_type))
            .await?)
    }

    pub async fn add_project_to_span(&self, uri: &str, project: &str) -> Result<()> {
        let s = self.writable("addProjectToPlanProjectSpan", uri)?;
        Ok(s.add_project_to_plan_project_span(uri, project).await?)
    }

    pub async fn remove_project_from_span(&self, uri: &str, project: &str) -> Result<()> {
        let s = self.writable("removeProjectFromPlanProjectSpan", uri)?;
        Ok(s.remove_project_from_plan_project_span(uri, project)
            .await?)
    }
}
