//! Every public operation of the `polarion` library is reachable as a command: each
//! `pub async fn` of a handle is called by some command handler, or listed here with
//! the reason it is not.

use std::collections::BTreeMap;

/// Library methods deliberately without a command, and why.
const EXCLUDED: &[(&str, &str)] = &[(
    "Polarion.logout",
    "the front ends end the session themselves (after each CLI run, when the MCP server stops)",
)];

/// Handle type → accessor on `Polarion` (`""`: methods of `Polarion` itself).
const ACCESSORS: &[(&str, &str)] = &[
    ("Polarion", ""),
    ("WorkItems", "items"),
    ("Documents", "documents"),
    ("Schema", "schema"),
    ("TestRuns", "tests"),
    ("Plans", "plans"),
    ("Wiki", "wiki"),
    ("Baselines", "baselines"),
    ("Revisions", "revisions"),
    ("Activities", "activities"),
    ("Folders", "folders"),
    ("Projects", "projects"),
    ("Users", "users"),
    ("Security", "security"),
    ("Builds", "builds"),
];

fn sources(dir: &str) -> Vec<(String, String)> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .map(|p| {
            (
                p.display().to_string(),
                std::fs::read_to_string(&p).unwrap(),
            )
        })
        .collect()
}

/// `Type.method` for every `pub async fn` inside an `impl Type` / `impl<'a> Type<'a>` block.
fn library_methods() -> Vec<String> {
    let accessors: BTreeMap<_, _> = ACCESSORS.iter().copied().collect();
    let mut methods = Vec::new();
    for (_, source) in sources(concat!(env!("CARGO_MANIFEST_DIR"), "/../polarion/src")) {
        let mut current: Option<String> = None;
        for line in source.lines() {
            if let Some(rest) = line.strip_prefix("impl") {
                let name = rest
                    .trim_start_matches("<'a>")
                    .trim()
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                current = accessors.contains_key(name.as_str()).then_some(name);
            } else if let (Some(ty), Some(rest)) =
                (&current, line.strip_prefix("    pub async fn "))
            {
                let method = rest.split(['(', '<']).next().unwrap();
                methods.push(format!("{ty}.{method}"));
            }
        }
    }
    methods
}

#[test]
fn every_library_method_has_a_command() {
    let methods = library_methods();
    assert!(methods.len() > 250, "parsed only {} methods", methods.len());

    let commands: String = sources(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
        .into_iter()
        .map(|(_, s)| s)
        .collect::<String>()
        .split_whitespace()
        .collect();
    let accessors: BTreeMap<_, _> = ACCESSORS.iter().copied().collect();
    let called = |m: &str| {
        let (ty, method) = m.split_once('.').unwrap();
        let pattern = match accessors[ty] {
            "" => format!("p.{method}("),
            accessor => format!(".{accessor}().{method}("),
        };
        commands.contains(&pattern)
    };
    let excluded = |m: &str| EXCLUDED.iter().any(|(e, _)| *e == m);

    let missing: Vec<&String> = methods
        .iter()
        .filter(|m| !called(m) && !excluded(m))
        .collect();
    assert!(
        missing.is_empty(),
        "library methods without a command (add one in polarion-commands, or exclude it in tests/coverage.rs): {missing:?}"
    );
    for (m, _) in EXCLUDED {
        assert!(
            methods.iter().any(|x| x == m),
            "excluded `{m}` is not a library method"
        );
        assert!(
            !called(m),
            "excluded `{m}` is used; remove it from EXCLUDED"
        );
    }
}

#[test]
fn catalog_is_consistent() {
    let commands = polarion_commands::commands();
    let mut names = std::collections::BTreeSet::new();
    for c in commands {
        assert!(
            names.insert(c.full_name()),
            "duplicate command {}",
            c.full_name()
        );
        assert!(
            polarion_commands::AREAS.iter().any(|(a, _)| *a == c.area),
            "{}: unknown area",
            c.full_name()
        );
        let mut params = std::collections::BTreeSet::new();
        let mut optional_positional = false;
        for p in c.params {
            assert!(
                params.insert(p.name),
                "{}: duplicate parameter {}",
                c.full_name(),
                p.name
            );
            if p.positional {
                assert!(
                    !(optional_positional && p.required),
                    "{}: required positional `{}` after an optional one",
                    c.full_name(),
                    p.name
                );
                optional_positional |= !p.required;
            }
        }
        assert!(c.input_schema()["properties"].is_object());
    }
    assert!(commands.len() > 230, "{} commands", commands.len());
}
