//! The command line, generated from the command catalog: `polarion <area> <command> [args]`.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::builder::BoolishValueParser;
use clap::{Arg, ArgAction, ArgMatches, Command as Clap};
use polarion_commands::{AREAS, Command, Config, Kind, commands, split_list};
use serde_json::{Map, Value};

const AFTER_HELP: &str = "\
Configuration comes from the environment (or the global options):
  POLARION_URL, POLARION_PROJECT, POLARION_INSECURE
  POLARION_TOKEN, or POLARION_USER and POLARION_PASSWORD (environment only)
  POLARION_WRITE_PROJECTS        projects that may be written (default: read-only)
  POLARION_ALLOW_GLOBAL_WRITES   writes not tied to a project (users, global folders)

Work items are ID (in the default project), PROJECT/ID or a URI; documents SPACE/NAME,
PROJECT/SPACE/NAME or a URI. `polarion commands` lists every command.
Exit codes: 0 success, 1 error, 2 usage error, 3 not found.";

/// Global options, not part of any command.
const GLOBAL: &[&str] = &[
    "url",
    "project",
    "insecure",
    "json",
    "write-projects",
    "allow-global-writes",
    "out",
];

fn global_args() -> Vec<Arg> {
    let heading = "Global options";
    vec![
        Arg::new("url")
            .long("url")
            .value_name("URL")
            .env("POLARION_URL")
            .global(true)
            .help_heading(heading)
            .help("Polarion base URL, e.g. https://polarion.example.com/polarion"),
        Arg::new("project")
            .short('p')
            .long("project")
            .value_name("PROJECT")
            .env("POLARION_PROJECT")
            .global(true)
            .help_heading(heading)
            .help("Project (default for ids without a project)"),
        Arg::new("insecure")
            .long("insecure")
            .env("POLARION_INSECURE")
            .action(ArgAction::SetTrue)
            .value_parser(BoolishValueParser::new())
            .global(true)
            .help_heading(heading)
            .help("Accept invalid TLS certificates (test servers only)"),
        Arg::new("write-projects")
            .long("write-projects")
            .value_name("PROJECTS")
            .env("POLARION_WRITE_PROJECTS")
            .global(true)
            .help_heading(heading)
            .help("Comma-separated projects that may be written (default: read-only)"),
        Arg::new("allow-global-writes")
            .long("allow-global-writes")
            .env("POLARION_ALLOW_GLOBAL_WRITES")
            .action(ArgAction::SetTrue)
            .value_parser(BoolishValueParser::new())
            .global(true)
            .help_heading(heading)
            .help("Allow writes not tied to a project (users, global folders, ...)"),
        Arg::new("json")
            .long("json")
            .action(ArgAction::SetTrue)
            .global(true)
            .help_heading(heading)
            .help("Print JSON (pretty on a terminal, compact otherwise)"),
        Arg::new("out")
            .long("out")
            .value_name("FILE")
            .global(true)
            .help_heading(heading)
            .help("Write binary results (attachments, PDF) to FILE"),
    ]
}

fn command_arg(index: &mut usize, p: &polarion_commands::Param) -> Arg {
    let mut arg = Arg::new(p.name).help(p.help.to_owned());
    if p.positional {
        *index += 1;
        arg = arg
            .index(*index)
            .required(p.required)
            .value_name(p.name.to_uppercase());
    } else {
        arg = arg
            .long(p.name)
            .required(p.required)
            .value_name(p.name.to_uppercase());
    }
    match p.kind {
        Kind::Boolean if !p.positional => arg
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value("true")
            .value_parser(BoolishValueParser::new()),
        Kind::StringList | Kind::ItemList => arg.action(ArgAction::Append).value_delimiter(','),
        Kind::Bytes => arg.value_name("BASE64|@FILE"),
        Kind::Json | Kind::Any => arg.value_name("JSON"),
        _ => arg,
    }
}

/// The full command tree.
pub fn build() -> Clap {
    let mut root = Clap::new("polarion")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Command-line access to Siemens Polarion ALM (read-only unless writes are enabled)")
        .after_help(AFTER_HELP)
        .subcommand_required(true)
        .arg_required_else_help(true)
        .args(global_args())
        .subcommand(
            Clap::new("commands")
                .about("List the commands (of one area)")
                .arg(Arg::new("area").index(1).help("Area, e.g. items")),
        );
    for (area, about) in AREAS {
        let mut sub = Clap::new(*area)
            .about(*about)
            .subcommand_required(true)
            .arg_required_else_help(true);
        for c in commands().iter().filter(|c| c.area == *area) {
            let mut index = 0;
            let args: Vec<Arg> = c
                .params
                .iter()
                // `project` is the global -p/--project option.
                .filter(|p| p.name != "project")
                .map(|p| command_arg(&mut index, p))
                .collect();
            sub = sub.subcommand(Clap::new(c.name).about(c.description()).args(args));
        }
        root = root.subcommand(sub);
    }
    root
}

/// What to do.
pub enum Invocation {
    /// `polarion commands [AREA]`.
    List(Option<String>),
    /// Run a catalog command.
    Run {
        command: &'static Command,
        args: Map<String, Value>,
    },
}

/// Options shared by every invocation.
pub struct Globals {
    pub config: Config,
    pub json: bool,
    pub out: Option<String>,
}

pub fn globals(m: &ArgMatches) -> Globals {
    let mut config = Config::from_env();
    // clap already merged the environment into these options.
    config.url = m.get_one::<String>("url").cloned().or(config.url);
    config.project = m.get_one::<String>("project").cloned().or(config.project);
    config.insecure = m.get_flag("insecure");
    if let Some(projects) = m.get_one::<String>("write-projects") {
        config.write_projects = split_list(projects);
    }
    config.global_writes = m.get_flag("allow-global-writes");
    Globals {
        config,
        json: m.get_flag("json"),
        out: m.get_one::<String>("out").cloned(),
    }
}

/// The invocation, with the command's arguments as JSON.
pub fn invocation(m: &ArgMatches) -> Result<Invocation, String> {
    let (area, area_matches) = m.subcommand().expect("subcommand_required");
    if area == "commands" {
        return Ok(Invocation::List(
            area_matches.get_one::<String>("area").cloned(),
        ));
    }
    let (name, cm) = area_matches.subcommand().expect("subcommand_required");
    let command =
        polarion_commands::find(&format!("{area}.{name}")).expect("built from the catalog");
    let mut args = Map::new();
    for p in command.params {
        if GLOBAL.contains(&p.name) {
            continue;
        }
        let value = match p.kind {
            Kind::Boolean if !p.positional => cm.get_one::<bool>(p.name).map(|b| Value::Bool(*b)),
            Kind::StringList | Kind::ItemList => cm
                .get_many::<String>(p.name)
                .map(|values| Value::Array(values.map(|v| Value::from(v.as_str())).collect())),
            _ => match cm.get_one::<String>(p.name) {
                None => None,
                Some(raw) => Some(convert(p.kind, raw)?),
            },
        };
        if let Some(value) = value {
            args.insert(p.name.to_owned(), value);
        }
    }
    Ok(Invocation::Run { command, args })
}

/// A command-line string as the JSON value the command expects.
fn convert(kind: Kind, raw: &str) -> Result<Value, String> {
    Ok(match kind {
        Kind::Json | Kind::Any => serde_json::from_str(raw).unwrap_or_else(|_| Value::from(raw)),
        Kind::Text if raw.trim_start().starts_with('{') => {
            serde_json::from_str(raw).unwrap_or_else(|_| Value::from(raw))
        }
        Kind::Bytes => match raw.strip_prefix('@') {
            Some(path) => {
                let data = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
                Value::from(BASE64.encode(data))
            }
            None => Value::from(raw),
        },
        _ => Value::from(raw),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Map<String, Value> {
        let m = build()
            .try_get_matches_from([&["polarion"], args].concat())
            .unwrap();
        match invocation(&m).unwrap() {
            Invocation::Run { args, .. } => args,
            Invocation::List(_) => panic!("expected a command"),
        }
    }

    #[test]
    fn tree_is_consistent() {
        build().debug_assert();
    }

    #[test]
    fn arguments_become_json() {
        let a = parse(&[
            "items",
            "query",
            "type:task",
            "--limit",
            "5",
            "--custom",
            "a,b",
            "--all",
        ]);
        assert_eq!(
            Value::Object(a),
            serde_json::json!({"query": "type:task", "limit": "5", "all": true, "custom": ["a", "b"]})
        );
        let a = parse(&[
            "items",
            "update",
            "P-1",
            "--custom-fields",
            r#"{"asil": "D"}"#,
        ]);
        assert_eq!(a["custom-fields"], serde_json::json!({"asil": "D"}));
        let a = parse(&["documents", "work-items", "Specs/SRS", "--deep=false"]);
        assert_eq!(a["deep"], Value::Bool(false));
    }
}
