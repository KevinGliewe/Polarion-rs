//! Build: the comrak tree of the Markdown.
//!
//! The tree passes `Node::validate`, and comrak's CommonMark writer writes it so that it parses
//! back to the same tree:
//! - inline formatting is nested from the atoms' styles, the longest run outermost (of equally
//!   long ones the one the HTML nests outside, except that italic goes around bold: `***x***`
//!   reads back that way);
//! - whitespace and breaks are hoisted out of emphasis, links and raw wrappers;
//! - emphasis whose delimiters would not open or close where they stand is written as HTML;
//! - superscript inside link text is HTML (comrak reads none there);
//! - a link whose text is its URL is written `[URL](URL)` where an autolink would not read back;
//! - in table cells a formula with a `|` stays its raw span, and a `|` in raw HTML is `&#124;`;
//! - line breaks are hard breaks (a line that is a single HTML tag would otherwise start an HTML
//!   block swallowing the next lines);
//! - no empty `Code`, no breaks in headings and table cells, task items replace items;
//! - escapes comrak's writer lacks are `Raw` nodes (see `escape`).
//!
//! The conservative build (for blocks the self-check rejects) writes emphasis, links, superscript,
//! subscript, images and linked work item ids as HTML, escapes all ASCII punctuation and makes
//! lists loose.

use std::fmt::Write as _;

use comrak::Arena;
use comrak::nodes::{
    AstNode, ListDelimType, ListType, NodeAlert, NodeCode, NodeCodeBlock, NodeHeading,
    NodeHtmlBlock, NodeLink, NodeList, NodeMath, NodeTable, NodeTaskItem, NodeValue,
};

use super::css::{Layer, Wrapper};
use super::escape::{self, Piece, RunCtx, Segment};
use super::lines::LBlock;
use super::walk::{Atom, AtomKind, IdFallback, LinkInfo};
use crate::html::push_attr;
use crate::{RawHtml, ToMarkdownOptions, ids, styles};

pub(super) type Node<'a> = &'a AstNode<'a>;

/// Whether `$$…$$` holds a display formula exactly (no line break, no `$$`, not starting or
/// ending with `$`, not empty).
pub(super) fn display_math(source: &str) -> bool {
    !source.is_empty()
        && !source.contains('\n')
        && !source.contains("$$")
        && !source.starts_with('$')
        && !source.ends_with('$')
}

/// Whether `$…$` holds an inline formula exactly: no whitespace at the edges, every `$` escaped,
/// not starting with a backtick (that is code math), not ending with a backslash.
fn dollar_math(source: &str) -> bool {
    let b = source.as_bytes();
    !b.is_empty()
        && !b[0].is_ascii_whitespace()
        && !b[b.len() - 1].is_ascii_whitespace()
        && b[0] != b'$'
        && b[0] != b'`'
        && b[b.len() - 1] != b'\\'
        && !source.contains('\n')
        && b.iter()
            .enumerate()
            .all(|(i, &c)| c != b'$' || (i > 0 && b[i - 1] == b'\\'))
}

/// Whether `` $`…`$ `` holds an inline formula exactly (see [`code_math_literal`]): no `` `$ ``
/// inside (also not made by a leading `$`), no line break, not empty.
fn code_math(source: &str) -> bool {
    !source.is_empty()
        && !source.starts_with('$')
        && !source.contains("`$")
        && !source.contains('\n')
}

/// Whether a built code math literal has the extra edge spaces of [`code_math_literal`] (the
/// built tree never has both edges a space otherwise).
pub(super) fn padded_code_math(math: &NodeMath) -> bool {
    let l = &math.literal;
    !math.dollar_math
        && !math.display_math
        && l.len() >= 2
        && l.starts_with(' ')
        && l.ends_with(' ')
        && !l.trim_matches(' ').is_empty()
}

/// What to write between `` $` `` and `` `$ `` for a formula: reading drops one space at each
/// edge when both edges are spaces (as in code spans), so such a source gets one more there.
fn code_math_literal(source: String) -> String {
    if source.starts_with(' ') && source.ends_with(' ') && !source.trim_matches(' ').is_empty() {
        format!(" {source} ")
    } else {
        source
    }
}

/// The canonical work item link span (what the forward direction writes for a linked id).
fn work_item_span(id: &str) -> String {
    let mut out = format!(
        "<span class=\"{}\" data-type=\"{}\" id=\"fake\" data-item-id=\"",
        styles::WORK_ITEM_CLASS,
        styles::WORK_ITEM_TYPE
    );
    push_attr(&mut out, id);
    let _ = write!(
        out,
        "\" data-option-id=\"{}\"></span>",
        styles::WORK_ITEM_OPTION
    );
    out
}

/// Inline content before it becomes comrak nodes.
#[derive(Debug, Clone)]
enum Inline {
    Text(String),
    /// A work item id: the bare id where the conversion back links it, else the fallback.
    Id {
        id: String,
        fallback: Fallback,
    },
    Code(String),
    Math {
        source: String,
        display: bool,
        raw: String,
    },
    Html(String),
    Image {
        url: String,
        title: String,
        alt: String,
    },
    /// A line break, written as a hard break (`\` at the line end): a soft one would show as a
    /// space in Markdown viewers, and after a line that is one HTML tag it would let that tag
    /// start an HTML block swallowing the next lines.
    Break,
    Wrap(Kind, Vec<Inline>),
}

/// What a work item id becomes where the conversion back would not link it.
#[derive(Debug, Clone)]
enum Fallback {
    /// The span (keep mode), the id as text (drop mode).
    Raw(String),
    /// A link to the work item with these children (none for an anchor without a label).
    Link {
        url: String,
        title: String,
        children: Vec<Inline>,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Emph,
    Strong,
    Strike,
    Insert,
    Highlight,
    Sup,
    Sub,
    Link { url: String, title: String },
}

impl Kind {
    /// The delimiter character written around the content.
    fn delimiter(&self) -> char {
        match self {
            Kind::Emph | Kind::Strong => '*',
            Kind::Strike | Kind::Sub => '~',
            Kind::Insert => '+',
            Kind::Highlight => '=',
            Kind::Sup => '^',
            Kind::Link { .. } => '[',
        }
    }

    /// The HTML around the content when the delimiters cannot be used.
    fn html(&self) -> (String, String) {
        let span = |style: &str| (format!("<span style=\"{style}\">"), "</span>".to_owned());
        match self {
            Kind::Emph => span(styles::ITALIC),
            Kind::Strong => span(styles::BOLD),
            Kind::Strike => span(styles::STRIKE),
            Kind::Insert => span(styles::UNDERLINE),
            Kind::Highlight => span(styles::HIGHLIGHT),
            Kind::Sup => ("<sup>".to_owned(), "</sup>".to_owned()),
            Kind::Sub => ("<sub>".to_owned(), "</sub>".to_owned()),
            Kind::Link { url, title } => {
                let mut open = String::from("<a href=\"");
                push_attr(&mut open, url);
                open.push('"');
                if !title.is_empty() {
                    open.push_str(" title=\"");
                    push_attr(&mut open, title);
                    open.push('"');
                }
                let _ = write!(
                    open,
                    " target=\"{}\" class=\"{}\">",
                    styles::LINK_TARGET,
                    styles::LINK_CLASS
                );
                (open, "</a>".to_owned())
            }
        }
    }
}

/// A formatting property to nest inline content by.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mark {
    Link(usize),
    Wrap(usize),
    Italic,
    Bold,
    Strike,
    Underline,
    Highlight,
    Sup,
    Sub,
}

impl Mark {
    fn layer(self) -> Layer {
        match self {
            Mark::Link(_) => Layer::Link,
            Mark::Wrap(w) => Layer::Wrap(w),
            Mark::Italic => Layer::Italic,
            Mark::Bold => Layer::Bold,
            Mark::Strike => Layer::Strike,
            Mark::Underline => Layer::Underline,
            Mark::Highlight => Layer::Highlight,
            Mark::Sup => Layer::Sup,
            Mark::Sub => Layer::Sub,
        }
    }
}

/// Where inline content is.
#[derive(Debug, Clone, Copy, Default)]
struct Place {
    /// In link text or image alternative text: no autolinks, ids are not linked.
    in_link: bool,
    cell: bool,
    /// The first child starts a line.
    line_start: bool,
    /// The delimiter character of the parent's emphasis.
    delim: Option<char>,
}

pub(super) struct Builder<'a, 'o> {
    pub(super) arena: &'a Arena<'a>,
    pub(super) options: &'o ToMarkdownOptions,
    pub(super) links: &'o [LinkInfo],
    pub(super) wrappers: &'o [Wrapper],
    pub(super) conservative: bool,
}

/// Whitespace or a break: hoisted out of formatting.
fn spacey(atom: &Atom) -> bool {
    match &atom.kind {
        AtomKind::Text(t) => t.bytes().all(|b| b == b' '),
        AtomKind::Br | AtomKind::RawBr => true,
        _ => false,
    }
}

/// Whether a destination has a space comrak's writer would percent-encode.
fn has_space(url: &str) -> bool {
    url.bytes()
        .any(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
}

fn is_whitespace(c: char) -> bool {
    c.is_whitespace()
}

/// Punctuation as comrak's flanking rules see it: a Unicode punctuation or symbol character. Not
/// alphanumeric and not whitespace comes close; combining marks, format characters (soft hyphen,
/// zero-width space, ...), controls and private use characters are neither.
fn is_punctuation(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_punctuation();
    }
    !(c.is_alphanumeric() || c.is_whitespace() || c.is_control() || is_mark_or_format(c))
}

/// Whether a character is (most likely) a combining mark, a format character or a private use
/// character: the main blocks of them.
fn is_mark_or_format(c: char) -> bool {
    // Punctuation and symbols among the Indic marks.
    if matches!(
        u32::from(c),
        0x09F2 | 0x09F3 | 0x09FA | 0x09FB | 0x0AF0 | 0x0AF1 | 0x0B70 | 0x0BF3
            ..=0x0BFA | 0x0C77 | 0x0C7F | 0x0C84 | 0x0D4F | 0x0D79
    ) {
        return false;
    }
    matches!(u32::from(c),
        0x0300..=0x036F
        | 0x0483..=0x0489
        | 0x0591..=0x05BD
        | 0x05BF
        | 0x05C1..=0x05C2
        | 0x05C4..=0x05C5
        | 0x05C7
        | 0x0610..=0x061A
        | 0x064B..=0x065F
        | 0x0670
        | 0x06D6..=0x06DC
        | 0x06DF..=0x06E4
        | 0x06E7..=0x06E8
        | 0x06EA..=0x06ED
        | 0x0711
        | 0x0730..=0x074A
        | 0x0900..=0x0963
        | 0x0966..=0x096F
        | 0x0971..=0x0DF3
        | 0x0E31..=0x0E3A
        | 0x0E47..=0x0E4E
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x20D0..=0x20FF
        | 0x302A..=0x302F
        | 0x3099..=0x309A
        | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F
        | 0xE0100..=0xE01EF
        // Format characters.
        | 0x00AD
        | 0x0600..=0x0605
        | 0x061C
        | 0x06DD
        | 0x070F
        | 0x180E
        | 0x200B..=0x200F
        | 0x202A..=0x202E
        | 0x2060..=0x2064
        | 0x2066..=0x206F
        | 0xFEFF
        | 0xFFF9..=0xFFFB
        | 0xE0001
        | 0xE0020..=0xE007F
        // Private use.
        | 0xE000..=0xF8FF
        | 0xF0000..=0x10FFFF
    )
}

/// The first character written for an inline (`in_emphasis`: whitespace there becomes `&#N;`).
fn first_char(item: &Inline, in_emphasis: bool) -> char {
    match item {
        Inline::Text(t) => match t.chars().next() {
            Some(c) if in_emphasis && c.is_whitespace() => '&',
            Some(c) => c,
            None => ' ',
        },
        Inline::Id { id, .. } => id.chars().next().unwrap_or('A'),
        Inline::Code(_) => '`',
        Inline::Math { .. } => '$',
        Inline::Html(_) => '<',
        Inline::Image { .. } => '!',
        Inline::Break => '\n',
        Inline::Wrap(kind, _) => kind.delimiter(),
    }
}

/// The last character written for an inline.
fn last_char(item: &Inline, in_emphasis: bool) -> char {
    match item {
        Inline::Text(t) => match t.chars().next_back() {
            Some(c) if in_emphasis && c.is_whitespace() => ';',
            Some(c) => c,
            None => ' ',
        },
        Inline::Id { id, .. } => id.chars().next_back().unwrap_or('1'),
        Inline::Code(_) => '`',
        Inline::Math { .. } => '$',
        Inline::Html(_) => '>',
        Inline::Image { .. } | Inline::Wrap(Kind::Link { .. }, _) => ')',
        Inline::Break => '\n',
        Inline::Wrap(kind, _) => kind.delimiter(),
    }
}

/// Whether an inline is written between `~` delimiters (strikethrough, subscript).
fn tilde_delimited(item: &Inline) -> bool {
    matches!(item, Inline::Wrap(kind, _) if kind.delimiter() == '~')
}

/// Whether comrak looks through this character for the one next to a delimiter run (it skips
/// `~`, `=` and `+` there).
fn skipped(c: char) -> bool {
    matches!(c, '~' | '=' | '+')
}

/// Whether the delimiters of `kind` disappear for comrak next to a run of `run`: they join the run
/// (the same character) or are looked through (`~ = +`).
fn see_through(kind: &Kind, run: char) -> bool {
    let d = kind.delimiter();
    d != '[' && (d == run || skipped(d))
}

/// The character comrak sees after a run of `run` that `item` follows: [`first_char`], looking
/// through delimiters that join the run or that comrak skips.
fn first_seen(item: &Inline, run: char, in_emphasis: bool) -> char {
    match item {
        Inline::Wrap(kind, children) if see_through(kind, run) => children
            .first()
            .map_or('\n', |c| first_seen(c, run, *kind != Kind::Insert)),
        other => first_char(other, in_emphasis),
    }
}

/// The character comrak sees before a run of `run` that follows `item` (see [`first_seen`]).
fn last_seen(item: &Inline, run: char, in_emphasis: bool) -> char {
    match item {
        Inline::Wrap(kind, children) if see_through(kind, run) => children
            .last()
            .map_or('\n', |c| last_seen(c, run, *kind != Kind::Insert)),
        other => last_char(other, in_emphasis),
    }
}

/// One side of inline content: the character written there (a parent's delimiter, `[`, a tag's
/// `>`, a line break) and what comrak sees beyond it when that character joins a delimiter run
/// of the content or is skipped.
#[derive(Debug, Clone, Copy)]
struct Side {
    written: char,
    beyond: char,
}

impl Side {
    fn plain(c: char) -> Side {
        Side {
            written: c,
            beyond: c,
        }
    }

    /// What comrak sees next to a run of `run` at this side.
    fn seen(self, run: char) -> char {
        if self.written == run || skipped(self.written) {
            self.beyond
        } else {
            self.written
        }
    }
}

/// Whether an HTML inline is the opening tag of a wrapper (`<span style="…">`, `<sup>`, ...).
fn is_open_tag(html: &str) -> bool {
    html.starts_with('<')
        && !html.starts_with("</")
        && !html.contains("</")
        && !html.ends_with("/>")
}

/// Whether an HTML inline is a closing tag.
fn is_close_tag(html: &str) -> bool {
    html.starts_with("</") && !html[2..].contains('<')
}

/// The spaces ending the last text of `items`, taken off it.
fn take_trailing_spaces(items: &mut Vec<Inline>) -> String {
    let Some(Inline::Text(t)) = items.last_mut() else {
        return String::new();
    };
    let kept = t.trim_end_matches(' ').len();
    let spaces = t.split_off(kept);
    if t.is_empty() {
        items.pop();
    }
    spaces
}

/// The spaces starting the first text of `items`, taken off it.
fn take_leading_spaces(items: &mut Vec<Inline>) -> String {
    let Some(Inline::Text(t)) = items.first_mut() else {
        return String::new();
    };
    let lead = t.len() - t.trim_start_matches(' ').len();
    let spaces: String = t.drain(..lead).collect();
    if t.is_empty() {
        items.remove(0);
    }
    spaces
}

/// Spaces at the edges of formatting and HTML wrappers moved outside them, and runs of spaces
/// collapsed, as reading the Markdown back does. Grouping hoists them already; this catches what
/// is left after the work item ids are decided.
fn hoist(items: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Inline::Wrap(kind, children) => {
                let mut children = hoist(children);
                let lead = take_leading_spaces(&mut children);
                let trail = take_trailing_spaces(&mut children);
                push_text(&mut out, &lead);
                if !children.is_empty() || matches!(kind, Kind::Link { .. }) {
                    out.push(Inline::Wrap(kind, children));
                }
                push_text(&mut out, &trail);
            }
            Inline::Id {
                id,
                fallback:
                    Fallback::Link {
                        url,
                        title,
                        children,
                    },
            } => out.push(Inline::Id {
                id,
                fallback: Fallback::Link {
                    url,
                    title,
                    children: hoist(children),
                },
            }),
            Inline::Html(close) if is_close_tag(&close) => {
                let trail = take_trailing_spaces(&mut out);
                out.push(Inline::Html(close));
                push_text(&mut out, &trail);
            }
            Inline::Text(t)
                if t.starts_with(' ')
                    && matches!(out.last(), Some(Inline::Html(open)) if is_open_tag(open)) =>
            {
                let rest = t.trim_start_matches(' ');
                if let Some(open) = out.pop() {
                    push_text(&mut out, &t[..t.len() - rest.len()]);
                    out.push(open);
                }
                push_text(&mut out, rest);
            }
            Inline::Text(t) => push_text(&mut out, &t),
            // Adjacent code spans would read back as one with the backticks inside.
            Inline::Code(code) => match out.last_mut() {
                Some(Inline::Code(last)) => last.push_str(&code),
                _ => out.push(Inline::Code(code)),
            },
            other => out.push(other),
        }
    }
    for item in &mut out {
        if let Inline::Text(t) = item
            && t.contains("  ")
        {
            let mut collapsed = String::with_capacity(t.len());
            for c in t.chars() {
                if !(c == ' ' && collapsed.ends_with(' ')) {
                    collapsed.push(c);
                }
            }
            *t = collapsed;
        }
    }
    out
}

/// Removes the spaces at the start and end of each line (also the `<br/>` lines of cells): Markdown
/// drops them when reading.
fn trim_lines(items: &mut Vec<Inline>) {
    let n = items.len();
    for k in 0..n {
        let line_break = |item: &Inline| {
            matches!(item, Inline::Break) || matches!(item, Inline::Html(h) if h == styles::BR)
        };
        let starts = k == 0 || line_break(&items[k - 1]);
        let ends = k + 1 == n || line_break(&items[k + 1]);
        if let Inline::Text(t) = &mut items[k] {
            if ends {
                t.truncate(t.trim_end_matches(' ').len());
            }
            if starts {
                let lead = t.len() - t.trim_start_matches(' ').len();
                t.drain(..lead);
            }
        }
    }
    items.retain(|item| !matches!(item, Inline::Text(t) if t.is_empty()));
}

fn push_text(out: &mut Vec<Inline>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(Inline::Text(last)) = out.last_mut() {
        last.push_str(text);
    } else {
        out.push(Inline::Text(text.to_owned()));
    }
}

/// Whether `first` (an item's first block) is a bullet list whose first item starts with a bullet
/// list: the item's line would be `- - -`, a thematic break when nothing follows on it.
fn bullets_on_one_line<'a>(first: &'a AstNode<'a>) -> bool {
    let bullet = |node: &'a AstNode<'a>| matches!(&node.data.borrow().value, NodeValue::List(list) if list.list_type == ListType::Bullet);
    bullet(first)
        && first.first_child().is_some_and(|item| {
            matches!(item.data.borrow().value, NodeValue::Item(_))
                && item.first_child().is_some_and(bullet)
        })
}

impl<'a> Builder<'a, '_> {
    fn keep(&self) -> bool {
        self.options.raw_html == RawHtml::Keep
    }

    fn node(&self, value: NodeValue) -> Node<'a> {
        self.arena.alloc(value.into())
    }

    /// The nodes of a block (several for a raw wrapper: its tags around its blocks).
    pub(super) fn block(&self, block: &LBlock, out: &mut Vec<Node<'a>>) {
        match block {
            LBlock::Para(atoms) => out.extend(self.paragraph(atoms)),
            LBlock::Heading(level, atoms) => {
                let heading = self.node(NodeValue::Heading(NodeHeading {
                    level: (*level).clamp(1, 6),
                    setext: false,
                    closed: false,
                }));
                let items = self.inlines(atoms, true);
                self.convert(items, heading, Place::default());
                out.push(heading);
            }
            LBlock::MathBlock(source) => out.push(self.code_block("math", source)),
            LBlock::Quote(depth, blocks) => {
                let outer = self.node(NodeValue::BlockQuote);
                let mut inner = outer;
                for _ in 1..*depth {
                    let quote = self.node(NodeValue::BlockQuote);
                    inner.append(quote);
                    inner = quote;
                }
                self.blocks_into(blocks, inner);
                out.push(outer);
            }
            LBlock::Alert(kind, title, blocks) => {
                let alert = self.node(NodeValue::Alert(Box::new(NodeAlert {
                    alert_type: *kind,
                    title: title.as_deref().map(alert_title),
                    multiline: false,
                    fence_length: 0,
                    fence_offset: 0,
                })));
                self.blocks_into(blocks, alert);
                out.push(alert);
            }
            LBlock::Code(lang, text) => out.push(self.code_block(lang, text)),
            LBlock::Diagram(source) => out.push(self.code_block("mermaid", source)),
            LBlock::List {
                ordered,
                start,
                tight,
                items,
            } => {
                let list = NodeList {
                    list_type: if *ordered {
                        ListType::Ordered
                    } else {
                        ListType::Bullet
                    },
                    marker_offset: 0,
                    padding: 2,
                    start: if *ordered { *start } else { 1 },
                    delimiter: ListDelimType::Period,
                    bullet_char: b'-',
                    tight: *tight && !self.conservative,
                    is_task_list: items.iter().any(|(task, _)| task.is_some()),
                };
                let node = self.node(NodeValue::List(list));
                for (task, blocks) in items {
                    let value = match task {
                        Some(done) => NodeValue::TaskItem(NodeTaskItem {
                            symbol: done.then_some('x'),
                            symbol_sourcepos: (0, 0, 0, 0).into(),
                        }),
                        None => NodeValue::Item(list),
                    };
                    let item = self.node(value);
                    self.blocks_into(blocks, item);
                    // comrak's writer puts a blank line before these, which would end the item.
                    let blank_before = item.first_child().map(|first| {
                        matches!(
                            first.data.borrow().value,
                            NodeValue::ThematicBreak
                                | NodeValue::HtmlBlock(_)
                                | NodeValue::Table(_)
                        ) || bullets_on_one_line(first)
                    });
                    match blank_before {
                        // An empty paragraph writes nothing but the blank line after the item,
                        // which comrak's writer otherwise leaves out: a paragraph after the list
                        // would continue the empty item.
                        None => item.append(self.node(NodeValue::Paragraph)),
                        // A comment (dropped by the conversion back) comes first instead.
                        Some(true) => {
                            let paragraph = self.node(NodeValue::Paragraph);
                            paragraph.append(self.node(NodeValue::HtmlInline("<!-- -->".into())));
                            item.prepend(paragraph);
                        }
                        Some(false) => {}
                    }
                    node.append(item);
                }
                out.push(node);
            }
            LBlock::Table { aligns, rows } => {
                let table = self.node(NodeValue::Table(Box::new(NodeTable {
                    alignments: aligns.clone(),
                    num_columns: aligns.len(),
                    num_rows: rows.len(),
                    num_nonempty_cells: 0,
                })));
                for (r, row) in rows.iter().enumerate() {
                    let row_node = self.node(NodeValue::TableRow(r == 0));
                    for cell in row.iter().take(aligns.len()) {
                        let cell_node = self.node(NodeValue::TableCell);
                        let items = self.cell_code(self.inlines(cell, false));
                        self.convert(
                            items,
                            cell_node,
                            Place {
                                cell: true,
                                ..Place::default()
                            },
                        );
                        row_node.append(cell_node);
                    }
                    for _ in row.len()..aligns.len() {
                        row_node.append(self.node(NodeValue::TableCell));
                    }
                    table.append(row_node);
                }
                out.push(table);
            }
            LBlock::Raw(html) => out.push(self.html_block(html.clone())),
            LBlock::RawWrap(open, blocks) => {
                out.push(self.html_block(open.clone()));
                for block in blocks {
                    self.block(block, out);
                }
                out.push(self.html_block("</div>".to_owned()));
            }
            LBlock::Rule => out.push(self.node(NodeValue::ThematicBreak)),
        }
    }

    fn blocks_into(&self, blocks: &[LBlock], parent: Node<'a>) {
        let mut nodes = Vec::new();
        for block in blocks {
            self.block(block, &mut nodes);
        }
        for node in nodes {
            parent.append(node);
        }
    }

    pub(super) fn html_block(&self, literal: String) -> Node<'a> {
        self.node(NodeValue::HtmlBlock(NodeHtmlBlock {
            block_type: 6,
            literal,
        }))
    }

    fn code_block(&self, info: &str, text: &str) -> Node<'a> {
        self.node(NodeValue::CodeBlock(Box::new(NodeCodeBlock {
            fenced: true,
            fence_char: b'`',
            fence_length: 3,
            fence_offset: 0,
            info: info.to_owned(),
            literal: format!("{text}\n"),
            closed: true,
        })))
    }

    fn paragraph(&self, atoms: &[Atom]) -> Option<Node<'a>> {
        let items = self.inlines(atoms, false);
        if items.is_empty() {
            return None;
        }
        let paragraph = self.node(NodeValue::Paragraph);
        self.convert(
            items,
            paragraph,
            Place {
                line_start: true,
                ..Place::default()
            },
        );
        Some(paragraph)
    }

    /// Atoms to inline content: nested, ids decided, delimiters checked.
    fn inlines(&self, atoms: &[Atom], heading: bool) -> Vec<Inline> {
        let atoms: Vec<Atom> = atoms
            .iter()
            .filter(|a| !matches!(a.kind, AtomKind::Empty))
            .cloned()
            .collect();
        let mut applied = Vec::new();
        let grouped = self.group(&atoms, &mut applied, heading);
        let mut resolved = hoist(self.resolve(grouped, false));
        trim_lines(&mut resolved);
        self.flank(&resolved, Side::plain('\n'), Side::plain('\n'))
    }

    fn has(&self, atom: &Atom, mark: Mark, heading: bool) -> bool {
        let s = &atom.style;
        match mark {
            Mark::Link(i) => s.link == Some(i),
            Mark::Wrap(w) => s.wraps.contains(&w),
            Mark::Italic => s.italic,
            Mark::Bold => s.bold && !heading,
            Mark::Strike => s.strike,
            Mark::Underline => s.underline,
            Mark::Highlight => s.highlight,
            Mark::Sup => s.sup,
            Mark::Sub => s.sub,
        }
    }

    /// Whether a raw wrapper is written (keep mode, something to write).
    fn effective(&self, wrapper: usize, heading: bool) -> bool {
        self.keep()
            && self
                .wrappers
                .get(wrapper)
                .is_some_and(|w| w.writes(heading))
    }

    /// The marks to nest by at this atom, outermost first when runs are equally long.
    fn candidates(&self, atom: &Atom, applied: &[Mark], heading: bool) -> Vec<Mark> {
        let s = &atom.style;
        let mut marks = Vec::new();
        if let Some(link) = s.link {
            marks.push(Mark::Link(link));
        }
        for &w in &s.wraps {
            if self.effective(w, heading) {
                marks.push(Mark::Wrap(w));
            }
        }
        marks.extend([
            Mark::Italic,
            Mark::Bold,
            Mark::Strike,
            Mark::Underline,
            Mark::Highlight,
            Mark::Sup,
            Mark::Sub,
        ]);
        marks.retain(|&m| !applied.contains(&m) && self.has(atom, m, heading));
        // The order the elements nested them in comes first.
        marks.sort_by_key(|&m| s.rank(m.layer()));
        marks
    }

    fn leaf(&self, atom: &Atom) -> Option<Inline> {
        Some(match &atom.kind {
            AtomKind::Text(t) => Inline::Text(t.clone()),
            AtomKind::Br => Inline::Break,
            AtomKind::RawBr => Inline::Html(styles::BR.to_owned()),
            AtomKind::Empty => return None,
            AtomKind::Image { url, title, alt } => {
                // comrak's writer percent-encodes the spaces of a destination (another URL).
                if self.conservative || has_space(url) {
                    let mut html = String::from("<img src=\"");
                    push_attr(&mut html, url);
                    html.push('"');
                    if !title.is_empty() {
                        html.push_str(" title=\"");
                        push_attr(&mut html, title);
                        html.push('"');
                    }
                    if !alt.is_empty() {
                        html.push_str(" alt=\"");
                        push_attr(&mut html, alt);
                        html.push('"');
                    }
                    let _ = write!(html, " style=\"{}\"/>", styles::IMG);
                    Inline::Html(html)
                } else {
                    Inline::Image {
                        url: url.clone(),
                        title: title.clone(),
                        alt: alt.clone(),
                    }
                }
            }
            AtomKind::Html(html) => Inline::Html(html.clone()),
            AtomKind::Id { id, fallback } => Inline::Id {
                id: id.clone(),
                fallback: match fallback {
                    IdFallback::Span(raw) => Fallback::Raw(raw.clone()),
                    IdFallback::Link { url, title } => Fallback::Link {
                        url: url.clone(),
                        title: title.clone(),
                        children: Vec::new(),
                    },
                },
            },
            AtomKind::Math {
                source,
                display,
                raw,
            } => Inline::Math {
                source: source.clone(),
                display: *display,
                raw: raw.clone(),
            },
        })
    }

    /// Nests atoms by their marks: at each atom the mark with the longest run (of equally long
    /// ones the one the HTML nests outside) becomes the outermost node of that run. Spaces and breaks ending a run
    /// do not count: they are hoisted out of it.
    fn group(&self, atoms: &[Atom], applied: &mut Vec<Mark>, heading: bool) -> Vec<Inline> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < atoms.len() {
            let atom = &atoms[i];
            if spacey(atom) {
                out.extend(self.leaf(atom));
                i += 1;
                continue;
            }
            // Each mark's run here: its extent without trailing spaces, its length.
            let mut runs: Vec<(usize, usize, Mark)> = Vec::new();
            let mut best: Option<(usize, usize, Mark)> = None;
            for mark in self.candidates(atom, applied, heading) {
                let run = &atoms[i..];
                let len = run
                    .iter()
                    .take_while(|a| self.has(a, mark, heading))
                    .count();
                let core = run[..len]
                    .iter()
                    .rposition(|a| !spacey(a))
                    .map_or(0, |p| p + 1);
                runs.push((core, len, mark));
                if best.is_none_or(|(c, _, _)| core > c) {
                    best = Some((core, len, mark));
                }
            }
            // Bold right around italic of the same extent is written `***x***`, which reads back
            // as italic around bold: italic goes outside (the two look the same).
            if let Some((core, _, Mark::Bold)) = best
                && let Some(&(_, len, Mark::Italic)) = runs
                    .iter()
                    .find(|(c, _, mark)| *mark != Mark::Bold && *c == core)
            {
                best = Some((core, len, Mark::Italic));
            }
            let best = best.map(|(_, len, mark)| (len, mark));
            let Some((len, mark)) = best else {
                if atom.style.mono && matches!(atom.kind, AtomKind::Text(_)) {
                    // Monospace text: one code span (no mark left on these atoms).
                    let end = i + atoms[i..]
                        .iter()
                        .take_while(|a| {
                            a.style.mono
                                && matches!(a.kind, AtomKind::Text(_))
                                && self.candidates(a, applied, heading).is_empty()
                        })
                        .count();
                    let mut literal = String::new();
                    for a in &atoms[i..end] {
                        if let AtomKind::Text(t) = &a.kind {
                            literal.push_str(t);
                        }
                    }
                    let core = literal.trim_matches(' ');
                    if core.is_empty() {
                        push_text(&mut out, &literal);
                    } else {
                        let lead = literal.len() - literal.trim_start_matches(' ').len();
                        push_text(&mut out, &literal[..lead]);
                        if code_span_writable(core) {
                            out.push(Inline::Code(core.to_owned()));
                        } else if self.keep() {
                            out.extend(self.around(
                                format!("<span style=\"{}\">", styles::MONO),
                                vec![Inline::Text(core.to_owned())],
                                "</span>".to_owned(),
                            ));
                        } else {
                            push_text(&mut out, core);
                        }
                        push_text(&mut out, &literal[lead + core.len()..]);
                    }
                    i = end;
                } else {
                    out.extend(self.leaf(atom));
                    i += 1;
                }
                continue;
            };
            let run = &atoms[i..i + len];
            let core_len = run.iter().rposition(|a| !spacey(a)).map_or(0, |p| p + 1);
            let mut core: Vec<Atom> = run[..core_len].to_vec();
            // Spaces at the edges of the run's text go outside.
            let mut lead = String::new();
            let mut trail = String::new();
            if let Some(Atom {
                kind: AtomKind::Text(t),
                ..
            }) = core.first_mut()
            {
                let trimmed = t.trim_start_matches(' ');
                lead = t[..t.len() - trimmed.len()].to_owned();
                *t = trimmed.to_owned();
            }
            if let Some(Atom {
                kind: AtomKind::Text(t),
                ..
            }) = core.last_mut()
            {
                let trimmed = t.trim_end_matches(' ').len();
                trail = t[trimmed..].to_owned();
                t.truncate(trimmed);
            }
            core.retain(|a| !matches!(&a.kind, AtomKind::Text(t) if t.is_empty()));
            push_text(&mut out, &lead);
            applied.push(mark);
            let inner = self.group(&core, applied, heading);
            applied.pop();
            if !inner.is_empty() {
                out.extend(self.wrap(mark, inner, applied, heading));
            }
            push_text(&mut out, &trail);
            for a in &run[core_len..] {
                out.extend(self.leaf(a));
            }
            i += len;
        }
        out
    }

    /// The node(s) of a mark around its content.
    fn wrap(&self, mark: Mark, inner: Vec<Inline>, applied: &[Mark], heading: bool) -> Vec<Inline> {
        let in_link = applied.iter().any(|m| matches!(m, Mark::Link(_)));
        let kind = match mark {
            Mark::Wrap(w) => {
                let open = self
                    .wrappers
                    .get(w)
                    .and_then(|w| w.open_tag(heading))
                    .unwrap_or_default();
                return self.around(open, inner, "</span>".to_owned());
            }
            Mark::Link(i) => {
                let info = &self.links[i];
                if let Some(id) = &info.work_item
                    && plain_text(&inner).trim() == id
                {
                    return vec![Inline::Id {
                        id: id.clone(),
                        fallback: Fallback::Link {
                            url: info.url.clone(),
                            title: info.title.clone(),
                            children: inner,
                        },
                    }];
                }
                Kind::Link {
                    url: info.url.clone(),
                    title: info.title.clone(),
                }
            }
            Mark::Italic => Kind::Emph,
            Mark::Bold => Kind::Strong,
            Mark::Strike => Kind::Strike,
            Mark::Underline => Kind::Insert,
            Mark::Highlight => Kind::Highlight,
            Mark::Sup => Kind::Sup,
            Mark::Sub => Kind::Sub,
        };
        // comrak's writer percent-encodes the spaces of a destination: such a link reads back
        // with another URL, so it is HTML (in both modes, the URL is content).
        let unwritable = matches!(&kind, Kind::Link { url, .. } if has_space(url));
        if self.conservative || unwritable || (kind == Kind::Sup && in_link) {
            let (open, close) = kind.html();
            return self.around(open, inner, close);
        }
        vec![Inline::Wrap(kind, inner)]
    }

    /// Inline content of a table cell, where a `|` would end the cell. Code with `\|` in it:
    /// comrak reads the pipe escape it writes there as a backslash and a pipe, so such code is
    /// monospace HTML (keep mode) or text. A formula with a `|` reads back changed the same way:
    /// it stays its raw span (both modes). In HTML a `|` is written `&#124;`.
    fn cell_code(&self, items: Vec<Inline>) -> Vec<Inline> {
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Inline::Math { source, raw, .. } if source.contains('|') => {
                    out.push(Inline::Html(raw.replace('|', "&#124;")));
                }
                Inline::Html(html) if html.contains('|') => {
                    out.push(Inline::Html(html.replace('|', "&#124;")));
                }
                Inline::Code(code) if code.contains("\\|") => {
                    if self.keep() {
                        out.extend(self.around(
                            format!("<span style=\"{}\">", styles::MONO),
                            vec![Inline::Text(code)],
                            "</span>".to_owned(),
                        ));
                    } else {
                        push_text(&mut out, &code);
                    }
                }
                Inline::Wrap(kind, children) => {
                    out.push(Inline::Wrap(kind, self.cell_code(children)))
                }
                other => out.push(other),
            }
        }
        out
    }

    fn around(&self, open: String, inner: Vec<Inline>, close: String) -> Vec<Inline> {
        let mut out = Vec::with_capacity(inner.len() + 2);
        out.push(Inline::Html(open));
        out.extend(inner);
        out.push(Inline::Html(close));
        out
    }

    /// Decides the work item ids by their runs: the bare id where the conversion back links it,
    /// the fallback elsewhere.
    fn resolve(&self, items: Vec<Inline>, in_link: bool) -> Vec<Inline> {
        let items: Vec<Inline> = items
            .into_iter()
            .map(|item| match item {
                Inline::Wrap(kind, children) => {
                    let link = in_link || matches!(kind, Kind::Link { .. });
                    Inline::Wrap(kind, self.resolve(children, link))
                }
                Inline::Id {
                    id,
                    fallback:
                        Fallback::Link {
                            url,
                            title,
                            children,
                        },
                } => Inline::Id {
                    id,
                    fallback: Fallback::Link {
                        url,
                        title,
                        children: self.resolve(children, true),
                    },
                },
                other => other,
            })
            .collect();
        let member = |item: &Inline| matches!(item, Inline::Text(_) | Inline::Id { .. });
        let mut linked = vec![false; items.len()];
        let mut i = 0;
        while i < items.len() {
            if !member(&items[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < items.len() && member(&items[i]) {
                i += 1;
            }
            let mut text = String::new();
            let mut ranges = Vec::new();
            for (k, item) in items[start..i].iter().enumerate() {
                match item {
                    Inline::Text(t) => text.push_str(t),
                    Inline::Id { id, .. } => {
                        ranges.push((start + k, text.len()..text.len() + id.len()));
                        text.push_str(id);
                    }
                    _ => {}
                }
            }
            for (k, range) in ranges {
                linked[k] = !in_link && ids::links(&text, &[], range, &self.options.work_items);
            }
        }
        let mut out = Vec::with_capacity(items.len());
        for (k, item) in items.into_iter().enumerate() {
            match item {
                Inline::Id { id, fallback } if !linked[k] => match fallback {
                    Fallback::Raw(raw) if self.keep() => out.push(Inline::Html(raw)),
                    Fallback::Raw(_) => push_text(&mut out, &id),
                    Fallback::Link {
                        url,
                        title,
                        children,
                    } => {
                        let kind = Kind::Link { url, title };
                        if self.conservative {
                            let (open, close) = kind.html();
                            out.extend(self.around(open, children, close));
                        } else {
                            out.push(Inline::Wrap(kind, children));
                        }
                    }
                },
                Inline::Id { id, .. } if self.conservative => {
                    out.push(Inline::Html(work_item_span(&id)));
                }
                Inline::Text(t) => push_text(&mut out, &t),
                other => out.push(other),
            }
        }
        out
    }

    /// Emphasis whose delimiters would not open or close where they stand is written as HTML,
    /// judged by the characters comrak sees next to each delimiter run (CommonMark's flanking
    /// rules; runs of one character join, `~ = +` are looked through). Inner emphasis is decided
    /// first: written as HTML it changes the characters its parent's delimiters stand next to.
    fn flank(&self, items: &[Inline], before: Side, after: Side) -> Vec<Inline> {
        let n = items.len();
        let mut out = Vec::with_capacity(n);
        for (k, item) in items.iter().enumerate() {
            let Inline::Wrap(kind, children) = item else {
                out.push(item.clone());
                continue;
            };
            if let Kind::Link { .. } = kind {
                let children = self.flank(children, Side::plain('['), Side::plain(']'));
                out.push(Inline::Wrap(kind.clone(), children));
                continue;
            }
            let d = kind.delimiter();
            // The characters next to the delimiter runs: as written and as comrak sees them.
            let (prev_written, prev) = match k {
                0 => (before.written, before.seen(d)),
                _ => (
                    last_char(&items[k - 1], false),
                    last_seen(&items[k - 1], d, false),
                ),
            };
            let (next_written, next) = match items.get(k + 1) {
                None => (after.written, after.seen(d)),
                Some(item) => (first_char(item, false), first_seen(item, d, false)),
            };
            let inner = self.flank(
                children,
                Side {
                    written: d,
                    beyond: prev,
                },
                Side {
                    written: d,
                    beyond: next,
                },
            );
            // comrak's writer writes whitespace at the edges of emphasis as `&#N;`, but not
            // inside an insert.
            let entity = *kind != Kind::Insert;
            let first = inner.first().map_or(' ', |c| first_seen(c, d, entity));
            let last = inner.last().map_or(' ', |c| last_seen(c, d, entity));
            let opens = !is_whitespace(first)
                && (!is_punctuation(first)
                    || is_whitespace(prev)
                    || is_punctuation(prev)
                    || d == '^'
                    || d == '~');
            let closes = !is_whitespace(last)
                && (!is_punctuation(last) || is_whitespace(next) || is_punctuation(next));
            // `~` runs next to each other (strikethrough and subscript) merge; a `~` of text is
            // escaped and joins none.
            let tilde = |item: Option<&Inline>| item.is_some_and(tilde_delimited);
            let tildes = d == '~'
                && (prev_written == '~' && k == 0
                    || next_written == '~' && k + 1 == n
                    || k > 0 && tilde(items.get(k - 1))
                    || tilde(items.get(k + 1))
                    || tilde(inner.first())
                    || tilde(inner.last()));
            if opens && closes && !tildes {
                out.push(Inline::Wrap(kind.clone(), inner));
            } else {
                let (open, close) = kind.html();
                let inner = self.flank(children, Side::plain('>'), Side::plain('<'));
                out.extend(self.around(open, inner, close));
            }
        }
        out
    }

    /// Inline content as comrak nodes under `parent`.
    fn convert(&self, items: Vec<Inline>, parent: Node<'a>, place: Place) {
        let n = items.len();
        let member: Vec<bool> = items
            .iter()
            .map(|item| matches!(item, Inline::Text(_) | Inline::Id { .. }))
            .collect();
        // The delimiter character an item writes next to its neighbors (emphasis).
        let edge: Vec<Option<char>> = items
            .iter()
            .map(|item| match item {
                Inline::Wrap(Kind::Link { .. }, _) => None,
                Inline::Wrap(kind, _) => Some(kind.delimiter()),
                _ => None,
            })
            .collect();
        let digit: Vec<bool> = items
            .iter()
            .map(|item| first_char(item, false).is_ascii_digit())
            .collect();
        let breaks: Vec<bool> = items
            .iter()
            .map(|item| matches!(item, Inline::Break))
            .collect();
        let mut run: Vec<Inline> = Vec::new();
        let mut run_start = 0;
        for (k, item) in items.into_iter().enumerate() {
            if member[k] {
                if run.is_empty() {
                    run_start = k;
                }
                run.push(item);
                if k + 1 < n && member[k + 1] {
                    continue;
                }
                let ctx = RunCtx {
                    line_start: if run_start == 0 {
                        place.line_start
                    } else {
                        breaks[run_start - 1]
                    },
                    in_link: place.in_link,
                    cell: place.cell,
                    conservative: self.conservative,
                    before: if run_start == 0 {
                        place.delim
                    } else {
                        edge[run_start - 1]
                    },
                    after: if k + 1 == n { place.delim } else { edge[k + 1] },
                };
                let pieces: Vec<Piece<'_>> = run
                    .iter()
                    .map(|item| match item {
                        Inline::Id { id, .. } => Piece::Id(id),
                        Inline::Text(t) => Piece::Text(t),
                        _ => Piece::Text(""),
                    })
                    .collect();
                for segment in escape::run(&pieces, &ctx) {
                    parent.append(self.segment(segment));
                }
                run.clear();
                continue;
            }
            match item {
                Inline::Text(_) | Inline::Id { .. } => {}
                Inline::Code(literal) => parent.append(self.node(NodeValue::Code(NodeCode {
                    num_backticks: 1,
                    literal,
                }))),
                Inline::Math {
                    source,
                    display,
                    raw,
                } => {
                    let next_digit = k + 1 < n && digit[k + 1];
                    let dollar = dollar_math(&source) && !next_digit;
                    let value = if display {
                        if display_math(&source) {
                            NodeValue::Math(NodeMath {
                                dollar_math: true,
                                display_math: true,
                                literal: source,
                            })
                        } else {
                            NodeValue::HtmlInline(raw)
                        }
                    } else if dollar {
                        NodeValue::Math(NodeMath {
                            dollar_math: true,
                            display_math: false,
                            literal: source,
                        })
                    } else if code_math(&source) {
                        NodeValue::Math(NodeMath {
                            dollar_math: false,
                            display_math: false,
                            literal: code_math_literal(source),
                        })
                    } else {
                        NodeValue::HtmlInline(raw)
                    };
                    parent.append(self.node(value));
                }
                Inline::Html(html) => parent.append(self.node(NodeValue::HtmlInline(html))),
                Inline::Image { url, title, alt } => {
                    let image = self.node(NodeValue::Image(Box::new(NodeLink { url, title })));
                    let ctx = RunCtx {
                        in_link: true,
                        cell: place.cell,
                        conservative: self.conservative,
                        ..RunCtx::default()
                    };
                    for segment in escape::run(&[Piece::Text(&alt)], &ctx) {
                        image.append(self.segment(segment));
                    }
                    parent.append(image);
                }
                Inline::Break => parent.append(self.node(NodeValue::LineBreak)),
                Inline::Wrap(kind, children) => {
                    let link = matches!(kind, Kind::Link { .. });
                    let delim = (!link).then(|| kind.delimiter());
                    // The URL of a link comrak's writer could write as an autolink that does
                    // not read back.
                    let autolink = match &kind {
                        Kind::Link { url, .. } if !autolink_safe(url) => Some(url.clone()),
                        _ => None,
                    };
                    let value = match kind {
                        Kind::Emph => NodeValue::Emph,
                        Kind::Strong => NodeValue::Strong,
                        Kind::Strike => NodeValue::Strikethrough,
                        Kind::Insert => NodeValue::Insert,
                        Kind::Highlight => NodeValue::Highlight,
                        Kind::Sup => NodeValue::Superscript,
                        Kind::Sub => NodeValue::Subscript,
                        Kind::Link { url, title } => {
                            NodeValue::Link(Box::new(NodeLink { url, title }))
                        }
                    };
                    let node = self.node(value);
                    let inner = Place {
                        in_link: place.in_link || link,
                        cell: place.cell,
                        line_start: false,
                        delim,
                    };
                    self.convert(children, node, inner);
                    if let Some(url) = autolink {
                        self.split_autolink_text(node, &url);
                    }
                    parent.append(node);
                }
            }
        }
    }

    /// A link whose text is one text node equal to its URL is written as an autolink: the text
    /// split in two nodes makes comrak's writer write `[text](URL)` instead.
    fn split_autolink_text(&self, link: Node<'a>, url: &str) {
        let Some(child) = link.first_child() else {
            return;
        };
        if child.next_sibling().is_some() {
            return;
        }
        let rest = {
            let mut data = child.data.borrow_mut();
            let NodeValue::Text(text) = &mut data.value else {
                return;
            };
            let Some(first) = text.chars().next() else {
                return;
            };
            if text.as_ref() != url.strip_prefix("mailto:").unwrap_or(url) {
                return;
            }
            let rest = text[first.len_utf8()..].to_owned();
            *text = first.to_string().into();
            rest
        };
        if !rest.is_empty() {
            child.insert_after(self.node(NodeValue::Text(rest.into())));
        }
    }

    fn segment(&self, segment: Segment) -> Node<'a> {
        self.node(match segment {
            Segment::Text(t) => NodeValue::Text(t.into()),
            Segment::Raw(r) => NodeValue::Raw(r),
        })
    }
}

/// An alert title as written after `[!TYPE]`: comrak's writer writes it as it is, its parser
/// decodes character references and then backslash escapes. So a `&` that could start a
/// reference is written `&amp;`, a backslash before ASCII punctuation `\\`.
fn alert_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut chars = title.chars().peekable();
    while let Some(c) = chars.next() {
        let next = chars.peek().copied();
        match c {
            '&' if next.is_some_and(|n| n == '#' || n.is_ascii_alphanumeric()) => {
                out.push_str("&amp;");
            }
            '\\' if next.is_some_and(|n| n.is_ascii_punctuation()) => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out
}

/// The title [`alert_title`] wrote, as comrak's parser reads it back.
pub(super) fn alert_title_read(written: &str) -> String {
    let mut out = String::with_capacity(written.len());
    let mut rest = written;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("&amp;")
            && after
                .chars()
                .next()
                .is_some_and(|n| n == '#' || n.is_ascii_alphanumeric())
        {
            out.push('&');
            rest = after;
        } else if let Some(after) = rest.strip_prefix("\\\\")
            && after
                .chars()
                .next()
                .is_some_and(|n| n.is_ascii_punctuation())
        {
            out.push('\\');
            rest = after;
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// Whether comrak's writer can write a code span with this literal: its backtick fence (the
/// shortest run of backticks the literal lacks) is at most 80 long, the longest comrak's parser
/// reads.
fn code_span_writable(literal: &str) -> bool {
    const MAX_BACKTICKS: usize = 80;
    let mut present = [false; MAX_BACKTICKS + 1];
    let mut run = 0usize;
    for b in literal.bytes().chain(std::iter::once(b' ')) {
        if b == b'`' {
            run += 1;
        } else {
            if run > 0 && run <= MAX_BACKTICKS {
                present[run] = true;
            }
            run = 0;
        }
    }
    present[1..].iter().any(|&p| !p)
}

/// Whether a URL reads back from an autolink (`<URL>`): no spaces, controls or angle brackets.
fn autolink_safe(url: &str) -> bool {
    !url.chars()
        .any(|c| c.is_ascii_control() || matches!(c, ' ' | '<' | '>'))
}

/// The text of inline content (for comparing a link's label with its target).
fn plain_text(items: &[Inline]) -> String {
    let mut out = String::new();
    for item in items {
        match item {
            Inline::Text(t) | Inline::Code(t) => out.push_str(t),
            Inline::Id { id, .. } => out.push_str(id),
            Inline::Wrap(_, children) => out.push_str(&plain_text(children)),
            Inline::Break => out.push(' '),
            _ => out.push('\u{fffc}'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_forms() {
        for s in ["x", "\\sqrt{x}", "a\\$b", "a b"] {
            assert!(dollar_math(s), "{s}");
        }
        for s in ["", " x", "x ", "a$b", "$x", "`x", "x\\", "a\nb"] {
            assert!(!dollar_math(s), "{s:?}");
        }
        for s in ["x", " x", " x ", "a$b", "`x`"] {
            assert!(code_math(s), "{s}");
        }
        for s in ["", "a`$b", "a\nb"] {
            assert!(!code_math(s), "{s:?}");
        }
        assert_eq!(code_math_literal(" x ".into()), "  x  ");
        assert_eq!(code_math_literal(" x".into()), " x");
        assert_eq!(code_math_literal("  ".into()), "  ");
        assert!(code_math(" "));
        for s in ["x", "\\int_{0}^{1}", "a$b", " x "] {
            assert!(display_math(s), "{s}");
        }
        for s in ["", "a$$b", "$x", "x$", "a\nb"] {
            assert!(!display_math(s), "{s:?}");
        }
    }

    #[test]
    fn html_forms_are_the_forward_ones() {
        assert_eq!(
            Kind::Strong.html().0,
            format!("<span style=\"{}\">", styles::BOLD)
        );
        assert_eq!(
            Kind::Link {
                url: "u\"".into(),
                title: String::new()
            }
            .html()
            .0,
            format!(
                "<a href=\"{}\" target=\"_top\" class=\"descriptionLink\">",
                crate::html::escape_attr("u\"")
            )
        );
        assert_eq!(
            work_item_span("PROJ-1"),
            r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-option-id="long"></span>"#
        );
    }
}
