//! Conversion options.

/// Which `PREFIX-123` ids in text are work item links.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WorkItemIds {
    /// Any `PREFIX-123`: an uppercase letter, at least one more uppercase letter, digit or `_`,
    /// a hyphen and digits.
    #[default]
    Any,
    /// Only ids with one of these prefixes, e.g. `MP` for `MP-42`.
    Prefixes(Vec<String>),
    /// No id becomes a link.
    Off,
}

impl WorkItemIds {
    /// Whether an id with this prefix (the part before the hyphen) is a link.
    pub fn links(&self, prefix: &str) -> bool {
        match self {
            WorkItemIds::Any => true,
            WorkItemIds::Prefixes(prefixes) => prefixes.iter().any(|p| p == prefix),
            WorkItemIds::Off => false,
        }
    }
}

/// What the conversion to Markdown does with formatting Markdown cannot express
/// (colors, fonts, alignment, merged table cells, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RawHtml {
    /// Keep it as HTML (inline spans, HTML blocks for tables and styled blocks), so converting
    /// back restores it.
    #[default]
    Keep,
    /// Drop it: the text stays, the formatting is lost (what Markdown cannot hold exactly, such as
    /// some formulas, stays HTML).
    Drop,
}

/// Options of the conversion from Markdown to Polarion HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToHtmlOptions {
    /// Which `PREFIX-123` ids become work item links.
    pub work_items: WorkItemIds,
    /// Highlight fenced code that names its language (feature `highlight`; default: on).
    pub highlight: bool,
}

impl Default for ToHtmlOptions {
    fn default() -> Self {
        Self {
            work_items: WorkItemIds::Any,
            highlight: true,
        }
    }
}

/// Options of the conversion from Polarion HTML to Markdown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToMarkdownOptions {
    /// Formatting Markdown cannot express: kept as HTML (default) or dropped.
    pub raw_html: RawHtml,
    /// The ids the conversion back to HTML will link (use the same value in both directions):
    /// a work item link to such an id is written as the plain id; another one stays its span
    /// (keep mode) or becomes its id as text (drop mode).
    pub work_items: WorkItemIds,
}
