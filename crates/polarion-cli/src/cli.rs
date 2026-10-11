//! The command line: global options plus `polarion <area> <command> [args]`, the command tree
//! generated from the command DTOs (`polarion_commands::Cli`).

use clap::builder::BoolishValueParser;
use clap::{ArgAction, Args, Parser, Subcommand};
use polarion_commands::{Config, split_list, split_targets};

const AFTER_HELP: &str = "\
Configuration comes from the environment (or the global options):
  POLARION_URL, POLARION_PROJECT, POLARION_INSECURE
  POLARION_TOKEN, or POLARION_USER and POLARION_PASSWORD (environment only)
  POLARION_WRITE_PROJECTS        projects that may be written (default: read-only)
  POLARION_ALLOW_GLOBAL_WRITES   writes not tied to a project (users, global folders)
  POLARION_WRITE_TARGETS         only these objects may be written (set, even empty, it
                                 overrides the two above): ID, PROJECT/ID, doc:SPACE/NAME,
                                 doc:PROJECT/SPACE/NAME, run:ID, run:PROJECT/ID or a URI,
                                 comma-separated (\\, for a comma in a name); entries without
                                 a project are in POLARION_PROJECT (-p does not move them)
Write targets: refused even for them are creates, deletes of whole objects, workflow actions,
status and resolution changes, and the body of a document that holds work items. Allowed are
their attachments, work records and test records, deleting those included. Every object a write
names (link targets, test cases, defects, plan items) must be a target too. What the server does
in response (suspect links on work items that link to a target, notifications) is not prevented.

Work items are ID (in the default project), PROJECT/ID or a URI; documents SPACE/NAME,
PROJECT/SPACE/NAME or a URI. Text arguments such as --markdown also take @FILE (a file's
content) or @- (standard input); @@ stands for a literal @. `polarion commands` lists every
command.
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
    /// Comma-separated objects that may be written, and nothing else (even empty: no writes;
    /// overrides --write-projects and POLARION_WRITE_TARGETS): ID, PROJECT/ID, doc:SPACE/NAME,
    /// doc:PROJECT/SPACE/NAME, run:ID, run:PROJECT/ID or a URI; entries without a project are
    /// in the project of -p
    // Not read from POLARION_WRITE_TARGETS by clap: the environment's targets are resolved in
    // POLARION_PROJECT as it is in the environment (`Config::from_env`), never in a -p.
    #[arg(long, value_name = "TARGETS", global = true)]
    pub write_targets: Option<String>,
    /// Print JSON (pretty on a terminal, compact otherwise)
    #[arg(long, global = true)]
    pub json: bool,
    /// Write binary results (attachments, PDF) and text results (Markdown) to FILE
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
    /// merged the environment into them, except for the write targets).
    pub fn config(&self) -> Config {
        let mut config = Config::from_env();
        config.url = self.url.clone().or(config.url);
        // An empty -p (or POLARION_PROJECT, which clap passes on as it is) is no project.
        let project = self.project.clone().filter(|p| !p.trim().is_empty());
        config.project = project.or(config.project);
        config.insecure = self.insecure;
        if let Some(projects) = &self.write_projects {
            config.write_projects = split_list(projects);
        }
        config.global_writes = self.allow_global_writes;
        // Set, even empty, restricts writes, resolved in the project of the command line.
        // Without the option the environment decides (also when it is set but empty), resolved
        // in POLARION_PROJECT from the environment (`Config::from_env`): -p cannot move them.
        if let Some(targets) = &self.write_targets {
            config.write_targets = Some(split_targets(targets));
            config.write_targets_project = config.project.clone();
        }
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
    fn the_help_of_writes_says_so() {
        let opts = Opts::command();
        let mut writes = 0;
        for entry in polarion_commands::catalog().iter().filter(|e| e.write) {
            let command = opts
                .find_subcommand(entry.area)
                .and_then(|area| area.find_subcommand(entry.name))
                .unwrap_or_else(|| panic!("{} is not on the command line", entry.full_name()));
            // `-h`, and `--help` where the doc comment has more to say.
            let helps = [command.get_about(), command.get_long_about()];
            for help in helps.into_iter().flatten() {
                assert!(
                    help.to_string().ends_with(
                        "(Modifies data: needs POLARION_WRITE_PROJECTS for the project, or \
                         POLARION_WRITE_TARGETS naming the object.)"
                    ),
                    "{}: {help}",
                    entry.full_name()
                );
            }
            writes += 1;
        }
        assert!(writes > 50, "{writes}");
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
    fn the_help_of_text_arguments_names_the_file_forms() {
        let opts = Opts::command();
        let items = opts.find_subcommand("items").unwrap();
        for name in [
            "set-description-markdown",
            "append-description-markdown",
            "description-markdown",
        ] {
            let command = items.find_subcommand(name).unwrap();
            for help in [command.get_about(), command.get_long_about()]
                .into_iter()
                .flatten()
            {
                assert!(!help.to_string().contains("```"), "{name}: {help}");
            }
            if name == "description-markdown" {
                continue;
            }
            let markdown = command
                .get_arguments()
                .find(|a| a.get_id() == "markdown")
                .unwrap();
            let help = markdown.get_help().unwrap().to_string();
            assert!(
                help.ends_with(
                    "(also @FILE for a file's content, @- for standard input, @@... for text that \
                     starts with @)"
                ),
                "{name}: {help}"
            );
            assert!(markdown.is_allow_hyphen_values_set(), "{name}");
        }
        // Over JSON the description says nothing about files.
        let entry = polarion_commands::find("items.set-description-markdown").unwrap();
        let description = entry.input_schema()["properties"]["markdown"]["description"].clone();
        assert_eq!(description, "The new description: the Markdown text itself");
    }

    #[test]
    fn text_arguments_read_files() {
        let file =
            std::env::temp_dir().join(format!("polarion-cli-markdown-{}.md", std::process::id()));
        std::fs::write(&file, "# Brakes\n\nStop the car.\n").unwrap();
        let from_file = format!("@{}", file.display());
        let a = args(&[
            "items",
            "set-description-markdown",
            "P-1",
            "--markdown",
            &from_file,
            "--keep-orphans",
            "--id-prefixes",
            "MP,OP",
        ]);
        std::fs::remove_file(&file).unwrap();
        assert_eq!(
            a,
            json!({"item": "P-1", "markdown": "# Brakes\n\nStop the car.\n", "keep-orphans": true, "id-prefixes": ["MP", "OP"]})
        );
        let a = args(&[
            "items",
            "append-description-markdown",
            "P-1",
            "--markdown",
            "@@team, see above",
        ]);
        assert_eq!(a["markdown"], "@team, see above");
        let a = args(&["items", "description-markdown", "P-1", "--drop-html"]);
        assert_eq!(a, json!({"item": "P-1", "drop-html": true}));

        // A file that cannot be read is a usage error.
        let e = Opts::try_parse_from([
            "polarion",
            "items",
            "set-description-markdown",
            "P-1",
            "--markdown",
            &from_file,
        ])
        .unwrap_err();
        assert_eq!(e.exit_code(), 2);
        assert!(e.to_string().contains("cannot read"), "{e}");
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
