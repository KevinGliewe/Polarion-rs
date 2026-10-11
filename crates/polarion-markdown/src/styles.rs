//! The Polarion style strings and the block separators both directions share.
//!
//! Every string here is part of the HTML the forward direction writes and the reverse direction
//! recognizes; tests compare them exactly.

use comrak::nodes::{AlertType, TableAlignment};

/// Bold (`**x**`).
pub(crate) const BOLD: &str = "font-weight: bold;";
/// Italic (`*x*`).
pub(crate) const ITALIC: &str = "font-style: italic;";
/// Strikethrough (`~~x~~`).
pub(crate) const STRIKE: &str = "text-decoration: line-through;";
/// Underline (`++x++`).
pub(crate) const UNDERLINE: &str = "text-decoration: underline;";
/// Highlight (`==x==`).
pub(crate) const HIGHLIGHT: &str = "background-color: #FFFF00;";
/// Monospace: inline code; also part of [`PRE`].
pub(crate) const MONO: &str = "font-family: 'Courier New', Courier, monospace;";

/// Font sizes of headings 1 to 6 in pt.
pub(crate) const HEADING_SIZES: [&str; 6] = ["22", "18", "14", "12", "11", "10.5"];
/// Line heights of headings 1 to 6.
#[allow(dead_code)] // the tests check the heading styles against it
pub(crate) const HEADING_LINE_HEIGHTS: [&str; 6] = ["1.2", "1.2", "1.2", "1.5", "1.5", "1.5"];

/// The style of a heading span; the level is clamped to 1..=6.
pub(crate) fn heading(level: u8) -> &'static str {
    match level {
        0 | 1 => "font-weight: bold; font-size: 22pt; line-height: 1.2;",
        2 => "font-weight: bold; font-size: 18pt; line-height: 1.2;",
        3 => "font-weight: bold; font-size: 14pt; line-height: 1.2;",
        4 => "font-weight: bold; font-size: 12pt; line-height: 1.5;",
        5 => "font-weight: bold; font-size: 11pt; line-height: 1.5;",
        _ => "font-weight: bold; font-size: 10.5pt; line-height: 1.5;",
    }
}

/// The heading level of text in this font size (pt): ≥22 → 1, ≥18 → 2, ≥14 → 3, ≥12 → 4,
/// ≥11 → 5, >10 → 6; `None` at 10pt (the default size) and below.
pub(crate) fn heading_level(size_pt: f32) -> Option<u8> {
    [22.0, 18.0, 14.0, 12.0, 11.0]
        .iter()
        .position(|&min| size_pt >= min)
        .map(|i| i as u8 + 1)
        .or((size_pt > 10.0).then_some(6))
}

/// `id` of tables (as Polarion's rich text editor writes it).
pub(crate) const TABLE_ID: &str = "polarion_wiki macro name=table";
/// `class` of tables.
pub(crate) const TABLE_CLASS: &str = "polarion-Document-table";
/// `style` of tables.
pub(crate) const TABLE: &str = "width: 100%; max-width: 1280px; margin-left: 0px; margin-right: auto; \
     border: 1px solid #CCCCCC; empty-cells: show; border-collapse: collapse;";

/// Horizontal alignment of a table column (`:--`, `:-:`, `--:`; none is left).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    /// The CSS `text-align` value.
    pub(crate) fn css(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        }
    }

    /// The alignment of a CSS `text-align` value (`left`, `center`, `right`, ignoring case and
    /// surrounding whitespace); `None` for any other value.
    pub(crate) fn from_css(value: &str) -> Option<Align> {
        [Align::Left, Align::Center, Align::Right]
            .into_iter()
            .find(|a| a.css().eq_ignore_ascii_case(value.trim()))
    }
}

impl From<TableAlignment> for Align {
    fn from(a: TableAlignment) -> Self {
        match a {
            TableAlignment::None | TableAlignment::Left => Align::Left,
            TableAlignment::Center => Align::Center,
            TableAlignment::Right => Align::Right,
        }
    }
}

/// The Markdown column alignment of a cell alignment: left is the default (`---`), so `:--`
/// does not survive a round trip.
impl From<Align> for TableAlignment {
    fn from(a: Align) -> Self {
        match a {
            Align::Left => TableAlignment::None,
            Align::Center => TableAlignment::Center,
            Align::Right => TableAlignment::Right,
        }
    }
}

/// `style` of a header cell.
pub(crate) fn th(align: Align) -> &'static str {
    match align {
        Align::Left => concat!(
            "font-weight: bold; background-color: #F0F0F0; text-align: left; ",
            "vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;"
        ),
        Align::Center => concat!(
            "font-weight: bold; background-color: #F0F0F0; text-align: center; ",
            "vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;"
        ),
        Align::Right => concat!(
            "font-weight: bold; background-color: #F0F0F0; text-align: right; ",
            "vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;"
        ),
    }
}

/// `style` of a data cell.
pub(crate) fn td(align: Align) -> &'static str {
    match align {
        Align::Left => {
            "text-align: left; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;"
        }
        Align::Center => {
            "text-align: center; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;"
        }
        Align::Right => {
            "text-align: right; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;"
        }
    }
}

/// `style` of code blocks (`<pre>`): indented and monospace.
pub(crate) const PRE: &str = "margin-left: 40px; font-family: 'Courier New', Courier, monospace;";
/// Prefix of the `class` of a code block naming its language (`language-rust`).
pub(crate) const LANGUAGE_CLASS_PREFIX: &str = "language-";

/// `target` of links.
pub(crate) const LINK_TARGET: &str = "_top";
/// `class` of links.
pub(crate) const LINK_CLASS: &str = "descriptionLink";

/// `style` of images (and of diagram images).
pub(crate) const IMG: &str = "max-width: 650px;";
/// URL scheme of work item attachments: `workitemimg:ID` (the id percent-encoded).
pub(crate) const ATTACHMENT_SCHEME: &str = "workitemimg:";

/// `style` of a quote (`>`), also of description details; nested quotes nest the div.
pub(crate) const QUOTE: &str = "margin-left: 40px;";

/// `class` of a work item link span:
/// `<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="ID" data-option-id="long"></span>`.
pub(crate) const WORK_ITEM_CLASS: &str = "polarion-rte-link";
/// `data-type` of a work item link span.
pub(crate) const WORK_ITEM_TYPE: &str = "workItem";
/// `data-option-id` of the work item link spans written: the id followed by the title.
pub(crate) const WORK_ITEM_OPTION: &str = "long";
/// `class` of a formula span:
/// `<span data-source="SOURCE"[ data-inline="false"] class="polarion-rte-formula"></span>`
/// (`data-inline="false"` for a display formula).
pub(crate) const FORMULA_CLASS: &str = "polarion-rte-formula";

/// Prefix of an open task (`- [ ]`).
pub(crate) const TASK_OPEN: &str = "☐ ";
/// Prefix of a done task (`- [x]`).
pub(crate) const TASK_DONE: &str = "☑ ";
/// Also read as a done task (not written).
pub(crate) const TASK_CROSSED: &str = "☒ ";

/// An alert (`> [!NOTE]` or `:::note`): its type, name, default title and GitHub's color.
///
/// HTML: `<div style="DIV_STYLE"><span style="TITLE_STYLE">TITLE</span><br/>…</div>`
/// ([`Alert::div_style`], [`Alert::title_style`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Alert {
    pub(crate) kind: AlertType,
    /// Lowercase name as in `:::note`.
    pub(crate) name: &'static str,
    /// The title written when the Markdown names none (comrak's default title).
    pub(crate) title: &'static str,
    pub(crate) color: &'static str,
}

/// The five alert types in comrak's order (note, tip, important, warning, caution).
pub(crate) const ALERTS: [Alert; 5] = [
    Alert {
        kind: AlertType::Note,
        name: "note",
        title: "Note",
        color: "#0969DA",
    },
    Alert {
        kind: AlertType::Tip,
        name: "tip",
        title: "Tip",
        color: "#1A7F37",
    },
    Alert {
        kind: AlertType::Important,
        name: "important",
        title: "Important",
        color: "#8250DF",
    },
    Alert {
        kind: AlertType::Warning,
        name: "warning",
        title: "Warning",
        color: "#9A6700",
    },
    Alert {
        kind: AlertType::Caution,
        name: "caution",
        title: "Caution",
        color: "#CF222E",
    },
];

impl Alert {
    /// The alert of a comrak alert type.
    pub(crate) fn of(alert_type: AlertType) -> Alert {
        // Every type is in the table.
        ALERTS
            .into_iter()
            .find(|a| a.kind == alert_type)
            .unwrap_or(ALERTS[0])
    }

    /// The alert named like this (`note`, ignoring case).
    pub(crate) fn named(name: &str) -> Option<Alert> {
        ALERTS
            .into_iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// `style` of the alert's div.
    pub(crate) fn div_style(self) -> String {
        format!(
            "margin-left: 40px; border-left: 3px solid {}; padding-left: 10px;",
            self.color
        )
    }

    /// `style` of the alert's title span.
    pub(crate) fn title_style(self) -> String {
        format!("font-weight: bold; color: {};", self.color)
    }
}

/// One line break.
pub(crate) const BR: &str = "<br/>";

/// What a block is for the separators around it. A block can start as one kind and end as another
/// (raw HTML such as `<div>…</div> text`): the separator after it uses the kind it ends with, the
/// one before it the kind it starts with (for raw HTML see `html::raw_block_kinds`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockKind {
    /// Inline content: paragraph, heading, diagram image, math block, inline-only raw HTML.
    Text,
    /// A block element that starts and ends its own line: div, table, list, pre, hr, alert.
    Box,
    /// The end of raw HTML ending with an opening tag (`<div style="…">`, alone or after other
    /// content): the following blocks are that element's content.
    Open,
    /// The start of raw HTML starting with a closing tag (`</div>`, alone or before other
    /// content): it ends the element the blocks before it are in.
    Close,
}

/// How many `<br/>` separate two blocks of one container: `prev` is the kind the earlier block
/// ends with, `next` the kind the later one starts with.
///
/// | Situation | Breaks |
/// |---|---|
/// | after a raw opening tag, before a raw closing tag | 0 |
/// | inside `<li>`: between two text blocks | 2 |
/// | inside `<li>`: a box, opening or closing tag involved | 0 |
/// | after a text block | 2 |
/// | after a box or a raw closing tag | 1 |
///
/// Nothing is written before the first block of a container or after the last. Either way the
/// browser shows one blank line between two blocks.
pub(crate) fn separator_breaks(prev: BlockKind, next: BlockKind, in_list_item: bool) -> usize {
    use BlockKind::*;
    match (prev, next) {
        (Open, _) | (_, Close) => 0,
        (Text, Text) if in_list_item => 2,
        _ if in_list_item => 0,
        (Text, _) => 2,
        (Box | Close, _) => 1,
    }
}

/// The separator between two blocks as HTML ([`separator_breaks`] times `<br/>`).
pub(crate) fn separator(prev: BlockKind, next: BlockKind, in_list_item: bool) -> &'static str {
    match separator_breaks(prev, next, in_list_item) {
        0 => "",
        1 => BR,
        _ => "<br/><br/>",
    }
}

#[cfg(test)]
mod tests {
    use super::BlockKind::*;
    use super::*;

    #[test]
    fn heading_styles_follow_the_tables() {
        for level in 1..=6u8 {
            let i = usize::from(level - 1);
            assert_eq!(
                heading(level),
                format!(
                    "font-weight: bold; font-size: {}pt; line-height: {};",
                    HEADING_SIZES[i], HEADING_LINE_HEIGHTS[i]
                )
            );
        }
        assert_eq!(heading(0), heading(1));
        assert_eq!(heading(9), heading(6));
    }

    #[test]
    fn heading_levels_by_size() {
        let cases = [
            (30.0, Some(1)),
            (22.0, Some(1)),
            (20.0, Some(2)),
            (18.0, Some(2)),
            (16.0, Some(3)),
            (14.5, Some(3)),
            (14.0, Some(3)),
            (13.0, Some(4)),
            (12.0, Some(4)),
            (11.0, Some(5)),
            (10.5, Some(6)),
            (10.0, None),
            (8.0, None),
        ];
        for (size, level) in cases {
            assert_eq!(heading_level(size), level, "{size}pt");
        }
        // Every written size reads back as its level.
        for (i, size) in HEADING_SIZES.iter().enumerate() {
            assert_eq!(heading_level(size.parse().unwrap()), Some(i as u8 + 1));
        }
    }

    #[test]
    fn cell_styles_carry_the_alignment() {
        for align in [Align::Left, Align::Center, Align::Right] {
            let text_align = format!("text-align: {};", align.css());
            assert!(th(align).contains(&text_align));
            assert!(td(align).starts_with(&text_align));
            assert_eq!(Align::from_css(align.css()), Some(align));
            assert_eq!(Align::from(TableAlignment::from(align)), align);
        }
        assert_eq!(
            th(Align::Left),
            "font-weight: bold; background-color: #F0F0F0; text-align: left; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;"
        );
        assert_eq!(
            td(Align::Right),
            "text-align: right; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;"
        );
        assert_eq!(Align::from(TableAlignment::None), Align::Left);
        assert_eq!(
            TableAlignment::from(Align::Left),
            TableAlignment::None,
            "left is the default"
        );
        assert_eq!(Align::from_css(" CENTER "), Some(Align::Center));
        assert_eq!(Align::from_css("justify"), None);
    }

    #[test]
    fn alerts() {
        for alert in ALERTS {
            assert_eq!(Alert::of(alert.kind), alert);
            assert_eq!(alert.title, alert.kind.default_title());
            assert_eq!(Alert::named(alert.name), Some(alert));
        }
        assert_eq!(Alert::of(AlertType::Warning).title, "Warning");
        assert_eq!(Alert::named("CAUTION"), Some(ALERTS[4]));
        assert_eq!(Alert::named("custom"), None);
        assert_eq!(
            ALERTS[0].div_style(),
            "margin-left: 40px; border-left: 3px solid #0969DA; padding-left: 10px;"
        );
        assert_eq!(
            ALERTS[0].title_style(),
            "font-weight: bold; color: #0969DA;"
        );
    }

    #[test]
    fn separators() {
        let cases = [
            // (prev, next, in li, expected)
            (Text, Text, false, "<br/><br/>"),
            (Text, Box, false, "<br/><br/>"),
            (Text, Open, false, "<br/><br/>"),
            (Text, Close, false, ""),
            (Box, Text, false, "<br/>"),
            (Box, Box, false, "<br/>"),
            (Box, Open, false, "<br/>"),
            (Box, Close, false, ""),
            (Close, Text, false, "<br/>"),
            (Close, Box, false, "<br/>"),
            (Close, Close, false, ""),
            (Open, Text, false, ""),
            (Open, Box, false, ""),
            (Open, Close, false, ""),
            (Text, Text, true, "<br/><br/>"),
            (Text, Box, true, ""),
            (Box, Text, true, ""),
            (Box, Box, true, ""),
            (Text, Open, true, ""),
            (Close, Text, true, ""),
            (Open, Text, true, ""),
            (Text, Close, true, ""),
        ];
        for (prev, next, li, expected) in cases {
            assert_eq!(
                separator(prev, next, li),
                expected,
                "{prev:?} → {next:?}, li: {li}"
            );
            assert_eq!(
                separator_breaks(prev, next, li),
                expected.matches(BR).count()
            );
        }
    }
}
