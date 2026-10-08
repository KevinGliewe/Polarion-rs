//! Command-line arguments.

use clap::builder::BoolishValueParser;
use clap::{ArgAction, Args, Parser, Subcommand};
use polarion::{DEFAULT_LIMIT, Fields};

const AFTER_HELP: &str = "\
Credentials are read from the environment only:
  POLARION_TOKEN                       personal access token, or
  POLARION_USER and POLARION_PASSWORD  user name and password

The client is read-only: nothing on the server is changed.
Exit codes: 0 success, 1 error, 2 usage error, 3 not found.";

/// Read-only command-line access to Siemens Polarion ALM.
#[derive(Debug, Parser)]
#[command(name = "polarion", version, after_help = AFTER_HELP)]
pub struct Cli {
    /// Polarion base URL, e.g. https://polarion.example.com/polarion
    #[arg(
        long,
        env = "POLARION_URL",
        global = true,
        help_heading = "Global options"
    )]
    pub url: Option<String>,

    /// Project for ids without a project prefix
    #[arg(
        short,
        long,
        env = "POLARION_PROJECT",
        global = true,
        help_heading = "Global options"
    )]
    pub project: Option<String>,

    /// Accept invalid TLS certificates (test servers only)
    #[arg(
        long,
        env = "POLARION_INSECURE",
        global = true,
        help_heading = "Global options",
        action = ArgAction::SetTrue,
        value_parser = BoolishValueParser::new()
    )]
    pub insecure: bool,

    /// Print JSON (pretty on a terminal, compact otherwise)
    #[arg(long, global = true, help_heading = "Global options")]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Projects
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Users
    #[command(subcommand)]
    User(UserCommand),
    /// Work items
    #[command(subcommand)]
    Item(ItemCommand),
    /// Documents (live documents, called modules in the SOAP API)
    #[command(subcommand)]
    Doc(DocCommand),
}

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Show a project (default: the --project one)
    Show { id: Option<String> },
    /// List all projects (two requests per project group)
    List,
}

#[derive(Debug, Subcommand)]
pub enum UserCommand {
    /// Show a user
    Show { id: String },
}

#[derive(Debug, Subcommand)]
pub enum ItemCommand {
    /// Show a work item with all fields
    Show {
        /// ID or PROJECT/ID
        id: String,
    },
    /// Search work items with a Lucene query, within --project if given
    Query {
        /// Lucene query, e.g. "type:requirement AND status:open"; empty matches everything
        #[arg(default_value = "")]
        query: String,
        /// Sort order, e.g. id, ~updated (descending), "status id"
        #[arg(long, default_value = "id")]
        sort: String,
        /// Maximum number of results
        #[arg(long, default_value_t = DEFAULT_LIMIT, conflicts_with = "all")]
        limit: u32,
        /// Return every match
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        fields: FieldArgs,
    },
    /// Count work items matching a Lucene query, within --project if given
    Count {
        #[arg(default_value = "")]
        query: String,
    },
    /// Show the links of a work item in both directions
    Links {
        /// ID or PROJECT/ID
        id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum DocCommand {
    /// List the document spaces of the project
    Spaces,
    /// List documents of the project (all spaces unless --space)
    List {
        #[arg(long)]
        space: Option<String>,
    },
    /// Show a document
    Show {
        /// SPACE/NAME or PROJECT/SPACE/NAME
        document: String,
        /// Include the body (Polarion HTML)
        #[arg(long)]
        body: bool,
    },
    /// List the work items of a document in document order
    Items {
        /// SPACE/NAME or PROJECT/SPACE/NAME
        document: String,
        #[command(flatten)]
        fields: FieldArgs,
    },
}

#[derive(Debug, Args)]
pub struct FieldArgs {
    /// Fields to fetch: standard, all, or a comma-separated list of field keys
    #[arg(long, default_value = "standard")]
    pub fields: String,
    /// Custom fields to fetch in addition (comma-separated keys)
    #[arg(long, value_delimiter = ',')]
    pub custom: Vec<String>,
}

impl FieldArgs {
    pub fn to_fields(&self) -> Fields {
        let base = match self.fields.trim() {
            "standard" => Fields::standard(),
            "all" => Fields::all(),
            list => Fields::new(list.split(',').map(str::trim).filter(|k| !k.is_empty())),
        };
        self.custom
            .iter()
            .map(|k| k.trim())
            .filter(|k| !k.is_empty())
            .fold(base, Fields::with_custom)
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn arguments_are_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn field_selection() {
        let args = FieldArgs {
            fields: "id, title".into(),
            custom: vec!["asil".into(), " ".into()],
        };
        assert_eq!(
            args.to_fields().keys(),
            ["id", "title", "customFields.asil"]
        );
        let args = FieldArgs {
            fields: "standard".into(),
            custom: vec![],
        };
        assert_eq!(args.to_fields(), Fields::standard());
    }
}
