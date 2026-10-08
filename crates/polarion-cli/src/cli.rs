//! The command line: global options plus `polarion <area> <command> [args]`, the command tree
//! generated from the command DTOs (`polarion_commands::Cli`).

use clap::builder::BoolishValueParser;
use clap::{ArgAction, Args, Parser, Subcommand};
use polarion_commands::{Config, split_list};

const AFTER_HELP: &str = "\
Configuration comes from the environment (or the global options):
  POLARION_URL, POLARION_PROJECT, POLARION_INSECURE
  POLARION_TOKEN, or POLARION_USER and POLARION_PASSWORD (environment only)
  POLARION_WRITE_PROJECTS        projects that may be written (default: read-only)
  POLARION_ALLOW_GLOBAL_WRITES   writes not tied to a project (users, global folders)

Work items are ID (in the default project), PROJECT/ID or a URI; documents SPACE/NAME,
PROJECT/SPACE/NAME or a URI. `polarion commands` lists every command.
Exit codes: 0 success, 1 error, 2 usage error, 3 not found.";

/// Command-line access to Siemens Polarion ALM (read-only unless writes are enabled)
#[derive(Debug, Parser)]
#[command(
    name = "polarion",
    version,
    after_help = AFTER_HELP,
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Opts {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Top,
}

/// Options shared by every command.
#[derive(Debug, Args)]
#[command(next_help_heading = "Global options")]
pub struct Global {
    /// Polarion base URL, e.g. https://polarion.example.com/polarion
    #[arg(long, value_name = "URL", env = "POLARION_URL", global = true)]
    pub url: Option<String>,
    /// Project (default for ids without a project)
    #[arg(
        short = 'p',
        long,
        value_name = "PROJECT",
        env = "POLARION_PROJECT",
        global = true
    )]
    pub project: Option<String>,
    /// Accept invalid TLS certificates (test servers only)
    #[arg(long, env = "POLARION_INSECURE", action = ArgAction::SetTrue,
        value_parser = BoolishValueParser::new(), global = true)]
    pub insecure: bool,
    /// Comma-separated projects that may be written (default: read-only)
    #[arg(
        long,
        value_name = "PROJECTS",
        env = "POLARION_WRITE_PROJECTS",
        global = true
    )]
    pub write_projects: Option<String>,
    /// Allow writes not tied to a project (users, global folders, ...)
    #[arg(long, env = "POLARION_ALLOW_GLOBAL_WRITES", action = ArgAction::SetTrue,
        value_parser = BoolishValueParser::new(), global = true)]
    pub allow_global_writes: bool,
    /// Print JSON (pretty on a terminal, compact otherwise)
    #[arg(long, global = true)]
    pub json: bool,
    /// Write binary results (attachments, PDF) to FILE
    #[arg(long, value_name = "FILE", global = true)]
    pub out: Option<String>,
}

// Parsed once per run: the size of the command variant does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
pub enum Top {
    /// List the commands (of one area)
    Commands {
        /// Area, e.g. items
        area: Option<String>,
    },
    #[command(flatten)]
    Run(polarion_commands::Cli),
}

impl Global {
    /// The connection settings: the environment, overridden by the options (clap already
    /// merged the environment into them).
    pub fn config(&self) -> Config {
        let mut config = Config::from_env();
        config.url = self.url.clone().or(config.url);
        config.project = self.project.clone().or(config.project);
        config.insecure = self.insecure;
        if let Some(projects) = &self.write_projects {
            config.write_projects = split_list(projects);
        }
        config.global_writes = self.allow_global_writes;
        config
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;
    use polarion_commands::AnyCommand;
    use serde_json::{Value, json};

    use super::*;

    fn args(argv: &[&str]) -> Value {
        let opts = Opts::try_parse_from([&["polarion"], argv].concat()).unwrap();
        let Top::Run(cli) = opts.command else {
            panic!("expected a command")
        };
        serde_json::to_value(AnyCommand::from(cli)).unwrap()["args"].clone()
    }

    #[test]
    fn tree_is_consistent() {
        Opts::command().debug_assert();
    }

    #[test]
    fn arguments_parse_into_the_dtos() {
        assert_eq!(
            args(&[
                "items",
                "query",
                "type:task",
                "--limit",
                "5",
                "--custom",
                "a,b",
                "--all"
            ]),
            json!({"query": "type:task", "limit": 5, "all": true, "custom": ["a", "b"]})
        );
        let a = args(&[
            "items",
            "update",
            "P-1",
            "--custom-fields",
            r#"{"asil": "D"}"#,
        ]);
        assert_eq!(a["custom-fields"], json!({"asil": "D"}));
        assert_eq!(a["item"], "P-1");
        let a = args(&["documents", "work-items", "Specs/SRS", "--deep=false"]);
        assert_eq!(a["deep"], Value::Bool(false));
        let a = args(&[
            "documents",
            "export-pdf",
            "Specs/SRS",
            "--orientation",
            "landscape",
        ]);
        assert_eq!(a["orientation"], "landscape");
    }

    #[test]
    fn bad_values_are_usage_errors() {
        for argv in [
            &["items", "query", "--limit", "many"][..],
            &[
                "documents",
                "export-pdf",
                "Specs/SRS",
                "--orientation",
                "diagonal",
            ],
            &["items", "get", "a/b/c"],
        ] {
            let e = Opts::try_parse_from([&["polarion"], argv].concat()).unwrap_err();
            assert_eq!(e.exit_code(), 2, "{argv:?}");
        }
    }
}
