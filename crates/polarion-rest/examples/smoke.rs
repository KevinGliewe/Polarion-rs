//! Smoke test against a real Polarion server.
//!
//! ```sh
//! POLARION_URL=https://polarion.example.com/polarion \
//! POLARION_TOKEN=<personal access token> \
//! POLARION_PROJECT=MyProject \
//! POLARION_SAVE_SPEC=polarion-openapi.json \
//! cargo run -p polarion-rest --example smoke
//! ```
//!
//! Read-only: the client refuses anything but `GET`, so this is safe to run
//! against a production server. It sends at most six small `GET` requests and
//! keeps going when one fails, so a single permission error does not hide the
//! rest.
//!
//! - `POLARION_PROJECT` (optional): also read that project and five of its work items.
//! - `POLARION_SAVE_SPEC` (optional): save the server's OpenAPI definition to this path.
//! - `POLARION_INSECURE=1` (optional): accept self-signed certificates.

use std::fmt::Display;

use polarion_rest::models::WorkItemAttributes;
use polarion_rest::{Client, Method, Query, ResourceKind};

/// Print the outcome of one request and return its value on success.
fn report<T, E: Display>(label: &str, result: Result<T, E>) -> Option<T> {
    match result {
        Ok(value) => {
            println!("OK    {label}");
            Some(value)
        }
        Err(e) => {
            println!("FAIL  {label}: {e}");
            None
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("POLARION_URL")?;
    let token = std::env::var("POLARION_TOKEN")?;
    let project = std::env::var("POLARION_PROJECT")
        .ok()
        .filter(|p| !p.is_empty());
    let insecure = std::env::var("POLARION_INSECURE").is_ok_and(|v| v == "1");
    let client = Client::builder(&url, &token)
        .danger_accept_invalid_certs(insecure)
        .read_only(true)
        .build()?;
    println!("REST base: {}\n", client.base_url());

    if let Some(me) = report("GET /user", client.get_current_user(&Query::new()).await) {
        println!("      authenticated as {:?}", me.data.id);
    }

    if let Some(projects) = report(
        "GET /projects",
        client.get_projects(&Query::new().page_size(5)).await,
    ) {
        println!(
            "      {} total, first page: {:?}",
            projects.total_count().unwrap_or_default(),
            projects
                .data
                .iter()
                .filter_map(|p| p.id.as_deref())
                .collect::<Vec<_>>()
        );
    }

    if let Some(project) = &project {
        if let Some(p) = report(
            &format!("GET /projects/{project}"),
            client.get_project(project, &Query::new()).await,
        ) {
            println!("      name: {:?}", p.data.attributes.and_then(|a| a.name));
        }

        let items = report(
            &format!("GET /projects/{project}/workitems (5, all fields)"),
            client
                .get_work_items(
                    project,
                    &Query::new()
                        .all_fields(WorkItemAttributes::TYPE)
                        .page_size(5),
                )
                .await,
        );
        if let Some(items) = &items {
            println!(
                "      {} work items in total",
                items.total_count().unwrap_or_default()
            );
            for item in &items.data {
                let Some(attrs) = &item.attributes else {
                    continue;
                };
                println!(
                    "      {:?} [{}] {:?} custom fields: {:?}",
                    item.id,
                    attrs.type_.as_deref().unwrap_or("?"),
                    attrs.title,
                    attrs.extra.keys().collect::<Vec<_>>()
                );
            }
        }

        let first_id = items
            .as_ref()
            .and_then(|i| i.data.first())
            .and_then(|i| i.id.as_deref())
            .map(|id| id.rsplit('/').next().unwrap_or(id).to_owned());
        if let Some(id) = first_id {
            report(
                &format!("GET /projects/{project}/workitems/{id}"),
                client
                    .get_work_item(
                        project,
                        &id,
                        &Query::new().all_fields(WorkItemAttributes::TYPE),
                    )
                    .await,
            );
        }
    }

    if let Ok(path) = std::env::var("POLARION_SAVE_SPEC") {
        let spec = report(
            "GET /definition",
            client
                .request_json(Method::GET, "definition", &Query::new(), None)
                .await,
        );
        if let Some(Some(spec)) = spec {
            std::fs::write(&path, serde_json::to_string_pretty(&spec)?)?;
            println!("      saved to {path}");
        }
    }
    Ok(())
}
