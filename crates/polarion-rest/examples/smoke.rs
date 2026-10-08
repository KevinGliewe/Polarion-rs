//! Smoke test against a real Polarion server.
//!
//! ```sh
//! POLARION_URL=https://polarion.example.com/polarion \
//! POLARION_TOKEN=<personal access token> \
//! POLARION_PROJECT=MyProject \
//! cargo run -p polarion-rest --example smoke
//! ```
//!
//! Set `POLARION_INSECURE=1` to accept self-signed certificates.

use polarion_rest::models::WorkItemAttributes;
use polarion_rest::{Client, Query, ResourceKind};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("POLARION_URL")?;
    let token = std::env::var("POLARION_TOKEN")?;
    let insecure = std::env::var("POLARION_INSECURE").is_ok_and(|v| v == "1");
    let client = Client::builder(&url, &token)
        .danger_accept_invalid_certs(insecure)
        .build()?;
    println!("REST base: {}", client.base_url());

    // Newer endpoint; report rather than fail so version differences are visible.
    match client.get_current_user(&Query::new()).await {
        Ok(me) => println!("Authenticated as {:?}", me.data.id),
        Err(e) => println!("GET /user failed (may not exist on this Polarion version): {e}"),
    }

    let projects = client.get_projects(&Query::new().page_size(5)).await?;
    println!(
        "Projects: {} total, first page: {:?}",
        projects.total_count().unwrap_or_default(),
        projects
            .data
            .iter()
            .filter_map(|p| p.id.as_deref())
            .collect::<Vec<_>>()
    );

    if let Ok(project) = std::env::var("POLARION_PROJECT") {
        let items = client
            .get_work_items(
                &project,
                &Query::new()
                    .all_fields(WorkItemAttributes::TYPE)
                    .page_size(5),
            )
            .await?;
        println!(
            "Work items in {project}: {}",
            items.total_count().unwrap_or_default()
        );
        for item in &items.data {
            let Some(attrs) = &item.attributes else {
                continue;
            };
            println!(
                "  {:?} [{}] {:?} custom fields: {:?}",
                item.id,
                attrs.type_.as_deref().unwrap_or("?"),
                attrs.title,
                attrs.extra.keys().collect::<Vec<_>>()
            );
        }
    }
    Ok(())
}
