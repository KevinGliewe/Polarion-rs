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
//! is safe to run against a production server. It sends about twenty small
//! requests and keeps going when one fails. Lines starting with `CHECK` compare
//! an assumption the write operations rely on with what the server returns.
//!
//! Optional:
//! - `POLARION_DOCUMENT=space/name`: the document to read (default: the first one found).
//! - `POLARION_WORK_ITEM=ID`: the work item to read (default: the first one found).
//! - `POLARION_LIST_PROJECTS=1`: also list all projects (two requests per project group).
//! - `POLARION_INSECURE=1`: accept self-signed certificates.

use polarion::{Auth, DocumentRef, Error, Fields, ItemRef, Polarion, WorkItemQuery};
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
    // Read-only: no `allow_writes`.
    let polarion = Polarion::builder(&url, Auth::Token(token))
        .danger_accept_invalid_certs(var("POLARION_INSECURE").as_deref() == Some("1"))
        .build()?;
    let (items, documents, schema) = (polarion.items(), polarion.documents(), polarion.schema());

    if let Some(p) = report(
        &format!("projects().get({project})"),
        polarion.projects().get(&project).await,
    ) {
        show(&p);
    }

    if var("POLARION_LIST_PROJECTS").as_deref() == Some("1") {
        if let Some(projects) = report("projects().list()", polarion.projects().list().await) {
            let ids: Vec<_> = projects.iter().filter_map(|p| p.id.as_deref()).collect();
            println!("      {} projects: {:?}", ids.len(), ids);
        }
    }

    if let Some(spaces) = report("documents().spaces", documents.spaces(&project).await) {
        println!("      {} spaces", spaces.len());
    }

    // Documents: list, one with its body, and its work items.
    let docs = report(
        "documents().list (all spaces)",
        documents.list(&project, None).await,
    )
    .unwrap_or_default();
    println!("      {} documents, first 3:", docs.len());
    show(&docs.iter().take(3).collect::<Vec<_>>());

    let location = var("POLARION_DOCUMENT").or_else(|| {
        let d = docs.first()?;
        Some(format!("{}/{}", d.space.as_deref()?, d.name.as_deref()?))
    });
    if let Some((space, name)) = location.as_deref().and_then(|l| l.split_once('/')) {
        if let Some(mut doc) = report(
            &format!("documents().get({space}/{name}, with body)"),
            documents.get(&project, space, name, true).await,
        ) {
            let body = doc.body.take().map(|b| b.content.chars().count());
            show(&doc);
            println!("      body: {body:?} chars");
            check_uri(
                "DocumentRef::new",
                doc.uri.as_deref(),
                &DocumentRef::new(&project, space, name).uri,
            );

            if let Some(reference) = doc.reference() {
                if let Some(list) = report(
                    "documents().work_items (standard fields)",
                    documents.work_items(&reference, &Fields::standard()).await,
                ) {
                    println!("      {} work items, first 3:", list.len());
                    show(&list.iter().take(3).collect::<Vec<_>>());
                }
            }
        }
    }

    // Work items: query with the standard fields, then probes for wider field sets.
    let base = WorkItemQuery::new("").project(&project).sort("~updated");
    let recent = report(
        "items().query (5 most recently updated, standard fields)",
        items.query(&base.clone().limit(5)).await,
    );
    if let Some(list) = &recent {
        show(list);
    }
    if let Some(count) = report("items().count", items.count(&base).await) {
        println!("      {count} work items in {project}");
    }
    if let Some(refs) = report(
        "items().query_refs (3)",
        items.query_refs(&base.clone().limit(3)).await,
    ) {
        let refs: Vec<String> = refs.iter().map(ToString::to_string).collect();
        println!("      {refs:?}");
    }
    if let Some(list) = report(
        "PROBE items().query with Fields::all() (1)",
        items
            .query(&base.clone().limit(1).fields(Fields::all()))
            .await,
    ) {
        show(&list);
    }
    if let Some(list) = report(
        "PROBE items().query with fields [id, customFields] (3)",
        items
            .query(
                &base
                    .clone()
                    .limit(3)
                    .fields(Fields::new(["id", "customFields"])),
            )
            .await,
    ) {
        for item in &list {
            println!(
                "      {:?}: {} custom fields",
                item.id.as_deref().unwrap_or("?"),
                item.custom_fields.len()
            );
        }
    }

    // One work item with everything, its links, history, workflow and field definitions.
    let id = var("POLARION_WORK_ITEM").or_else(|| recent.as_ref()?.first()?.id.clone());
    if let Some(id) = id {
        let reference = ItemRef::new(&project, &id);
        if let Some(item) = report(&format!("items().get({id})"), items.get(&reference).await) {
            show(&item);
            check_uri("ItemRef::new", item.uri.as_deref(), &reference.uri);
            if let Some(kind) = &item.kind {
                if let Some(fields) = report(
                    &format!("schema().custom_fields({kind})"),
                    schema.custom_fields(&project, kind).await,
                ) {
                    show(&fields);
                }
            }
        }
        // These use the URI built by `ItemRef::new`, like every write does.
        if let Some(links) = report("items().back_links", items.back_links(&reference).await) {
            show(&links);
        }
        if let Some(changes) = report("items().history", items.history(&reference).await) {
            println!("      {} revisions, last 2:", changes.len());
            show(&changes.iter().rev().take(2).collect::<Vec<_>>());
        }
        if let Some(actions) = report("items().actions", items.actions(&reference).await) {
            show(&actions);
        }
        if let Some(actions) = report(
            "items().unavailable_actions",
            items.unavailable_actions(&reference).await,
        ) {
            show(&actions);
        }
    }

    for key in ["type", "status"] {
        if let Some(options) = report(
            &format!("schema().field_options({key})"),
            schema.field_options(&project, key).await,
        ) {
            let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
            println!("      {ids:?}");
        }
    }

    match items
        .get(&ItemRef::new(&project, "SMOKE-DOES-NOT-EXIST-1"))
        .await
    {
        Err(Error::NotFound { .. }) => println!("OK    items().get(missing) -> NotFound"),
        Err(e) => println!("FAIL  items().get(missing) -> expected NotFound, got: {e}"),
        Ok(_) => println!("FAIL  items().get(missing) -> expected NotFound, got a work item"),
    }

    report("logout", polarion.logout().await);
    Ok(())
}

/// Compare a URI built by the library with the one the server returned.
fn check_uri(what: &str, server: Option<&str>, built: &str) {
    match server {
        Some(server) if server == built => println!("CHECK {what} builds the server's URI"),
        Some(server) => println!("CHECK {what} MISMATCH: server `{server}`, built `{built}`"),
        None => println!("CHECK {what}: the server returned no URI"),
    }
}
