//! Which tools the server offers: the optional config file (`--config`, `POLARION_MCP_CONFIG`)
//! and `POLARION_MCP_TOOLS`, resolved against the command catalog.
//!
//! Selectors name commands (`items.delete`, `items.*`, `*.delete`) or tools (`get_work_item`,
//! `items_add_link`, `polarion_call`); `*` is a wildcard, `default` is the curated set and `all`
//! every command.

use std::collections::BTreeSet;
use std::path::Path;

use polarion_commands::{Entry, catalog, find, split_list};
use serde::Deserialize;

/// Everyday reads as dedicated tools: (tool name, command).
pub const CURATED: &[(&str, &str)] = &[
    ("get_work_item", "items.get"),
    ("query_work_items", "items.query"),
    ("count_work_items", "items.count"),
    ("work_item_links", "items.links"),
    ("work_item_history", "items.history"),
    ("list_documents", "documents.list"),
    ("get_document", "documents.get"),
    ("document_work_items", "documents.work-items"),
    ("custom_fields", "schema.custom-fields"),
    ("field_options", "schema.field-options"),
    ("search_test_runs", "tests.search"),
    ("get_test_run", "tests.get"),
    ("search_plans", "plans.search"),
    ("get_project", "projects.get"),
    ("get_user", "users.get"),
];

/// The tools that reach every command.
pub const GENERIC: &[&str] = &["polarion_commands", "polarion_describe", "polarion_call"];

/// The config file.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    /// Commands with a tool of their own.
    tools: Option<Selectors>,
    /// Commands and tools switched off.
    #[serde(default)]
    disable: Vec<String>,
    /// `outputSchema` on tools and `structuredContent` on results.
    output_schemas: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged, expecting = "a selector or a list of selectors")]
enum Selectors {
    One(String),
    Many(Vec<String>),
}

impl Selectors {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(s) => vec![s],
            Self::Many(v) => v,
        }
    }
}

/// The tools to offer and the commands that are switched off.
#[derive(Debug, Clone)]
pub struct Selection {
    /// Commands with a tool of their own: the curated ones first, then in catalog order.
    pub tools: Vec<&'static Entry>,
    /// Generic tools that are listed.
    pub generic: Vec<&'static str>,
    /// Full names of the commands that are switched off.
    disabled: BTreeSet<String>,
    /// `outputSchema` on tools and `structuredContent` on results.
    pub output_schemas: bool,
}

impl Default for Selection {
    fn default() -> Self {
        Self::resolve(&["default".to_owned()], &[], true).expect("the default selection resolves")
    }
}

impl Selection {
    /// From the config file at `path` and `tools_env` (`POLARION_MCP_TOOLS`, which replaces the
    /// file's `tools`).
    pub fn load(path: Option<&Path>, tools_env: Option<&str>) -> Result<Self, String> {
        let file = match path {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("config file {}: {e}", path.display()))?;
                toml::from_str::<File>(&text)
                    .map_err(|e| format!("config file {}: {e}", path.display()))?
            }
            None => File::default(),
        };
        let in_file = |key: &str, e: String| match path {
            Some(path) => format!("config file {}: `{key}`: {e}", path.display()),
            None => e,
        };
        let tools = match tools_env {
            Some(value) => split_list(value),
            None => file
                .tools
                .map_or_else(|| vec!["default".to_owned()], Selectors::into_vec),
        };
        let (tools, _) = select_all(&tools, false).map_err(|e| match tools_env {
            Some(_) => format!("POLARION_MCP_TOOLS: {e}"),
            None => in_file("tools", e),
        })?;
        let (disabled, generic) =
            select_all(&file.disable, true).map_err(|e| in_file("disable", e))?;
        Ok(Self::new(
            tools,
            disabled,
            &generic,
            file.output_schemas.unwrap_or(true),
        ))
    }

    fn resolve(tools: &[String], disable: &[String], output_schemas: bool) -> Result<Self, String> {
        let (tools, _) = select_all(tools, false)?;
        let (disabled, generic) = select_all(disable, true)?;
        Ok(Self::new(tools, disabled, &generic, output_schemas))
    }

    fn new(
        tools: Vec<&'static Entry>,
        disabled: Vec<&'static Entry>,
        disabled_generic: &[&'static str],
        output_schemas: bool,
    ) -> Self {
        let disabled: BTreeSet<String> = disabled.iter().map(|c| c.full_name()).collect();
        let selected: BTreeSet<String> = tools
            .iter()
            .map(|c| c.full_name())
            .filter(|n| !disabled.contains(n))
            .collect();
        let curated = CURATED
            .iter()
            .map(|(_, command)| (*command).to_owned())
            .filter(|n| selected.contains(n));
        let others = catalog()
            .iter()
            .map(Entry::full_name)
            .filter(|n| selected.contains(n) && curated_name(n).is_none());
        Self {
            tools: curated
                .chain(others)
                .map(|n| find(&n).expect("selected commands exist"))
                .collect(),
            generic: GENERIC
                .iter()
                .copied()
                .filter(|g| !disabled_generic.contains(g))
                .collect(),
            disabled,
            output_schemas,
        }
    }

    /// Whether `command` is switched off.
    pub fn is_disabled(&self, command: &Entry) -> bool {
        self.disabled.contains(&command.full_name())
    }

    /// Whether any command is switched off.
    pub fn disables_commands(&self) -> bool {
        !self.disabled.is_empty()
    }

    /// Whether the generic tool `name` is listed.
    pub fn lists(&self, name: &str) -> bool {
        self.generic.contains(&name)
    }
}

/// A command's tool name: the curated one, or `items.back-links` → `items_back_links`.
pub fn tool_name(command: &Entry) -> String {
    curated_name(&command.full_name()).map_or_else(|| generic_name(command), str::to_owned)
}

/// Whether `command` is one of the everyday reads (`CURATED`).
pub fn is_curated(command: &Entry) -> bool {
    curated_name(&command.full_name()).is_some()
}

fn generic_name(command: &Entry) -> String {
    format!("{}_{}", command.area, command.name.replace('-', "_"))
}

fn curated_name(full_name: &str) -> Option<&'static str> {
    CURATED
        .iter()
        .find(|(_, command)| *command == full_name)
        .map(|(name, _)| *name)
}

/// The commands, and with `generic` the generic tools, that `selectors` name. A selector that
/// names nothing is an error.
fn select_all(
    selectors: &[String],
    generic: bool,
) -> Result<(Vec<&'static Entry>, Vec<&'static str>), String> {
    let mut commands = Vec::new();
    let mut tools = Vec::new();
    for selector in selectors {
        let (c, t) = select(selector);
        let t: Vec<&'static str> = if generic { t } else { Vec::new() };
        if c.is_empty() && t.is_empty() {
            return Err(
                if !generic && GENERIC.iter().any(|g| matches(selector, g)) {
                    format!("`{selector}` names no command (the generic tools are always listed)")
                } else {
                    format!("`{selector}` matches no command or tool")
                },
            );
        }
        commands.extend(c);
        tools.extend(t);
    }
    Ok((commands, tools))
}

/// The commands and generic tools one selector names.
fn select(selector: &str) -> (Vec<&'static Entry>, Vec<&'static str>) {
    let selector = selector.trim();
    match selector {
        "" => (Vec::new(), Vec::new()),
        "default" => (
            CURATED
                .iter()
                .map(|(_, c)| find(c).expect("curated commands exist"))
                .collect(),
            Vec::new(),
        ),
        "all" => (catalog().iter().collect(), Vec::new()),
        s if s.contains('.') => (
            catalog()
                .iter()
                .filter(|c| matches(s, &c.full_name()))
                .collect(),
            Vec::new(),
        ),
        s => (
            catalog()
                .iter()
                .filter(|c| {
                    matches(s, &generic_name(c))
                        || curated_name(&c.full_name()).is_some_and(|n| matches(s, n))
                })
                .collect(),
            GENERIC.iter().copied().filter(|g| matches(s, g)).collect(),
        ),
    }
}

/// `name` matches `pattern`, in which `*` stands for any run of characters.
fn matches(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(mut rest) = name.strip_prefix(parts.next().unwrap_or_default()) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(commands: &[&Entry]) -> Vec<String> {
        commands.iter().map(|c| c.full_name()).collect()
    }

    fn selection(toml: &str) -> Result<Selection, String> {
        let path = std::env::temp_dir().join(format!(
            "polarion-mcp-config-{}-{:x}.toml",
            std::process::id(),
            toml.bytes()
                .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b.into()))
        ));
        std::fs::write(&path, toml).unwrap();
        let result = Selection::load(Some(&path), None);
        std::fs::remove_file(&path).unwrap();
        result
    }

    #[test]
    fn wildcards() {
        assert!(matches("items.get", "items.get"));
        assert!(!matches("items.get", "items.get-many"));
        assert!(matches("items.*", "items.get"));
        assert!(matches("*.delete", "items.delete"));
        assert!(!matches("*.delete", "items.delete-link"));
        assert!(matches("*", ""));
        assert!(matches("a*b*c", "abc"));
        assert!(matches("a*b*c", "a-b-b-c"));
        assert!(!matches("a*b*c", "a-c-b"));
        assert!(!matches("ab*ba", "aba"));
    }

    #[test]
    fn the_default_is_the_curated_set() {
        let selection = Selection::default();
        assert_eq!(selection.tools.len(), CURATED.len());
        assert_eq!(tool_name(selection.tools[0]), "get_work_item");
        assert_eq!(selection.generic, GENERIC);
        assert!(selection.output_schemas);
        assert!(!selection.disables_commands());
        assert_eq!(
            names(&Selection::load(None, None).unwrap().tools),
            names(&selection.tools)
        );
    }

    #[test]
    fn all_is_every_command_once() {
        let selection = Selection::load(None, Some("all")).unwrap();
        assert_eq!(selection.tools.len(), catalog().len());
        let tools: BTreeSet<String> = selection.tools.iter().map(|c| tool_name(c)).collect();
        assert_eq!(tools.len(), catalog().len());
        assert!(tools.contains("get_work_item") && !tools.contains("items_get"));
        assert_eq!(
            names(&selection.tools[..CURATED.len()]),
            CURATED.iter().map(|(_, c)| *c).collect::<Vec<_>>()
        );
    }

    #[test]
    fn selectors_name_commands_and_tools() {
        let tools = |s: &str| names(&Selection::load(None, Some(s)).unwrap().tools);
        assert!(tools("items.*").iter().all(|n| n.starts_with("items.")));
        assert!(tools("items.*").len() > 20);
        assert_eq!(tools("get_work_item"), ["items.get"]);
        assert_eq!(tools("items_get"), ["items.get"]);
        assert_eq!(tools("items.get, items.get"), ["items.get"]);
        assert!(tools("*.delete").contains(&"items.delete".to_owned()));
        assert_eq!(tools(""), Vec::<String>::new());
    }

    #[test]
    fn disabled_commands_and_tools_are_off() {
        let selection = selection(
            r#"
            # comments are fine
            tools = "all"
            disable = ["items.*", "get_document", "polarion_d*"]
            output_schemas = false
            "#,
        )
        .unwrap();
        assert!(!selection.output_schemas);
        assert!(selection.disables_commands());
        assert!(selection.is_disabled(find("items.get").unwrap()));
        assert!(selection.is_disabled(find("documents.get").unwrap()));
        assert!(!selection.is_disabled(find("documents.list").unwrap()));
        assert!(
            selection
                .tools
                .iter()
                .all(|c| c.area != "items" && c.full_name() != "documents.get")
        );
        assert_eq!(selection.generic, ["polarion_commands", "polarion_call"]);
        assert_eq!(tool_name(selection.tools[0]), "list_documents");
    }

    #[test]
    fn the_environment_replaces_the_files_tools() {
        let path = std::env::temp_dir().join(format!(
            "polarion-mcp-config-env-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "tools = \"all\"\ndisable = [\"documents.get\"]").unwrap();
        let selection = Selection::load(Some(&path), Some("documents.*"));
        std::fs::remove_file(&path).unwrap();
        let selection = selection.unwrap();
        assert!(selection.tools.iter().all(|c| c.area == "documents"));
        assert!(selection.is_disabled(find("documents.get").unwrap()));
    }

    #[test]
    fn mistakes_are_errors() {
        let error = |toml: &str| selection(toml).unwrap_err();
        assert!(error("tools = [\"itmes.*\"]").contains("`itmes.*` matches no command or tool"));
        assert!(error("tools = \"polarion_call\"").contains("generic tools are always listed"));
        assert!(error("disable = [\"get_work_itme\"]").contains("`disable`"));
        assert!(error("tool = \"all\"").contains("unknown field"));
        assert!(error("tools = 3").contains("a selector or a list of selectors"));
        assert!(error("tools = [").contains("config file"));
        let missing = Selection::load(Some(Path::new("/nonexistent/polarion-mcp.toml")), None);
        assert!(
            missing
                .unwrap_err()
                .contains("/nonexistent/polarion-mcp.toml")
        );
        let env = Selection::load(None, Some("default,itmes.*")).unwrap_err();
        assert!(env.starts_with("POLARION_MCP_TOOLS: `itmes.*`"), "{env}");
    }
}
