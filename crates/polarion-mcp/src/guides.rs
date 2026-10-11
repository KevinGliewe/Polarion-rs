//! Guides for agents, one per area (`guides/*.md`, embedded): `polarion_guide` returns one with a
//! section read from the project, and the static text is also an MCP resource
//! (`polarion://guide/<area>`). The optional gate (`require_guides`) refuses an area's commands
//! until its guide was read.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU8, Ordering};

use polarion::{EnumOption, Polarion};
use polarion_commands::CommandError;

/// A guide, by area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guide {
    Overview,
    Queries,
    WorkItems,
    Documents,
    Tests,
    Plans,
    Wiki,
}

/// The URI prefix of the guide resources.
pub const URI_PREFIX: &str = "polarion://guide/";

/// Lists in the project section are cut after this many entries.
const MAX_OPTIONS: usize = 40;
/// The custom field keys are read for at most this many work item types.
const MAX_TYPES: usize = 12;
/// Custom field keys shown per type.
const MAX_KEYS: usize = 30;

/// The enumeration of the link roles between work items.
const LINK_ROLE_ENUM: &str = "workitem-link-role";

impl Guide {
    pub const ALL: [Self; 7] = [
        Self::Overview,
        Self::Queries,
        Self::WorkItems,
        Self::Documents,
        Self::Tests,
        Self::Plans,
        Self::Wiki,
    ];

    /// The area name: `overview`, `queries`, `work-items`, ...
    pub fn name(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Queries => "queries",
            Self::WorkItems => "work-items",
            Self::Documents => "documents",
            Self::Tests => "tests",
            Self::Plans => "plans",
            Self::Wiki => "wiki",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.name() == name)
    }

    /// One line on what the guide covers.
    pub fn about(self) -> &'static str {
        match self {
            Self::Overview => {
                "How the server is organized: tools, references, results, write scope, errors"
            }
            Self::Queries => "Polarion's Lucene query syntax, sorting, limits and fields",
            Self::WorkItems => {
                "Work items: fields, custom fields, links, comments, attachments, workflow, Markdown descriptions"
            }
            Self::Documents => "Documents: spaces, URIs, work items, bodies, comments, PDF export",
            Self::Tests => "Test management: test runs, records, steps, templates, attachments",
            Self::Plans => "Plans: templates, plan items, statistics",
            Self::Wiki => "Wiki pages: spaces, pages, searches",
        }
    }

    /// The guide's Markdown.
    pub fn text(self) -> &'static str {
        match self {
            Self::Overview => include_str!("../guides/overview.md"),
            Self::Queries => include_str!("../guides/queries.md"),
            Self::WorkItems => include_str!("../guides/work-items.md"),
            Self::Documents => include_str!("../guides/documents.md"),
            Self::Tests => include_str!("../guides/tests.md"),
            Self::Plans => include_str!("../guides/plans.md"),
            Self::Wiki => include_str!("../guides/wiki.md"),
        }
    }

    /// The resource URI: `polarion://guide/work-items`.
    pub fn uri(self) -> String {
        format!("{URI_PREFIX}{}", self.name())
    }

    /// The guide of a command area: `items` → work items, `documents`, `tests`, `plans` and
    /// `wiki` by name, every other area → the overview. `queries` is no command area.
    pub fn of_area(area: &str) -> Self {
        match area {
            "items" => Self::WorkItems,
            "documents" => Self::Documents,
            "tests" => Self::Tests,
            "plans" => Self::Plans,
            "wiki" => Self::Wiki,
            _ => Self::Overview,
        }
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }

    /// What the project section lists: link roles, custom field keys by type (types and statuses
    /// always).
    fn wants(self) -> (bool, bool) {
        match self {
            Self::WorkItems => (true, true),
            Self::Queries => (true, false),
            _ => (false, false),
        }
    }
}

/// The guides read so far (by this server process).
#[derive(Debug, Default)]
pub struct Read(AtomicU8);

impl Read {
    pub fn mark(&self, guide: Guide) {
        self.0.fetch_or(guide.bit(), Ordering::SeqCst);
    }

    pub fn has(&self, guide: Guide) -> bool {
        self.0.load(Ordering::SeqCst) & guide.bit() != 0
    }
}

/// The section `polarion_guide` appends: the project's work item types and statuses, for some
/// areas its link roles and the custom field keys by type, read now. A read that fails gives a
/// note instead.
pub async fn project_section(
    polarion: Result<&Polarion, &CommandError>,
    project: Option<&str>,
    guide: Guide,
) -> String {
    let Some(project) = project else {
        return "\n\n## Project\n\nNo default project: call `polarion_guide` with `project` to list a \
                project's work item types, statuses and link roles here.\n"
            .to_owned();
    };
    let mut text = format!("\n\n## Project `{project}` (read from the server)\n\n");
    let polarion = match polarion {
        Ok(polarion) => polarion,
        Err(e) => {
            let _ = writeln!(
                text,
                "The project's configuration could not be read: {}",
                short(&e.message)
            );
            return text;
        }
    };
    let schema = polarion.schema();
    let types = match schema.field_options(project, "type").await {
        Ok(types) => visible(types),
        Err(e) => {
            let _ = writeln!(
                text,
                "The project's configuration could not be read: {}",
                short(&e.to_string())
            );
            return text;
        }
    };
    let _ = writeln!(text, "- Work item types: {}", options(&types));
    let statuses = schema
        .field_options(project, "status")
        .await
        .map(visible)
        .map(|s| options(&s));
    let _ = writeln!(text, "- Statuses (`status`): {}", or_note(statuses));
    let (roles, custom) = guide.wants();
    if roles {
        let roles = schema
            .enum_options(project, LINK_ROLE_ENUM)
            .await
            .map(visible)
            .map(|r| options(&r));
        let _ = writeln!(text, "- Link roles: {}", or_note(roles));
    }
    if custom {
        let _ = writeln!(text, "- Custom field keys by type:");
        for kind in types.iter().take(MAX_TYPES) {
            let keys = schema
                .custom_field_keys(project, &kind.id)
                .await
                .map(|keys| list(keys.iter().map(|k| format!("`{k}`")), MAX_KEYS));
            let _ = writeln!(text, "  - `{}`: {}", kind.id, or_note(keys));
        }
        if types.len() > MAX_TYPES {
            let _ = writeln!(
                text,
                "  - {} more types: `custom_fields` with `type` lists their fields",
                types.len() - MAX_TYPES
            );
        }
    }
    text
}

/// Options that are not hidden.
fn visible(options: Vec<EnumOption>) -> Vec<EnumOption> {
    options.into_iter().filter(|o| !o.hidden).collect()
}

/// `` `id` (Name), `id2` ``, cut after [`MAX_OPTIONS`]; `none` for none.
fn options(options: &[EnumOption]) -> String {
    list(
        options.iter().map(|o| match o.name.as_deref() {
            Some(name) if name != o.id && !name.is_empty() => format!("`{}` ({name})", o.id),
            _ => format!("`{}`", o.id),
        }),
        MAX_OPTIONS,
    )
}

fn list(items: impl ExactSizeIterator<Item = String>, max: usize) -> String {
    let count = items.len();
    if count == 0 {
        return "none".to_owned();
    }
    let mut shown: Vec<String> = items.take(max).collect();
    if count > max {
        shown.push(format!("... ({} more)", count - max));
    }
    shown.join(", ")
}

fn or_note(read: polarion::Result<String>) -> String {
    read.unwrap_or_else(|e| format!("could not be read ({})", short(&e.to_string())))
}

/// The first line of an error message, at most 200 characters.
fn short(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();
    match line.char_indices().nth(200) {
        Some((i, _)) => format!("{}...", &line[..i]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_guide_has_its_text_and_name() {
        for guide in Guide::ALL {
            assert_eq!(Guide::parse(guide.name()), Some(guide));
            let lines = guide.text().lines().count();
            assert!(guide.text().starts_with("# "), "{}", guide.name());
            assert!(
                (25..=160).contains(&lines),
                "{}: {lines} lines",
                guide.name()
            );
            assert_eq!(guide.uri(), format!("polarion://guide/{}", guide.name()));
        }
        assert_eq!(Guide::parse("items"), None);
    }

    #[test]
    fn areas_map_to_guides() {
        for (area, guide) in [
            ("items", Guide::WorkItems),
            ("documents", Guide::Documents),
            ("tests", Guide::Tests),
            ("plans", Guide::Plans),
            ("wiki", Guide::Wiki),
            ("schema", Guide::Overview),
            ("session", Guide::Overview),
            ("users", Guide::Overview),
        ] {
            assert_eq!(Guide::of_area(area), guide, "{area}");
        }
        // Every command area has a guide, and none is `queries`.
        for (area, _) in polarion_commands::AREAS {
            assert_ne!(Guide::of_area(area), Guide::Queries, "{area}");
        }
    }

    #[test]
    fn guides_read_are_remembered() {
        let read = Read::default();
        assert!(Guide::ALL.iter().all(|g| !read.has(*g)));
        read.mark(Guide::Tests);
        read.mark(Guide::Wiki);
        for guide in Guide::ALL {
            assert_eq!(
                read.has(guide),
                matches!(guide, Guide::Tests | Guide::Wiki),
                "{guide:?}"
            );
        }
    }

    #[test]
    fn lists_are_cut() {
        let option = |id: &str, name: Option<&str>| EnumOption {
            id: id.to_owned(),
            name: name.map(str::to_owned),
            ..EnumOption::default()
        };
        assert_eq!(
            options(&[option("open", Some("Open")), option("done", Some("done"))]),
            "`open` (Open), `done`"
        );
        assert_eq!(options(&[]), "none");
        let many: Vec<EnumOption> = (0..45).map(|i| option(&format!("s{i}"), None)).collect();
        let text = options(&many);
        assert!(text.ends_with("`s39`, ... (5 more)"), "{text}");
        assert_eq!(short(&"x".repeat(250)).len(), 203);
        assert_eq!(short("first\nsecond"), "first");
    }
}
