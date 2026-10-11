//! The self-check: the Markdown of each top-level block is parsed again and converted back to
//! HTML (highlighting off); that HTML, with the separators the kinds of its blocks give, must be
//! the one of the built block.
//!
//! A block that differs is built conservatively (see `build`); if that still differs, keep mode
//! writes the intended HTML as a raw HTML block (lossless) and drop mode keeps the conservative
//! version. Blocks are checked in chunks, one parse per chunk; a chunk that differs is checked
//! block by block, so the whole check takes linear time. A final check of the whole document
//! backs this up.

use std::panic::{AssertUnwindSafe, catch_unwind};

use comrak::Arena;
use comrak::nodes::NodeValue;
use comrak::options::Plugins;

use super::Report;
use super::build::{self, Builder, Node};
use super::css::Wrapper;
use super::lines::LBlock;
use super::walk::LinkInfo;
use crate::styles::{self, BlockKind};
use crate::{RawHtml, ToHtmlOptions, ToMarkdownOptions, emit, html, parse};

/// Top-level blocks checked with one parse.
const CHUNK: usize = 64;

/// The forward direction's top-level blocks: start kind, end kind, HTML.
type Blocks = Vec<(BlockKind, BlockKind, String)>;

/// The Markdown of a tree; `None` if comrak's writer fails (it never should for built trees).
fn format<'a>(root: Node<'a>) -> Option<String> {
    catch_unwind(AssertUnwindSafe(|| {
        let mut out = String::new();
        comrak::format_commonmark_with_plugins(
            root,
            &parse::options(),
            &mut out,
            &Plugins::default(),
        )
        .ok()
        .map(|()| out)
    }))
    .ok()
    .flatten()
}

/// The forward direction's blocks of some Markdown.
fn reparsed(markdown: &str, forward: &ToHtmlOptions) -> Option<Blocks> {
    catch_unwind(AssertUnwindSafe(|| {
        let arena = Arena::new();
        let root = parse::parse(&arena, markdown);
        emit::top_blocks(root, forward)
    }))
    .ok()
}

/// The forward direction's blocks of a built tree as its Markdown reads back: code math whose
/// literal `build` gave an extra space at each edge is read without them, alert titles with
/// their escapes decoded.
fn built_blocks<'a>(doc: Node<'a>, forward: &ToHtmlOptions) -> Blocks {
    let titles: Vec<(Node<'a>, String)> = doc
        .descendants()
        .filter_map(|node| match &node.data.borrow().value {
            NodeValue::Alert(alert) => alert.title.clone().map(|title| (node, title)),
            _ => None,
        })
        .collect();
    for (node, title) in &titles {
        if let NodeValue::Alert(alert) = &mut node.data.borrow_mut().value {
            alert.title = Some(build::alert_title_read(title));
        }
    }
    let blocks = padded_blocks(doc, forward);
    for (node, title) in titles {
        if let NodeValue::Alert(alert) = &mut node.data.borrow_mut().value {
            alert.title = Some(title);
        }
    }
    blocks
}

/// [`built_blocks`] for code math.
fn padded_blocks<'a>(doc: Node<'a>, forward: &ToHtmlOptions) -> Blocks {
    let padded: Vec<Node<'a>> = doc
        .descendants()
        .filter(|node| {
            matches!(&node.data.borrow().value, NodeValue::Math(math) if build::padded_code_math(math))
        })
        .collect();
    let set = |strip: bool| {
        for node in &padded {
            if let NodeValue::Math(math) = &mut node.data.borrow_mut().value {
                math.literal = if strip {
                    math.literal[1..math.literal.len() - 1].to_owned()
                } else {
                    format!(" {} ", math.literal)
                };
            }
        }
    };
    set(true);
    let blocks = emit::top_blocks(doc, forward);
    set(false);
    blocks
}

/// The HTML of top-level blocks joined as the forward direction joins them.
fn joined(blocks: &Blocks) -> String {
    let mut out = String::new();
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            out.push_str(styles::separator(blocks[i - 1].1, block.0, false));
        }
        out.push_str(&block.2);
    }
    out
}

/// Whether `got` writes the HTML of `expected`: the same joined HTML, starting with the same kind
/// and with the same separator before the block that follows (`next`; nothing at the end).
fn same(expected: &Blocks, got: &Blocks, next: Option<BlockKind>) -> bool {
    let start = |blocks: &Blocks| blocks.first().map(|b| b.0);
    let end = |blocks: &Blocks| {
        next.and_then(|next| {
            blocks
                .last()
                .map(|b| styles::separator_breaks(b.1, next, false))
        })
    };
    start(expected) == start(got) && end(expected) == end(got) && joined(expected) == joined(got)
}

/// The kind a top-level block starts with in the HTML the forward direction writes for it
/// (without diagram attachments: a diagram is a code block).
fn start_kind(block: &LBlock) -> BlockKind {
    match block {
        LBlock::Para(_) | LBlock::Heading(..) | LBlock::MathBlock(_) => BlockKind::Text,
        LBlock::Raw(html) => html::raw_block_kinds(html).0,
        _ => BlockKind::Box,
    }
}

struct Checker<'a, 'o> {
    arena: &'a Arena<'a>,
    normal: Builder<'a, 'o>,
    conservative: Builder<'a, 'o>,
    forward: ToHtmlOptions,
    keep: bool,
    fallbacks: usize,
    invalid: usize,
}

impl<'a> Checker<'a, '_> {
    fn document(&self) -> Node<'a> {
        self.arena.alloc(NodeValue::Document.into())
    }

    /// A temporary document holding `nodes`; with `count`, the invalid ones are counted.
    fn holding(&mut self, nodes: &[Node<'a>], count: bool) -> Node<'a> {
        let doc = self.document();
        for &node in nodes {
            doc.append(node);
            if count && node.validate().is_err() {
                self.invalid += 1;
            }
        }
        doc
    }

    /// Whether the Markdown of `doc` reads back as `expected` (followed by a block starting with
    /// `next`).
    fn reads_back(&self, doc: Node<'a>, expected: &Blocks, next: Option<BlockKind>) -> bool {
        format(doc)
            .and_then(|markdown| reparsed(&markdown, &self.forward))
            .is_some_and(|got| same(expected, &got, next))
    }

    /// Checks one top-level block (its nodes) and appends what passes to `out`; `next` is the
    /// kind the following block starts with.
    fn block(
        &mut self,
        block: &LBlock,
        nodes: Vec<Node<'a>>,
        out: Node<'a>,
        next: Option<BlockKind>,
    ) {
        let doc = self.holding(&nodes, false);
        let expected = built_blocks(doc, &self.forward);
        if self.reads_back(doc, &expected, next) {
            out.extend(nodes);
            return;
        }
        self.fallbacks += 1;
        #[cfg(test)]
        diagnostics::record(|| {
            let markdown = format(doc).unwrap_or_default();
            let want = joined(&expected);
            let got = reparsed(&markdown, &self.forward).map(|got| joined(&got));
            let got = got.unwrap_or_default();
            let at = want
                .bytes()
                .zip(got.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(want.len().min(got.len()));
            let window = |s: &str| {
                let mut start = at.saturating_sub(150);
                while !s.is_char_boundary(start) {
                    start -= 1;
                }
                let mut end = (at + 150).min(s.len());
                while !s.is_char_boundary(end) {
                    end += 1;
                }
                s[start..end].to_owned()
            };
            format!(
                "markdown: {markdown:?}\nexpected: …{:?}…\ngot:      …{:?}…",
                window(&want),
                window(&got)
            )
        });
        let mut conservative = Vec::new();
        self.conservative.block(block, &mut conservative);
        let doc = self.holding(&conservative, true);
        if self.reads_back(doc, &expected, next) || !self.keep {
            out.extend(conservative);
            return;
        }
        // The intended HTML as it is: one HTML block per block, on one line; inline content
        // after a lone tag line (else it would be read as a paragraph).
        for (first, _, html) in &expected {
            let flat = html.replace('\n', "&#10;");
            let literal = if *first == BlockKind::Text {
                format!("<span>\n{flat}\n</span>")
            } else {
                flat
            };
            let raw = self.normal.html_block(literal);
            out.append(raw);
            if raw.validate().is_err() {
                self.invalid += 1;
            }
        }
    }
}

/// The top-level blocks, raw wrappers unfolded (their tags and blocks are siblings).
fn unfold(blocks: Vec<LBlock>, out: &mut Vec<LBlock>) {
    for block in blocks {
        match block {
            LBlock::RawWrap(open, inner) => {
                out.push(LBlock::Raw(open));
                unfold(inner, out);
                out.push(LBlock::Raw("</div>".to_owned()));
            }
            other => out.push(other),
        }
    }
}

/// Builds, checks and writes the document.
pub(super) fn render(
    blocks: Vec<LBlock>,
    links: &[LinkInfo],
    wrappers: &[Wrapper],
    options: &ToMarkdownOptions,
) -> Report {
    let mut units = Vec::with_capacity(blocks.len());
    unfold(blocks, &mut units);
    let arena = Arena::new();
    let builder = |conservative| Builder {
        arena: &arena,
        options,
        links,
        wrappers,
        conservative,
    };
    let mut checker = Checker {
        arena: &arena,
        normal: builder(false),
        conservative: builder(true),
        forward: ToHtmlOptions {
            work_items: options.work_items.clone(),
            highlight: false,
        },
        keep: options.raw_html == RawHtml::Keep,
        fallbacks: 0,
        invalid: 0,
    };
    let document = checker.document();
    for (c, chunk) in units.chunks(CHUNK).enumerate() {
        let next_of = |k: usize| units.get(c * CHUNK + k + 1).map(start_kind);
        let nodes: Vec<Vec<Node<'_>>> = chunk
            .iter()
            .map(|block| {
                let mut nodes = Vec::new();
                checker.normal.block(block, &mut nodes);
                nodes
            })
            .collect();
        let all: Vec<Node<'_>> = nodes.iter().flatten().copied().collect();
        let doc = checker.holding(&all, true);
        let expected = built_blocks(doc, &checker.forward);
        if checker.reads_back(doc, &expected, next_of(chunk.len() - 1)) {
            document.extend(all);
            continue;
        }
        for (k, (block, nodes)) in chunk.iter().zip(nodes).enumerate() {
            checker.block(block, nodes, document, next_of(k));
        }
    }
    if document.validate().is_err() {
        checker.invalid += 1;
    }
    let markdown = format(document).unwrap_or_default();
    let expected = built_blocks(document, &checker.forward);
    if reparsed(&markdown, &checker.forward).is_none_or(|got| !same(&expected, &got, None)) {
        checker.fallbacks += 1;
    }
    let markdown = tidy_quoted_fences(markdown, &expected, &checker.forward);
    let markdown = tidy_empty_headings(markdown, &expected, &checker.forward);
    let mut markdown = markdown.trim_end_matches('\n');
    // An empty heading at the end: its marker without the space comrak writes after it.
    let last_line = markdown.rsplit('\n').next().unwrap_or_default();
    if let Some(marker) = last_line.strip_suffix(' ')
        && (1..=6).contains(&marker.len())
        && marker.bytes().all(|b| b == b'#')
    {
        markdown = &markdown[..markdown.len() - 1];
    }
    Report {
        markdown: markdown.to_owned(),
        fallbacks: checker.fallbacks,
        invalid: checker.invalid,
    }
}

/// Whether a line is only quote markers after an indentation (`> `, `> > `, `  > `, ...).
fn quote_prefix(line: &str) -> bool {
    line.ends_with("> ") && line.bytes().all(|b| b == b'>' || b == b' ')
}

/// comrak's writer starts a quote whose first block is a code block with two lines of only the
/// quote markers before the fence (`> `, `> `, `` > ``` ``; in a list item `- > `, `  > `,
/// `` > ``` ``): the fence goes on the first of them, when the Markdown reads back the same.
fn tidy_quoted_fences(markdown: String, expected: &Blocks, forward: &ToHtmlOptions) -> String {
    let lines: Vec<&str> = markdown.split('\n').collect();
    let mut tidy: Vec<String> = Vec::with_capacity(lines.len());
    let mut changed = false;
    let mut i = 0;
    while i < lines.len() {
        if i + 2 < lines.len() {
            let (first, second, third) = (lines[i], lines[i + 1], lines[i + 2]);
            let markers = second.trim_start_matches(' ');
            let fence = third
                .strip_prefix(second)
                .filter(|rest| rest.starts_with("```") || rest.starts_with("~~~"));
            if let Some(fence) = fence
                && quote_prefix(second)
                && first.len() == second.len()
                && first.ends_with(markers)
            {
                tidy.push(format!("{first}{fence}"));
                changed = true;
                i += 3;
                continue;
            }
        }
        tidy.push(lines[i].to_owned());
        i += 1;
    }
    if !changed {
        return markdown;
    }
    let tidy = tidy.join("\n");
    match reparsed(&tidy, forward) {
        Some(got) if same(expected, &got, None) => tidy,
        _ => markdown,
    }
}

/// Whether a line is an empty heading as comrak's writer writes it: its marker (after quote or
/// list prefixes) and a space.
fn empty_heading_line(line: &str) -> bool {
    let Some(marker) = line.strip_suffix(' ') else {
        return false;
    };
    let hashes = marker.len() - marker.trim_end_matches('#').len();
    let before = &marker[..marker.len() - hashes];
    (1..=6).contains(&hashes) && (before.is_empty() || before.ends_with([' ', '>']))
}

/// comrak's writer writes an empty heading as its marker and a space (`# `): the space goes, when
/// the Markdown reads back the same.
fn tidy_empty_headings(markdown: String, expected: &Blocks, forward: &ToHtmlOptions) -> String {
    if !markdown.split('\n').any(empty_heading_line) {
        return markdown;
    }
    let tidy: Vec<&str> = markdown
        .split('\n')
        .map(|line| {
            if empty_heading_line(line) {
                &line[..line.len() - 1]
            } else {
                line
            }
        })
        .collect();
    let tidy = tidy.join("\n");
    match reparsed(&tidy, forward) {
        Some(got) if same(expected, &got, None) => tidy,
        _ => markdown,
    }
}

/// What the self-check rejected, for tests that look into it.
#[cfg(test)]
pub(super) mod diagnostics {
    use std::cell::RefCell;

    thread_local! {
        static REJECTED: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    }

    /// Starts collecting the blocks the self-check rejects on this thread.
    pub(crate) fn collect() {
        REJECTED.with(|r| *r.borrow_mut() = Some(Vec::new()));
    }

    /// The blocks rejected since [`collect`].
    pub(crate) fn take() -> Vec<String> {
        REJECTED.with(|r| r.borrow_mut().replace(Vec::new()).unwrap_or_default())
    }

    pub(super) fn record(entry: impl FnOnce() -> String) {
        REJECTED.with(|r| {
            if let Some(entries) = r.borrow_mut().as_mut() {
                entries.push(entry());
            }
        });
    }
}
