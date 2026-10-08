//! Command execution: arguments in, a JSON value and how to show it out.

use polarion::soap::Error as SoapError;
use polarion::{Auth, Polarion, WorkItemQuery};
use serde::Serialize;
use serde_json::{Value, json};

use crate::cli::{Cli, Command, DocCommand, ItemCommand, ProjectCommand, UserCommand};
use crate::render::{DOCUMENT_COLUMNS, DOCUMENT_ITEM_COLUMNS, ITEM_COLUMNS, PROJECT_COLUMNS, View};

/// A command's result.
#[derive(Debug)]
pub struct Output {
    pub value: Value,
    pub view: View,
    /// A hint for stderr, e.g. that results were cut off.
    pub note: Option<String>,
}

impl Output {
    fn new(value: impl Serialize, view: View) -> Self {
        Self {
            value: serde_json::to_value(value).expect("domain types always serialize"),
            view,
            note: None,
        }
    }
}

#[derive(Debug)]
pub enum Failure {
    /// Missing or invalid configuration or arguments.
    Config(String),
    Polarion(polarion::Error),
}

impl From<polarion::Error> for Failure {
    fn from(e: polarion::Error) -> Self {
        Failure::Polarion(e)
    }
}

impl From<SoapError> for Failure {
    fn from(e: SoapError) -> Self {
        Failure::Polarion(e.into())
    }
}

impl Failure {
    /// Stable error code for JSON output.
    pub fn code(&self) -> &'static str {
        match self {
            Failure::Config(_) => "config",
            Failure::Polarion(polarion::Error::NotFound { .. }) => "notFound",
            Failure::Polarion(polarion::Error::Soap(e)) => match e {
                SoapError::Fault { .. } => "fault",
                SoapError::Login(_) => "login",
                SoapError::ReadOnly { .. } => "readOnly",
                SoapError::Http(_) | SoapError::Status { .. } => "http",
                SoapError::Url(_) | SoapError::Config(_) => "config",
                _ => "other",
            },
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Failure::Polarion(polarion::Error::NotFound { .. }) => 3,
            _ => 1,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Failure::Config(m) => m.clone(),
            Failure::Polarion(e) => e.to_string(),
        }
    }
}

pub async fn run(cli: &Cli) -> Result<Output, Failure> {
    let polarion = connect(cli)?;
    let result = dispatch(cli, &polarion).await;
    // Every login opens a session on the server (which may hold a license); end it right away.
    let _ = polarion.logout().await;
    result
}

fn connect(cli: &Cli) -> Result<Polarion, Failure> {
    let url = cli
        .url
        .as_deref()
        .filter(|u| !u.is_empty())
        .ok_or_else(|| Failure::Config("no server: set POLARION_URL or pass --url".into()))?;
    let soap = polarion::soap::Client::builder(url, auth_from_env()?)
        .read_only(true)
        .danger_accept_invalid_certs(cli.insecure)
        .build()?;
    Ok(Polarion::from_client(soap))
}

fn auth_from_env() -> Result<Auth, Failure> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    if let Some(token) = var("POLARION_TOKEN") {
        return Ok(Auth::Token(token));
    }
    match (var("POLARION_USER"), var("POLARION_PASSWORD")) {
        (Some(user), Some(password)) => Ok(Auth::Password { user, password }),
        _ => Err(Failure::Config(
            "no credentials: set POLARION_TOKEN, or POLARION_USER and POLARION_PASSWORD".into(),
        )),
    }
}

fn project(cli: &Cli) -> Result<&str, Failure> {
    cli.project
        .as_deref()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| {
            Failure::Config(
                "no project: pass -p/--project, set POLARION_PROJECT, or use PROJECT/ID".into(),
            )
        })
}

/// `ID` or `PROJECT/ID`.
fn item_id<'a>(cli: &'a Cli, arg: &'a str) -> Result<(&'a str, &'a str), Failure> {
    match arg.split_once('/') {
        Some((p, id)) if !p.is_empty() && !id.is_empty() && !id.contains('/') => Ok((p, id)),
        Some(_) => Err(Failure::Config(format!(
            "invalid work item `{arg}`: expected ID or PROJECT/ID"
        ))),
        None => Ok((project(cli)?, arg)),
    }
}

/// `SPACE/NAME` or `PROJECT/SPACE/NAME`.
fn document_ref<'a>(cli: &'a Cli, arg: &'a str) -> Result<(&'a str, &'a str, &'a str), Failure> {
    let parts: Vec<&str> = arg.split('/').collect();
    if parts.iter().all(|p| !p.is_empty()) {
        match parts[..] {
            [space, name] => return Ok((project(cli)?, space, name)),
            [project, space, name] => return Ok((project, space, name)),
            _ => {}
        }
    }
    Err(Failure::Config(format!(
        "invalid document `{arg}`: expected SPACE/NAME or PROJECT/SPACE/NAME"
    )))
}

async fn dispatch(cli: &Cli, polarion: &Polarion) -> Result<Output, Failure> {
    Ok(match &cli.command {
        Command::Project(ProjectCommand::Show { id }) => {
            let id = match id {
                Some(id) => id.as_str(),
                None => project(cli)?,
            };
            Output::new(polarion.project(id).await?, View::Object)
        }
        Command::Project(ProjectCommand::List) => {
            Output::new(polarion.projects().await?, View::Table(PROJECT_COLUMNS))
        }
        Command::User(UserCommand::Show { id }) => {
            Output::new(polarion.user(id).await?, View::Object)
        }
        Command::Item(ItemCommand::Show { id }) => {
            let (project, id) = item_id(cli, id)?;
            Output::new(polarion.work_item(project, id).await?, View::Object)
        }
        Command::Item(ItemCommand::Query {
            query,
            sort,
            limit,
            all,
            fields,
        }) => {
            let mut q = WorkItemQuery::new(query.as_str())
                .sort(sort.as_str())
                .fields(fields.to_fields());
            if let Some(p) = &cli.project {
                q = q.project(p.as_str());
            }
            q = if *all { q.unlimited() } else { q.limit(*limit) };
            let items = polarion.query_work_items(&q).await?;
            let truncated = !all && items.len() == *limit as usize;
            let mut out = Output::new(items, View::Table(ITEM_COLUMNS));
            if truncated {
                out.note = Some(format!(
                    "showing the first {limit} results; use --limit N or --all for more"
                ));
            }
            out
        }
        Command::Item(ItemCommand::Count { query }) => {
            let mut q = WorkItemQuery::new(query.as_str());
            if let Some(p) = &cli.project {
                q = q.project(p.as_str());
            }
            Output::new(polarion.count_work_items(&q).await?, View::Scalar)
        }
        Command::Item(ItemCommand::Links { id }) => {
            let (project, id) = item_id(cli, id)?;
            let item = polarion.work_item(project, id).await?;
            let incoming = match &item.uri {
                Some(uri) => polarion.back_links(uri).await?,
                None => Vec::new(),
            };
            Output::new(
                json!({"outgoing": item.links, "incoming": incoming}),
                View::Links,
            )
        }
        Command::Doc(DocCommand::Spaces) => {
            Output::new(polarion.spaces(project(cli)?).await?, View::Lines)
        }
        Command::Doc(DocCommand::List { space }) => Output::new(
            polarion.documents(project(cli)?, space.as_deref()).await?,
            View::Table(DOCUMENT_COLUMNS),
        ),
        Command::Doc(DocCommand::Show { document, body }) => {
            let (project, space, name) = document_ref(cli, document)?;
            Output::new(
                polarion.document(project, space, name, *body).await?,
                View::Object,
            )
        }
        Command::Doc(DocCommand::Items { document, fields }) => {
            let (project, space, name) = document_ref(cli, document)?;
            let doc = polarion.document(project, space, name, false).await?;
            let uri = doc.uri.ok_or_else(|| polarion::Error::NotFound {
                kind: "document",
                id: document.clone(),
            })?;
            let items = polarion
                .document_work_items(&uri, &fields.to_fields())
                .await?;
            Output::new(items, View::Table(DOCUMENT_ITEM_COLUMNS))
        }
    })
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    fn cli(args: &[&str]) -> Cli {
        Cli::try_parse_from([&["polarion"], args].concat()).unwrap()
    }

    #[test]
    fn work_item_ids() {
        let with = cli(&["-p", "P", "item", "show", "x"]);
        assert_eq!(item_id(&with, "P-1").unwrap(), ("P", "P-1"));
        assert_eq!(item_id(&with, "Q/Q-1").unwrap(), ("Q", "Q-1"));
        assert!(item_id(&with, "Q/").is_err());
        assert!(item_id(&with, "a/b/c").is_err());
    }

    #[test]
    fn document_refs() {
        let with = cli(&["-p", "P", "doc", "spaces"]);
        assert_eq!(
            document_ref(&with, "Specs/SRS").unwrap(),
            ("P", "Specs", "SRS")
        );
        assert_eq!(
            document_ref(&with, "Q/Specs/SRS").unwrap(),
            ("Q", "Specs", "SRS")
        );
        assert!(document_ref(&with, "SRS").is_err());
        assert!(document_ref(&with, "Specs//SRS").is_err());
    }
}
