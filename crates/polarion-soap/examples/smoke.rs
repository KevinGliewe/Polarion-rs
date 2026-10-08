//! Smoke test of the SOAP client against a real Polarion server.
//!
//! ```sh
//! POLARION_URL=https://polarion.example.com/polarion \
//! POLARION_TOKEN=<personal access token> \
//! POLARION_PROJECT=MyProject \
//! cargo run -p polarion-soap --example smoke
//! ```
//!
//! Read-only: the client refuses every operation that may modify data, so this
//! is safe to run against a production server. It sends at most seven small
//! requests (login, project, document spaces, documents, 5 work items, one
//! work item with all fields, logout) and keeps going when one fails.
//!
//! `POLARION_INSECURE=1` accepts self-signed certificates.

use polarion_soap::types::AnyValue;
use polarion_soap::{Auth, Client, Error};

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

/// Short, single-line description of a custom field value.
fn describe(value: Option<&AnyValue>) -> String {
    let text = match value {
        None | Some(AnyValue::Nil) => "<empty>".to_owned(),
        Some(AnyValue::EnumOptionId(e)) => format!("enum {:?}", e.id.as_deref().unwrap_or("")),
        Some(AnyValue::ArrayOfEnumOptionId(v)) => format!(
            "enums {:?}",
            v.iter().filter_map(|e| e.id.as_deref()).collect::<Vec<_>>()
        ),
        Some(AnyValue::Text(t)) => {
            format!("text ({}) {} chars", t.type_, t.content.chars().count())
        }
        Some(AnyValue::Unknown { xsi_type, .. }) => format!("UNKNOWN type {xsi_type:?}"),
        Some(other) => format!("{other:?}"),
    };
    text.chars().take(80).collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("POLARION_URL")?;
    let token = std::env::var("POLARION_TOKEN")?;
    let project = std::env::var("POLARION_PROJECT")
        .ok()
        .filter(|p| !p.is_empty());
    let insecure = std::env::var("POLARION_INSECURE").is_ok_and(|v| v == "1");
    let client = Client::builder(&url, Auth::Token(token))
        .danger_accept_invalid_certs(insecure)
        .read_only(true)
        .build()?;
    println!("Services: {}\n", client.services_url());

    if report("Session.logInWithToken", client.login().await).is_none() {
        return Ok(());
    }

    if let Some(project) = &project {
        if let Some(p) = report(
            &format!("Project.getProject({project})"),
            client.project().get_project(project).await,
        ) {
            println!("      name: {:?}", p.name);
        }

        if let Some(spaces) = report(
            "Tracker.getDocumentSpaces",
            client.tracker().get_document_spaces(project).await,
        ) {
            println!(
                "      {} spaces: {:?}",
                spaces.len(),
                spaces.iter().take(10).collect::<Vec<_>>()
            );
        }

        let fields = ["id", "title", "moduleFolder", "moduleName"].map(String::from);
        if let Some(docs) = report(
            "Tracker.queryModules (5)",
            client
                .tracker()
                .query_modules(&format!("project.id:{project}"), "title", &fields, 5)
                .await,
        ) {
            for d in &docs {
                println!(
                    "      {}/{}: {:?}",
                    d.module_folder.as_deref().unwrap_or("?"),
                    d.module_name.as_deref().unwrap_or("?"),
                    d.title
                );
            }
        }

        let fields = ["id", "title", "type", "status"].map(String::from);
        let items = report(
            "Tracker.queryWorkItemsLimited (5)",
            client
                .tracker()
                .query_work_items_limited(&format!("project.id:{project}"), "id", &fields, 5)
                .await,
        );
        for item in items.iter().flatten() {
            println!(
                "      {} [{}] {:?}",
                item.id.as_deref().unwrap_or("?"),
                item.type_
                    .as_ref()
                    .and_then(|t| t.id.as_deref())
                    .unwrap_or("?"),
                item.title
            );
        }

        let first = items
            .as_ref()
            .and_then(|i| i.first())
            .and_then(|i| i.id.clone());
        if let Some(id) = first {
            if let Some(item) = report(
                &format!("Tracker.getWorkItemById({id}) (all fields)"),
                client.tracker().get_work_item_by_id(project, &id).await,
            ) {
                let custom = item.custom_fields.unwrap_or_default();
                println!("      {} custom fields:", custom.len());
                for c in &custom {
                    println!("        {} = {}", c.key, describe(c.value.as_ref()));
                }
            }
        }
    }

    report("Session.endSession", client.logout().await);
    Ok(())
}
