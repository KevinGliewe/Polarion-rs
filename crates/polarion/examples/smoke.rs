//! Smoke test of the `polarion` library against a real server, printing the JSON form of the results.
//!
//! ```sh
//! POLARION_URL=https://polarion.example.com/polarion \
//! POLARION_TOKEN=<personal access token> \
//! POLARION_PROJECT=MyProject \
//! cargo run -p polarion --example smoke
//! ```
//!
//! Read-only: the client refuses every operation that may modify data, so this
//! is safe to run against a production server. It sends about a dozen small
//! requests and keeps going when one fails.
//!
//! Optional:
//! - `POLARION_DOCUMENT=space/name`: the document to read (default: the first one found).
//! - `POLARION_WORK_ITEM=ID`: the work item to read (default: the first one found).
//! - `POLARION_LIST_PROJECTS=1`: also list all projects (two requests per project group).
//! - `POLARION_INSECURE=1`: accept self-signed certificates.

use polarion::{Auth, Error, Fields, Polarion, WorkItemQuery};
use serde::Serialize;
use serde_json::Value;

const MAX_STRING: usize = 200;

fn report<T>(label: &str, result: Result<T, Error>) -> Option<T> {
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

/// Print the JSON form, with long strings shortened.
fn show(value: &impl Serialize) {
    fn shorten(v: &mut Value) {
        match v {
            Value::String(s) if s.chars().count() > MAX_STRING => {
                let total = s.chars().count();
                *s = format!(
                    "{}… ({total} chars)",
                    s.chars().take(MAX_STRING).collect::<String>()
                );
            }
            Value::Array(items) => items.iter_mut().for_each(shorten),
            Value::Object(map) => map.values_mut().for_each(shorten),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(value).expect("serializable");
    shorten(&mut v);
    let text = serde_json::to_string_pretty(&v).expect("serializable");
    for line in text.lines() {
        println!("      {line}");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let url = var("POLARION_URL").ok_or("POLARION_URL is not set")?;
    let token = var("POLARION_TOKEN").ok_or("POLARION_TOKEN is not set")?;
    let project = var("POLARION_PROJECT").ok_or("POLARION_PROJECT is not set")?;
    let soap = polarion::soap::Client::builder(&url, Auth::Token(token))
        .danger_accept_invalid_certs(var("POLARION_INSECURE").as_deref() == Some("1"))
        .read_only(true)
        .build()?;
    let polarion = Polarion::from_client(soap);

    if let Some(p) = report(
        &format!("project({project})"),
        polarion.project(&project).await,
    ) {
        show(&p);
    }

    if var("POLARION_LIST_PROJECTS").as_deref() == Some("1") {
        if let Some(projects) = report("projects()", polarion.projects().await) {
            let ids: Vec<_> = projects.iter().filter_map(|p| p.id.as_deref()).collect();
            println!("      {} projects: {:?}", ids.len(), ids);
        }
    }

    if let Some(spaces) = report("spaces", polarion.spaces(&project).await) {
        println!("      {} spaces", spaces.len());
    }

    // Documents: list, one with its body, and its work items.
    let documents = report(
        "documents (all spaces)",
        polarion.documents(&project, None).await,
    )
    .unwrap_or_default();
    println!("      {} documents, first 3:", documents.len());
    show(&documents.iter().take(3).collect::<Vec<_>>());

    let location = var("POLARION_DOCUMENT").or_else(|| {
        let d = documents.first()?;
        Some(format!("{}/{}", d.space.as_deref()?, d.name.as_deref()?))
    });
    if let Some((space, name)) = location.as_deref().and_then(|l| l.split_once('/')) {
        if let Some(mut doc) = report(
            &format!("document({space}/{name})"),
            polarion.document(&project, space, name).await,
        ) {
            let body = doc.body.take().map(|b| b.content.chars().count());
            show(&doc);
            println!("      body: {body:?} chars");

            if let Some(uri) = &doc.uri {
                if let Some(items) = report(
                    "document_work_items (standard fields)",
                    polarion.document_work_items(uri, &Fields::standard()).await,
                ) {
                    println!("      {} work items, first 3:", items.len());
                    show(&items.iter().take(3).collect::<Vec<_>>());
                }
            }
        }
    }

    // Work items: query with the standard fields, then probes for wider field sets.
    let base = WorkItemQuery::new("").project(&project).sort("~updated");
    let recent = report(
        "query_work_items (5 most recently updated, standard fields)",
        polarion.query_work_items(&base.clone().limit(5)).await,
    );
    if let Some(items) = &recent {
        show(items);
    }
    if let Some(count) = report("count_work_items", polarion.count_work_items(&base).await) {
        println!("      {count} work items in {project}");
    }
    if let Some(items) = report(
        "PROBE query_work_items with Fields::all() (1)",
        polarion
            .query_work_items(&base.clone().limit(1).fields(Fields::all()))
            .await,
    ) {
        show(&items);
    }
    if let Some(items) = report(
        "PROBE query_work_items with fields [id, customFields] (3)",
        polarion
            .query_work_items(
                &base
                    .clone()
                    .limit(3)
                    .fields(Fields::new(["id", "customFields"])),
            )
            .await,
    ) {
        for item in &items {
            println!(
                "      {:?}: {} custom fields",
                item.id.as_deref().unwrap_or("?"),
                item.custom_fields.len()
            );
        }
    }

    // One work item with everything, and its back links.
    let id = var("POLARION_WORK_ITEM").or_else(|| recent.as_ref()?.first()?.id.clone());
    if let Some(id) = id {
        if let Some(item) = report(
            &format!("work_item({id})"),
            polarion.work_item(&project, &id).await,
        ) {
            show(&item);
            if let Some(uri) = &item.uri {
                if let Some(links) = report("back_links", polarion.back_links(uri).await) {
                    show(&links);
                }
            }
        }
    }

    match polarion.work_item(&project, "SMOKE-DOES-NOT-EXIST-1").await {
        Err(Error::NotFound { .. }) => println!("OK    work_item(missing) -> NotFound"),
        Err(e) => println!("FAIL  work_item(missing) -> expected NotFound, got: {e}"),
        Ok(_) => println!("FAIL  work_item(missing) -> expected NotFound, got a work item"),
    }

    report("logout", polarion.logout().await);
    Ok(())
}
