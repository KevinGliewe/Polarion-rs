//! Which tools the server offers: the optional config file (`--config`, `POLARION_MCP_CONFIG`)
//! and `POLARION_MCP_TOOLS`, resolved against the command catalog; and which commands need their
//! area's guide first (`require_guides`, `POLARION_MCP_REQUIRE_GUIDES`).
//!
//! Selectors name commands (`items.delete`, `items.*`, `*.delete`) or tools (`get_work_item`,
//! `items_add_hyperlink`, `polarion_call`); `*` is a wildcard, `default` is the curated set and
//! `all` every command. The curated writes (`CURATED_WRITES`) are listed only when writes are
//! enabled.

use std::collections::BTreeSet;
use std::path::Path;

use polarion_commands::{Entry, catalog, find, split_list};
use serde::Deserialize;

/// Everyday reads as dedicated tools, by area: (tool name, command).
pub const CURATED: &[(&str, &str)] = &[
    // Work items.
    ("get_work_item", "items.get"),
    ("query_work_items", "items.query"),
    ("count_work_items", "items.count"),
    ("work_item_links", "items.links"),
    ("work_item_history", "items.history"),
    (
        "get_work_item_description_markdown",
        "items.description-markdown",
    ),
    // Documents.
    ("list_documents", "documents.list"),
    ("get_document", "documents.get"),
    ("document_work_items", "documents.work-items"),
    ("search_documents", "documents.query"),
    ("export_document_pdf", "documents.export-pdf"),
    // Schema.
    ("custom_fields", "schema.custom-fields"),
    ("field_options", "schema.field-options"),
    // Test management.
    ("search_test_runs", "tests.search"),
    ("get_test_run", "tests.get"),
    ("test_case_steps", "tests.steps"),
    ("test_run_records", "tests.records"),
    ("test_run_report", "tests.report"),
    // Plans.
    ("search_plans", "plans.search"),
    ("get_plan", "plans.get"),
    ("plan_statistics", "plans.statistics"),
    // Wiki.
    ("query_wiki_pages", "wiki.query"),
    ("get_wiki_page", "wiki.get"),
    // Project and user.
    ("get_project", "projects.get"),
    ("get_user", "users.get"),
];

/// Everyday writes as dedicated tools, by area: (tool name, command). Listed only when writes
/// are enabled; without, these commands have no tool of their own (`polarion_call` reaches them,
/// and the library refuses them).
pub const CURATED_WRITES: &[(&str, &str)] = &[
    // Work items.
    ("create_work_item", "items.create"),
    ("update_work_item", "items.update"),
    (
        "set_work_item_description_markdown",
        "items.set-description-markdown",
    ),
    (
        "append_work_item_description_markdown",
        "items.append-description-markdown",
    ),
    ("add_work_item_comment", "items.add-comment"),
    ("add_work_item_link", "items.add-link"),
    ("remove_work_item_link", "items.remove-link"),
    ("add_work_item_attachment", "items.add-attachment"),
    // Documents.
    ("create_document", "documents.create"),
    ("update_document", "documents.update"),
    ("add_document_comment", "documents.add-comment"),
    // Test runs.
    ("create_test_run", "tests.create"),
    ("update_test_run", "tests.update"),
    ("add_test_record", "tests.add-record"),
    ("set_test_steps", "tests.set-steps"),
    // Plans.
    ("create_plan", "plans.create"),
    ("update_plan", "plans.update"),
    ("add_plan_items", "plans.add-items"),
    ("remove_plan_items", "plans.remove-items"),
];

/// The tools that reach every command, and the guides.
pub const GENERIC: &[&str] = &[
    "polarion_guide",
    "polarion_commands",
    "polarion_describe",
    "polarion_call",
];

/// Which commands are refused until the guide of their area was read (`polarion_guide`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequireGuides {
    /// The commands that write (when writes are enabled at all).
    #[default]
    Write,
    /// Every command.
    All,
    /// None.
    None,
}

impl RequireGuides {
    /// `write`, `all` or `none` (`POLARION_MCP_REQUIRE_GUIDES`).
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "write" => Ok(Self::Write),
            "all" => Ok(Self::All),
            "none" => Ok(Self::None),
            other => Err(format!("expected `write`, `all` or `none`, not `{other}`")),
        }
    }
}

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
    /// Which commands need their area's guide first.
    require_guides: Option<RequireGuides>,
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
    /// Commands with a tool of their own: the curated reads, the curated writes (only when writes
    /// are enabled), then the others in catalog order.
    pub tools: Vec<&'static Entry>,
    /// Generic tools that are listed.
    pub generic: Vec<&'static str>,
    /// Full names of the commands that are switched off.
    disabled: BTreeSet<String>,
    /// `outputSchema` on tools and `structuredContent` on results.
    pub output_schemas: bool,
    /// Which commands need their area's guide first; `None` when `polarion_guide` is disabled
    /// (and nothing else was asked for).
    pub require_guides: RequireGuides,
}

impl Default for Selection {
    /// The default tools of a read-only server.
    fn default() -> Self {
        Self::resolve(&["default".to_owned()], &[], true, false)
            .expect("the default selection resolves")
    }
}

impl Selection {
    /// From the config file at `path`, `tools_env` (`POLARION_MCP_TOOLS`, which replaces the
    /// file's `tools`) and `guides_env` (`POLARION_MCP_REQUIRE_GUIDES`, which replaces the file's
    /// `require_guides`). `writes`: whether writes are enabled (`Config::writes_enabled`); without,
    /// the curated writes are not listed.
    pub fn load(
        path: Option<&Path>,
        tools_env: Option<&str>,
        guides_env: Option<&str>,
        writes: bool,
    ) -> Result<Self, String> {
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
        let require_guides = match guides_env {
            Some(value) => Some(
                RequireGuides::parse(value)
                    .map_err(|e| format!("POLARION_MCP_REQUIRE_GUIDES: {e}"))?,
            ),
            None => file.require_guides,
        };
        let mut selection = Self::new(
            tools,
            disabled,
            &generic,
            file.output_schemas.unwrap_or(true),
            writes,
        );
        // Without `polarion_guide` no guide can be read: the default gate is off, and asking for
        // one is a mistake.
        if !selection.lists("polarion_guide") {
            if require_guides.is_some_and(|r| r != RequireGuides::None) {
                let source = if guides_env.is_some() {
                    "POLARION_MCP_REQUIRE_GUIDES"
                } else {
                    "`require_guides`"
                };
                return Err(in_file(
                    "disable",
                    format!("{source} needs `polarion_guide`, which is disabled"),
                ));
            }
            selection.require_guides = RequireGuides::None;
        } else if let Some(required) = require_guides {
            selection.require_guides = required;
        }
        Ok(selection)
    }

    fn resolve(
        tools: &[String],
        disable: &[String],
        output_schemas: bool,
        writes: bool,
    ) -> Result<Self, String> {
        let (tools, _) = select_all(tools, false)?;
        let (disabled, generic) = select_all(disable, true)?;
        Ok(Self::new(tools, disabled, &generic, output_schemas, writes))
    }

    /// The selected commands that are not disabled: the curated reads, the curated writes when
    /// `writes`, then the others in catalog order. A command with a curated tool never gets a
    /// generic one, so without `writes` the curated writes are not listed at all.
    fn new(
        tools: Vec<&'static Entry>,
        disabled: Vec<&'static Entry>,
        disabled_generic: &[&'static str],
        output_schemas: bool,
        writes: bool,
    ) -> Self {
        let disabled: BTreeSet<String> = disabled.iter().map(|c| c.full_name()).collect();
        let selected: BTreeSet<String> = tools
            .iter()
            .map(|c| c.full_name())
            .filter(|n| !disabled.contains(n))
            .collect();
        let curated_writes = if writes { CURATED_WRITES } else { &[] };
        let curated = CURATED
            .iter()
            .chain(curated_writes)
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
            require_guides: RequireGuides::default(),
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

/// Whether `command` has a curated tool: one of the everyday reads (`CURATED`) or writes
/// (`CURATED_WRITES`).
pub fn is_curated(command: &Entry) -> bool {
    curated_name(&command.full_name()).is_some()
}

fn generic_name(command: &Entry) -> String {
    format!("{}_{}", command.area, command.name.replace('-', "_"))
}

fn curated_name(full_name: &str) -> Option<&'static str> {
    CURATED
        .iter()
        .chain(CURATED_WRITES)
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
                .chain(CURATED_WRITES)
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
        selection_with(toml, false)
    }

    /// The selection the config file `toml` makes, with `writes` enabled or not.
    fn selection_with(toml: &str, writes: bool) -> Result<Selection, String> {
        let path = std::env::temp_dir().join(format!(
            "polarion-mcp-config-{}-{writes}-{:x}.toml",
            std::process::id(),
            toml.bytes()
                .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b.into()))
        ));
        std::fs::write(&path, toml).unwrap();
        let result = Selection::load(Some(&path), None, None, writes);
        std::fs::remove_file(&path).unwrap();
        result
    }

    fn curated(table: &[(&str, &'static str)]) -> Vec<&'static str> {
        table.iter().map(|(_, c)| *c).collect()
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
        assert_eq!(selection.require_guides, RequireGuides::Write);
        assert!(!selection.disables_commands());
        assert_eq!(
            names(&Selection::load(None, None, None, false).unwrap().tools),
            names(&selection.tools)
        );
    }

    #[test]
    fn the_curated_writes_follow_the_reads_when_writes_are_enabled() {
        let selection = Selection::load(None, None, None, true).unwrap();
        assert_eq!(
            names(&selection.tools),
            [curated(CURATED), curated(CURATED_WRITES)].concat()
        );
        assert_eq!(
            tool_name(selection.tools[CURATED.len()]),
            "create_work_item"
        );
        assert!(
            selection.tools[CURATED.len()..]
                .iter()
                .all(|c| c.write && is_curated(c))
        );
        // The table names existing commands, none twice: the reads read, the writes write.
        let commands: BTreeSet<&str> = CURATED.iter().chain(CURATED_WRITES).map(|t| t.1).collect();
        assert_eq!(commands.len(), CURATED.len() + CURATED_WRITES.len());
        assert!(commands.iter().all(|c| find(c).is_some()));
        assert_eq!((CURATED.len(), CURATED_WRITES.len()), (25, 19));
        assert!(CURATED.iter().all(|(_, c)| !find(c).unwrap().write));
        assert!(CURATED_WRITES.iter().all(|(_, c)| find(c).unwrap().write));
        // By area: each area's tools together, in the order of the areas.
        let areas = |table: &[(&str, &str)]| {
            let mut areas: Vec<&str> = table.iter().map(|(_, c)| find(c).unwrap().area).collect();
            areas.dedup();
            areas
        };
        assert_eq!(
            areas(CURATED),
            [
                "items",
                "documents",
                "schema",
                "tests",
                "plans",
                "wiki",
                "projects",
                "users"
            ]
        );
        assert_eq!(
            areas(CURATED_WRITES),
            ["items", "documents", "tests", "plans"]
        );
        assert_eq!(
            tool_name(selection.tools[CURATED.len() + 8]),
            "create_document"
        );
    }

    #[test]
    fn all_is_every_command_once() {
        for writes in [false, true] {
            let selection = Selection::load(None, Some("all"), None, writes).unwrap();
            // Without writes, the curated writes are not listed, under no name.
            let expected = catalog().len() - if writes { 0 } else { CURATED_WRITES.len() };
            assert_eq!(selection.tools.len(), expected);
            let tools: BTreeSet<String> = selection.tools.iter().map(|c| tool_name(c)).collect();
            assert_eq!(tools.len(), expected);
            assert!(tools.contains("get_work_item") && !tools.contains("items_get"));
            assert!(!tools.contains("items_create") && !tools.contains("items_add_link"));
            assert_eq!(tools.contains("create_work_item"), writes);
            assert!(tools.contains("items_delete"));
            let curated_writes = if writes {
                curated(CURATED_WRITES)
            } else {
                vec![]
            };
            let first = CURATED.len() + curated_writes.len();
            assert_eq!(
                names(&selection.tools[..first]),
                [curated(CURATED), curated_writes].concat()
            );
        }
    }

    #[test]
    fn selectors_name_commands_and_tools() {
        let tools = |s: &str| names(&Selection::load(None, Some(s), None, false).unwrap().tools);
        let with_writes =
            |s: &str| names(&Selection::load(None, Some(s), None, true).unwrap().tools);
        assert!(tools("items.*").iter().all(|n| n.starts_with("items.")));
        assert!(tools("items.*").len() > 20);
        assert_eq!(tools("get_work_item"), ["items.get"]);
        assert_eq!(tools("items_get"), ["items.get"]);
        assert_eq!(tools("items.get, items.get"), ["items.get"]);
        assert!(tools("*.delete").contains(&"items.delete".to_owned()));
        assert_eq!(tools(""), Vec::<String>::new());
        // The curated writes by tool name, command name or generic name, only with writes.
        assert_eq!(
            with_writes("items.update, create_work_item, items_add_link, get_work_item"),
            [
                "items.get",
                "items.create",
                "items.update",
                "items.add-link"
            ]
        );
        assert_eq!(
            tools("items.update, create_work_item, get_work_item"),
            ["items.get"]
        );
        assert!(!tools("items.*").contains(&"items.create".to_owned()));
        assert!(with_writes("items.*").contains(&"items.create".to_owned()));
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
        assert_eq!(
            selection.generic,
            ["polarion_guide", "polarion_commands", "polarion_call"]
        );
        assert_eq!(tool_name(selection.tools[0]), "list_documents");
    }

    #[test]
    fn disabled_curated_writes_are_not_listed() {
        let selection = selection_with(
            "disable = [\"add_work_item_comment\", \"items.remove-link\"]",
            true,
        )
        .unwrap();
        let mut expected = [curated(CURATED), curated(CURATED_WRITES)].concat();
        expected.retain(|c| !matches!(*c, "items.add-comment" | "items.remove-link"));
        assert_eq!(names(&selection.tools), expected);
        assert!(selection.is_disabled(find("items.add-comment").unwrap()));
    }

    #[test]
    fn a_disabled_new_curated_tool_is_not_listed() {
        for writes in [false, true] {
            let selection = selection_with(
                "tools = \"all\"
disable = [\"test_run_report\", \"tests.update\"]",
                writes,
            )
            .unwrap();
            let tools: Vec<String> = selection.tools.iter().map(|c| tool_name(c)).collect();
            for gone in [
                "test_run_report",
                "tests_report",
                "update_test_run",
                "tests_update",
            ] {
                assert!(!tools.iter().any(|t| t == gone), "{gone} listed");
            }
            assert!(selection.is_disabled(find("tests.report").unwrap()));
            assert!(selection.is_disabled(find("tests.update").unwrap()));
            assert_eq!(tools.iter().any(|t| t == "add_test_record"), writes);
            assert!(tools.iter().any(|t| t == "test_run_records"));
        }
    }

    #[test]
    fn the_environment_replaces_the_files_tools() {
        let path = std::env::temp_dir().join(format!(
            "polarion-mcp-config-env-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "tools = \"all\"\ndisable = [\"documents.get\"]").unwrap();
        let selection = Selection::load(Some(&path), Some("documents.*"), None, false);
        std::fs::remove_file(&path).unwrap();
        let selection = selection.unwrap();
        assert!(selection.tools.iter().all(|c| c.area == "documents"));
        assert!(selection.is_disabled(find("documents.get").unwrap()));
    }

    #[test]
    fn guides_are_required_as_configured() {
        let required = |toml: &str| selection(toml).map(|s| s.require_guides);
        assert_eq!(required(""), Ok(RequireGuides::Write));
        assert_eq!(required("require_guides = \"all\""), Ok(RequireGuides::All));
        assert_eq!(
            required("require_guides = \"none\""),
            Ok(RequireGuides::None)
        );
        assert_eq!(
            required("require_guides = \"write\""),
            Ok(RequireGuides::Write)
        );
        let error = required("require_guides = \"reads\"").unwrap_err();
        assert!(error.contains("unknown variant `reads`"), "{error}");
        // Without polarion_guide: off by default, an error when asked for.
        assert_eq!(
            required("disable = [\"polarion_guide\"]"),
            Ok(RequireGuides::None)
        );
        assert_eq!(
            required(
                "disable = [\"polarion_g*\"]
require_guides = \"none\""
            ),
            Ok(RequireGuides::None)
        );
        let error = required(
            "disable = [\"polarion_guide\"]
require_guides = \"all\"",
        )
        .unwrap_err();
        assert!(
            error.contains("`require_guides` needs `polarion_guide`, which is disabled"),
            "{error}"
        );

        // The environment wins over the file.
        let env = |value: &str, toml: Option<&str>| {
            let path = std::env::temp_dir().join(format!(
                "polarion-mcp-config-guides-{}-{value}.toml",
                std::process::id()
            ));
            if let Some(toml) = toml {
                std::fs::write(&path, toml).unwrap();
            }
            let result = Selection::load(toml.map(|_| path.as_path()), None, Some(value), false);
            if toml.is_some() {
                std::fs::remove_file(&path).unwrap();
            }
            result.map(|s| s.require_guides)
        };
        assert_eq!(env("all", None), Ok(RequireGuides::All));
        assert_eq!(env(" None ", None), Ok(RequireGuides::None));
        assert_eq!(
            env("write", Some("require_guides = \"all\"")),
            Ok(RequireGuides::Write)
        );
        let error = env("sometimes", None).unwrap_err();
        assert!(
            error.starts_with("POLARION_MCP_REQUIRE_GUIDES: expected `write`, `all` or `none`"),
            "{error}"
        );
        let error = env("write", Some("disable = [\"polarion_guide\"]")).unwrap_err();
        assert!(
            error.contains("POLARION_MCP_REQUIRE_GUIDES needs `polarion_guide`"),
            "{error}"
        );
        assert_eq!(
            env("none", Some("disable = [\"polarion_guide\"]")),
            Ok(RequireGuides::None)
        );
    }

    #[test]
    fn mistakes_are_errors() {
        let error = |toml: &str| selection(toml).unwrap_err();
        assert!(error("tools = [\"itmes.*\"]").contains("`itmes.*` matches no command or tool"));
        assert!(error("tools = \"polarion_call\"").contains("generic tools are always listed"));
        assert!(error("tools = \"polarion_guide\"").contains("generic tools are always listed"));
        assert!(error("disable = [\"get_work_itme\"]").contains("`disable`"));
        assert!(error("tool = \"all\"").contains("unknown field"));
        assert!(error("tools = 3").contains("a selector or a list of selectors"));
        assert!(error("tools = [").contains("config file"));
        let missing = Selection::load(
            Some(Path::new("/nonexistent/polarion-mcp.toml")),
            None,
            None,
            false,
        );
        assert!(
            missing
                .unwrap_err()
                .contains("/nonexistent/polarion-mcp.toml")
        );
        let env = Selection::load(None, Some("default,itmes.*"), None, false).unwrap_err();
        assert!(env.starts_with("POLARION_MCP_TOOLS: `itmes.*`"), "{env}");
    }
}
