//! Markdown to Polarion HTML: an emitter over comrak's AST (comrak's HTML renderer is not used).
//!
//! Blocks are not wrapped in `<p>`: two blocks of one container are separated by `<br/>`s
//! ([`styles::separator`]). Nesting deeper than [`MAX_DEPTH`] is written as escaped text. The
//! HTML contains no character XML does not allow ([`html::xml_safe`]): Polarion receives it in a
//! SOAP message.

mod inline;

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write as _;

use comrak::Arena;
use comrak::nodes::{
    AstNode, ListType, NodeCodeBlock, NodeList, NodeTable, NodeValue, TableAlignment,
};

use crate::html::{self, push_attr, push_text};
use crate::styles::{self, Alert, Align, BlockKind};
use crate::{DiagramAttachments, ToHtmlOptions, highlight, mermaid, parse};

use inline::Ctx;

/// Nesting (blocks and inlines together) deeper than this is written as escaped text.
pub(crate) const MAX_DEPTH: usize = 64;

pub(crate) fn to_html(
    markdown: &str,
    options: &ToHtmlOptions,
    diagrams: &DiagramAttachments,
) -> String {
    let arena = Arena::new();
    let root = parse::parse(&arena, markdown);
    emit_document(root, options, diagrams)
}

/// The HTML of a parsed or hand-built tree: the children of `root` as blocks.
///
/// In hand-built trees (the reverse direction's self-check) a `Raw` node of a backslash and one
/// ASCII punctuation character (`\-`) is that character escaped, any other `Raw` is literal
/// text (`\=\=` too: one escape per node); both take part in work item id detection like `Text`
/// and `Escaped` nodes. Inline nodes directly in a block container are one paragraph; node kinds
/// this crate does not enable, and nodes where comrak would not put them, contribute their
/// content. Any tree is accepted.
///
/// Work item ids are not linked while a raw `<a>` is open, in the order the HTML is written and
/// across blocks, as a browser keeps it open: until a raw `</a>` or a Markdown link (which closes
/// it), or the end of the table cell it was opened in.
pub(crate) fn emit_document<'a>(
    root: &'a AstNode<'a>,
    options: &ToHtmlOptions,
    diagrams: &DiagramAttachments,
) -> String {
    emit_with_budget(root, options, diagrams, highlight::Budget::per_document())
}

/// [`emit_document`] with this highlighting budget.
fn emit_with_budget<'a>(
    root: &'a AstNode<'a>,
    options: &ToHtmlOptions,
    diagrams: &DiagramAttachments,
    highlight_budget: highlight::Budget,
) -> String {
    let mut emitter = Emitter {
        options,
        diagrams,
        root,
        footnotes: None,
        highlight_budget,
        raw_link: false,
        reached: Vec::new(),
    };
    let blocks = emitter.blocks(root, 0);
    let out = join(&blocks, false);
    // Raw HTML and highlighted code too.
    match html::xml_safe(&out) {
        Cow::Borrowed(_) => out,
        Cow::Owned(safe) => safe,
    }
}

/// The top-level blocks [`emit_document`] writes for a tree, each with the kinds it starts and
/// ends with (which decide the separators around it), without diagram attachments. The reverse
/// direction's self-check compares them for the tree it built and for its Markdown parsed again.
pub(crate) fn top_blocks<'a>(
    root: &'a AstNode<'a>,
    options: &ToHtmlOptions,
) -> Vec<(BlockKind, BlockKind, String)> {
    let diagrams = DiagramAttachments::new();
    let mut emitter = Emitter {
        options,
        diagrams: &diagrams,
        root,
        footnotes: None,
        highlight_budget: highlight::Budget::per_document(),
        raw_link: false,
        reached: Vec::new(),
    };
    emitter
        .blocks(root, 0)
        .into_iter()
        .map(|block| (block.first, block.last, block.html))
        .collect()
}

/// The mermaid blocks [`emit_document`] reaches, in document order (repeats and empty blocks
/// included): the ones it writes as images when their attachments are known. Blocks nested deeper
/// than [`MAX_DEPTH`] are not among them, since they are written as text.
pub(crate) fn reached_diagrams<'a>(root: &'a AstNode<'a>) -> Vec<mermaid::Diagram> {
    let options = ToHtmlOptions {
        work_items: crate::WorkItemIds::Off,
        highlight: false,
    };
    let diagrams = DiagramAttachments::new();
    let mut emitter = Emitter {
        options: &options,
        diagrams: &diagrams,
        root,
        footnotes: None,
        highlight_budget: highlight::Budget::per_document(),
        raw_link: false,
        reached: Vec::new(),
    };
    emitter.blocks(root, 0);
    emitter.reached
}

/// Line breaks as `\n`: comrak keeps `\r\n` and `\r` in code blocks and display formulas.
fn unix_lines(s: &str) -> Cow<'_, str> {
    if s.contains('\r') {
        Cow::Owned(s.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(s)
    }
}

/// A code span or inline formula without carriage returns: comrak turns their line breaks into
/// spaces, but of a `\r\n` it keeps the `\r`.
fn without_cr(s: &str) -> Cow<'_, str> {
    if s.contains('\r') {
        Cow::Owned(s.replace('\r', ""))
    } else {
        Cow::Borrowed(s)
    }
}

/// A block's HTML and its kinds for the separators.
#[derive(Debug)]
struct Block {
    first: BlockKind,
    last: BlockKind,
    html: String,
}

impl Block {
    fn text(html: String) -> Block {
        Block {
            first: BlockKind::Text,
            last: BlockKind::Text,
            html,
        }
    }

    fn boxed(html: String) -> Block {
        Block {
            first: BlockKind::Box,
            last: BlockKind::Box,
            html,
        }
    }
}

/// Blocks joined by their separators.
fn join(blocks: &[Block], in_list_item: bool) -> String {
    let mut out = String::with_capacity(blocks.iter().map(|b| b.html.len() + 10).sum());
    let mut prev: Option<BlockKind> = None;
    for block in blocks {
        if let Some(prev) = prev {
            out.push_str(styles::separator(prev, block.first, in_list_item));
        }
        out.push_str(&block.html);
        prev = Some(block.last);
    }
    out
}

struct Emitter<'a, 'o> {
    options: &'o ToHtmlOptions,
    diagrams: &'o DiagramAttachments,
    root: &'a AstNode<'a>,
    /// Footnote numbers by name (from the references), collected at the first definition.
    footnotes: Option<HashMap<String, u32>>,
    /// What highlighting may still spend in this document.
    highlight_budget: highlight::Budget,
    /// A raw `<a>` is open where the HTML written so far ends: ids are not linked.
    raw_link: bool,
    /// The mermaid blocks reached so far ([`reached_diagrams`]).
    reached: Vec<mermaid::Diagram>,
}

/// Whether a node is inline content (`Raw` included).
fn is_inline(value: &NodeValue) -> bool {
    !value.block() && !matches!(value, NodeValue::FrontMatter(_))
}

impl<'a> Emitter<'a, '_> {
    /// The blocks of a container at `depth` (its children are one deeper).
    fn blocks(&mut self, container: &'a AstNode<'a>, depth: usize) -> Vec<Block> {
        let children: Vec<&'a AstNode<'a>> = container.children().collect();
        let mut out = Vec::with_capacity(children.len());
        let mut footnote_rule = false;
        let mut i = 0;
        while i < children.len() {
            if is_inline(&children[i].data.borrow().value) {
                let end = (i..children.len())
                    .find(|&j| !is_inline(&children[j].data.borrow().value))
                    .unwrap_or(children.len());
                let mut html = String::new();
                self.inline_seq(&children[i..end], depth + 1, Ctx::default(), &mut html);
                if !html.is_empty() {
                    out.push(Block::text(html));
                }
                i = end;
                continue;
            }
            self.block(children[i], depth + 1, &mut out, &mut footnote_rule);
            i += 1;
        }
        out
    }

    /// The blocks of a container joined.
    fn joined(&mut self, container: &'a AstNode<'a>, depth: usize, in_list_item: bool) -> String {
        join(&self.blocks(container, depth), in_list_item)
    }

    /// Appends the block(s) of `node` (several for flattened containers: footnotes, description lists).
    fn block(
        &mut self,
        node: &'a AstNode<'a>,
        depth: usize,
        out: &mut Vec<Block>,
        footnote_rule: &mut bool,
    ) {
        if depth > MAX_DEPTH {
            let text = inline::plain_text(node);
            if !text.is_empty() {
                out.push(Block::text(html::escape_text(&text).into_owned()));
            }
            return;
        }
        // Shared borrows: the emitter only reads the tree.
        let ast = node.data.borrow();
        match &ast.value {
            NodeValue::Paragraph => {
                let html = self.inlines(node, depth, Ctx::default());
                if !html.is_empty() {
                    out.push(Block::text(html));
                }
            }
            NodeValue::Heading(heading) => {
                let inner = self.inlines(
                    node,
                    depth,
                    Ctx {
                        heading: true,
                        ..Ctx::default()
                    },
                );
                let style = styles::heading(heading.level);
                out.push(Block::text(format!(
                    "<span style=\"{style}\">{inner}</span>"
                )));
            }
            NodeValue::ThematicBreak => out.push(Block::boxed("<hr/>".to_owned())),
            NodeValue::BlockQuote | NodeValue::MultilineBlockQuote(_) => {
                let inner = self.joined(node, depth, false);
                out.push(Block::boxed(format!(
                    "<div style=\"{}\">{inner}</div>",
                    styles::QUOTE
                )));
            }
            NodeValue::Alert(alert) => {
                let html = self.alert(
                    Alert::of(alert.alert_type),
                    alert.title.as_deref(),
                    node,
                    depth,
                );
                out.push(Block::boxed(html));
            }
            NodeValue::BlockDirective(directive) => {
                // Words split as in code block info strings (`mermaid::is_mermaid_info`).
                let info = directive.info.trim();
                let (name, title) = info.split_once(char::is_whitespace).unwrap_or((info, ""));
                let html = match Alert::named(name) {
                    Some(alert) => self.alert(alert, Some(title.trim()), node, depth),
                    None => format!("<div>{}</div>", self.joined(node, depth, false)),
                };
                out.push(Block::boxed(html));
            }
            NodeValue::CodeBlock(code) => out.push(self.code_block(code)),
            NodeValue::HtmlBlock(raw) => {
                if let Some(block) = raw_block(&raw.literal) {
                    self.raw_link = html::link_open_after(&block.html, self.raw_link);
                    out.push(block);
                }
            }
            NodeValue::List(list) => out.push(Block::boxed(self.list(node, list, depth))),
            NodeValue::Table(table) => out.push(Block::boxed(self.table(node, table, depth))),
            NodeValue::FootnoteDefinition(definition) => {
                if !*footnote_rule {
                    out.push(Block::boxed("<hr/>".to_owned()));
                    *footnote_rule = true;
                }
                let mark = format!("<sup>[{}]</sup>", self.footnote_number(&definition.name));
                let mut blocks = self.blocks(node, depth);
                match blocks.first_mut() {
                    Some(first) => {
                        first.html.insert_str(0, &format!("{mark} "));
                        first.first = BlockKind::Text;
                    }
                    None => blocks.push(Block::text(mark)),
                }
                out.extend(blocks);
            }
            NodeValue::DescriptionList => self.description_list(node, depth, out),
            NodeValue::FrontMatter(_) => {}
            // Items outside a list, nested documents, description parts outside their list,
            // extensions this crate does not enable: their content.
            _ => out.extend(self.blocks(node, depth)),
        }
    }

    fn alert(
        &mut self,
        alert: Alert,
        title: Option<&str>,
        node: &'a AstNode<'a>,
        depth: usize,
    ) -> String {
        let mut out = format!(
            "<div style=\"{}\"><span style=\"{}\">",
            alert.div_style(),
            alert.title_style()
        );
        push_text(
            &mut out,
            title
                .filter(|t| !t.trim().is_empty())
                .unwrap_or(alert.title),
        );
        out.push_str("</span>");
        out.push_str(styles::BR);
        out.push_str(&self.joined(node, depth, false));
        out.push_str("</div>");
        out
    }

    /// A code block: a `<pre>`; a diagram with an attachment its image; a `math` block a display
    /// formula. The language is the first word of the info string, as written (words end at any
    /// whitespace, as for [`mermaid::is_mermaid_info`]).
    fn code_block(&mut self, code: &NodeCodeBlock) -> Block {
        let lang = code.info.split_whitespace().next().unwrap_or("");
        let mermaid = mermaid::is_mermaid_info(&code.info);
        if mermaid {
            let diagram = mermaid::diagram(&code.literal);
            let image = self.diagrams.get(&diagram.file_name).map(|id| {
                let mut out = format!("<img src=\"{}", styles::ATTACHMENT_SCHEME);
                push_attr(&mut out, &html::percent_encode(id));
                out.push_str("\" title=\"");
                push_attr(&mut out, &diagram.file_name);
                let _ = write!(out, "\" style=\"{}\"/>", styles::IMG);
                out
            });
            self.reached.push(diagram);
            if let Some(image) = image {
                return Block::text(image);
            }
        }
        let literal = unix_lines(&code.literal);
        let content = literal.strip_suffix('\n').unwrap_or(&literal);
        if lang.eq_ignore_ascii_case("math") {
            let mut out = String::new();
            inline::formula(&mut out, content, true);
            return Block::text(out);
        }
        let highlighted = if self.options.highlight && !lang.is_empty() && !mermaid {
            highlight::highlight(lang, content, &mut self.highlight_budget)
        } else {
            None
        };
        let mut out = String::from("<pre");
        if !lang.is_empty() {
            out.push_str(" class=\"");
            push_attr(&mut out, styles::LANGUAGE_CLASS_PREFIX);
            push_attr(&mut out, lang);
            out.push('"');
        }
        let _ = write!(out, " style=\"{}\">", styles::PRE);
        // HTML parsers drop one line break right after `<pre>`.
        if content.starts_with('\n') {
            out.push('\n');
        }
        match highlighted {
            Some(spans) => out.push_str(&spans),
            None => push_text(&mut out, content),
        }
        out.push_str("</pre>");
        Block::boxed(out)
    }

    fn list(&mut self, node: &'a AstNode<'a>, list: &NodeList, depth: usize) -> String {
        let mut out = String::new();
        let close = match list.list_type {
            ListType::Bullet => {
                out.push_str("<ul>");
                "</ul>"
            }
            ListType::Ordered => {
                match list.start {
                    1 => out.push_str("<ol>"),
                    start => {
                        let _ = write!(out, "<ol start=\"{start}\">");
                    }
                }
                "</ol>"
            }
        };
        for item in node.children() {
            out.push_str("<li>");
            if depth + 1 > MAX_DEPTH {
                push_text(&mut out, &inline::plain_text(item));
            } else {
                if let NodeValue::TaskItem(task) = &item.data.borrow().value {
                    out.push_str(if task.symbol.is_some() {
                        styles::TASK_DONE
                    } else {
                        styles::TASK_OPEN
                    });
                }
                out.push_str(&self.joined(item, depth + 1, true));
            }
            out.push_str("</li>");
        }
        out.push_str(close);
        out
    }

    fn table(&mut self, node: &'a AstNode<'a>, table: &NodeTable, depth: usize) -> String {
        let mut out = format!(
            "<table id=\"{}\" class=\"{}\" style=\"{}\"><tbody>",
            styles::TABLE_ID,
            styles::TABLE_CLASS,
            styles::TABLE
        );
        // A raw `<a>` opened before the table holds it; one opened in a cell ends with the cell.
        let raw_link = self.raw_link;
        for row in node.children() {
            let header = matches!(row.data.borrow().value, NodeValue::TableRow(true));
            out.push_str("<tr>");
            for (i, cell) in row.children().enumerate() {
                let align = Align::from(
                    table
                        .alignments
                        .get(i)
                        .copied()
                        .unwrap_or(TableAlignment::None),
                );
                let (tag, style) = if header {
                    ("th", styles::th(align))
                } else {
                    ("td", styles::td(align))
                };
                let _ = write!(out, "<{tag} style=\"{style}\">");
                self.raw_link = raw_link;
                if depth + 2 > MAX_DEPTH {
                    push_text(&mut out, &inline::plain_text(cell));
                } else {
                    out.push_str(&self.inlines(cell, depth + 2, Ctx::default()));
                }
                let _ = write!(out, "</{tag}>");
            }
            out.push_str("</tr>");
        }
        self.raw_link = raw_link;
        out.push_str("</tbody></table>");
        out
    }

    /// A term as bold text, its details as an indented div (lossy: Polarion has no `<dl>`).
    fn description_list(&mut self, node: &'a AstNode<'a>, depth: usize, out: &mut Vec<Block>) {
        let mut footnote_rule = false;
        for item in node.children() {
            for part in item.children() {
                let ast = part.data.borrow();
                match &ast.value {
                    NodeValue::DescriptionTerm => {
                        for child in part.children() {
                            let paragraph =
                                matches!(child.data.borrow().value, NodeValue::Paragraph);
                            if paragraph && depth + 3 <= MAX_DEPTH {
                                let inner = self.inlines(child, depth + 3, Ctx::default());
                                out.push(Block::text(format!(
                                    "<span style=\"{}\">{inner}</span>",
                                    styles::BOLD
                                )));
                            } else {
                                self.block(child, depth + 3, out, &mut footnote_rule);
                            }
                        }
                    }
                    NodeValue::DescriptionDetails => {
                        let inner = self.joined(part, depth + 2, false);
                        out.push(Block::boxed(format!(
                            "<div style=\"{}\">{inner}</div>",
                            styles::QUOTE
                        )));
                    }
                    _ => self.block(part, depth + 2, out, &mut footnote_rule),
                }
            }
        }
    }

    fn footnote_number(&mut self, name: &str) -> u32 {
        let root = self.root;
        let numbers = self.footnotes.get_or_insert_with(|| {
            root.descendants()
                .filter_map(|n| match &n.data.borrow().value {
                    NodeValue::FootnoteReference(r) => Some((r.name.clone(), r.ix)),
                    _ => None,
                })
                .collect()
        });
        match numbers.get(name) {
            Some(&n) => n,
            None => {
                let n = numbers
                    .values()
                    .max()
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(1);
                numbers.insert(name.to_owned(), n);
                n
            }
        }
    }
}

/// A raw HTML block without comments, sanitized; `None` if nothing is left.
fn raw_block(literal: &str) -> Option<Block> {
    let stripped = html::strip_comments(literal);
    let clean = html::sanitize_raw_html(&stripped);
    let html = clean.trim();
    if html.is_empty() {
        return None;
    }
    let (first, last) = html::raw_block_kinds(html);
    Some(Block {
        first,
        last,
        html: html.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use comrak::Arena;
    use comrak::nodes::{AstNode, NodeValue};

    use super::*;
    use crate::WorkItemIds;

    const LINK: &str = r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-12" data-option-id="long"></span>"#;

    fn node<'a>(arena: &'a Arena<'a>, value: NodeValue) -> &'a AstNode<'a> {
        arena.alloc(value.into())
    }

    fn paragraph<'a>(arena: &'a Arena<'a>, inlines: Vec<NodeValue>) -> &'a AstNode<'a> {
        let root = node(arena, NodeValue::Document);
        let p = node(arena, NodeValue::Paragraph);
        root.append(p);
        for value in inlines {
            p.append(node(arena, value));
        }
        root
    }

    fn emit<'a>(root: &'a AstNode<'a>) -> String {
        emit_document(root, &ToHtmlOptions::default(), &DiagramAttachments::new())
    }

    fn text(s: &str) -> NodeValue {
        NodeValue::Text(s.to_owned().into())
    }

    fn raw(s: &str) -> NodeValue {
        NodeValue::Raw(s.to_owned())
    }

    #[test]
    fn raw_escapes_take_part_in_id_runs() {
        let arena = Arena::new();
        // `MY\_PROJ\-12` as the reverse direction builds it: no link.
        let root = paragraph(
            &arena,
            vec![text("MY"), raw("\\_"), text("PROJ"), raw("\\-"), text("12")],
        );
        assert_eq!(emit(root), "MY_PROJ-12");
        // `MY\_PROJ-12`: one id.
        let root = paragraph(&arena, vec![text("MY"), raw("\\_"), text("PROJ-12")]);
        assert!(emit(root).contains(r#"data-item-id="MY_PROJ-12""#));
        // A linked id written as Raw.
        let root = paragraph(&arena, vec![text("see "), raw("PROJ-12"), text(".")]);
        assert_eq!(emit(root), format!("see {LINK}."));
        // `PROJ\-12` as Text + Raw + Text.
        let root = paragraph(&arena, vec![text("PROJ"), raw("\\-"), text("12")]);
        assert_eq!(emit(root), "PROJ-12");
        // A Raw of other content is literal text (escaped), never markup; one escape per node.
        let root = paragraph(&arena, vec![raw("<b>"), raw("\\a"), raw("\\")]);
        assert_eq!(emit(root), "&lt;b&gt;\\a\\");
        let root = paragraph(&arena, vec![raw("\\=\\="), raw("\\="), raw("\\=")]);
        assert_eq!(emit(root), "\\=\\===");
        // An escaped backslash.
        let root = paragraph(&arena, vec![raw("\\\\"), text("PROJ-12")]);
        assert_eq!(emit(root), "\\PROJ-12");
        // The boundary looks across nodes: `aPROJ-12` is no id.
        let root = paragraph(&arena, vec![text("a"), raw("PROJ-12")]);
        assert_eq!(emit(root), "aPROJ-12");
    }

    #[test]
    fn parsed_escapes_take_part_in_id_runs() {
        let html = to_html(
            "MY\\_PROJ-12 MY\\_PROJ\\-12 PROJ\\-12 \\(PROJ-12\\)",
            &ToHtmlOptions::default(),
            &DiagramAttachments::new(),
        );
        assert_eq!(
            html,
            format!(
                "{} MY_PROJ-12 PROJ-12 ({LINK})",
                LINK.replace("PROJ-12", "MY_PROJ-12")
            )
        );
    }

    #[test]
    fn heading_levels_are_clamped() {
        let arena = Arena::new();
        let root = node(&arena, NodeValue::Document);
        for level in [0, 7, 200] {
            let heading = node(
                &arena,
                NodeValue::Heading(comrak::nodes::NodeHeading {
                    level,
                    ..Default::default()
                }),
            );
            heading.append(node(&arena, text("h")));
            root.append(heading);
        }
        let h1 = format!("<span style=\"{}\">h</span>", styles::heading(1));
        let h6 = format!("<span style=\"{}\">h</span>", styles::heading(6));
        assert_eq!(emit(root), format!("{h1}<br/><br/>{h6}<br/><br/>{h6}"));
    }

    #[test]
    fn inline_nodes_in_block_containers_are_a_paragraph() {
        let arena = Arena::new();
        let root = node(&arena, NodeValue::Document);
        root.append(node(&arena, raw("PROJ-12")));
        root.append(node(&arena, NodeValue::ThematicBreak));
        root.append(node(&arena, text("a")));
        root.append(node(&arena, NodeValue::SoftBreak));
        root.append(node(&arena, text("b")));
        assert_eq!(emit(root), format!("{LINK}<br/><br/><hr/><br/>a<br/>b"));
    }

    #[test]
    fn empty_code_and_odd_trees_do_not_panic() {
        let arena = Arena::new();
        let root = node(&arena, NodeValue::Document);
        let p = node(&arena, NodeValue::Paragraph);
        root.append(p);
        p.append(node(
            &arena,
            NodeValue::Code(comrak::nodes::NodeCode::default()),
        ));
        p.append(node(&arena, NodeValue::Strong));
        let list = node(&arena, NodeValue::List(NodeList::default()));
        root.append(list);
        list.append(node(&arena, NodeValue::Paragraph));
        let cell = node(&arena, NodeValue::TableCell);
        root.append(cell);
        cell.append(node(&arena, text("c")));
        assert_eq!(
            emit(root),
            format!(
                "<span style=\"{}\"></span><span style=\"font-weight: bold;\"></span><br/><br/><ul><li></li></ul><br/>c",
                styles::MONO
            )
        );
    }

    /// The HTML of `markdown` with this highlighting budget.
    #[cfg(feature = "highlight")]
    fn with_budget(markdown: &str, budget: highlight::Budget) -> String {
        let arena = Arena::new();
        let root = parse::parse(&arena, markdown);
        emit_with_budget(
            root,
            &ToHtmlOptions::default(),
            &DiagramAttachments::new(),
            budget,
        )
    }

    /// Highlighting stops when the next code block would exceed the document's budget.
    #[cfg(feature = "highlight")]
    #[test]
    fn highlighting_has_a_budget_per_document() {
        let code = "let x = 1;\n".repeat(highlight::Budget::BYTES / 2 / 11);
        let block = format!("```rust\n{code}```");
        let markdown = [block.as_str(), "```rust\nlet y = 2;\n```", &block, &block].join("\n\n");
        let html = with_budget(
            &markdown,
            highlight::Budget {
                bytes: highlight::Budget::BYTES,
                time: std::time::Duration::from_secs(3600),
            },
        );
        let blocks: Vec<&str> = html.split("</pre>").collect();
        assert_eq!(blocks.len(), 5, "{html}");
        let highlighted = |b: &str| b.contains("<span style=\"");
        assert!(highlighted(blocks[0]) && highlighted(blocks[1]) && highlighted(blocks[2]));
        assert!(
            !highlighted(blocks[3]),
            "beyond the budget: {}",
            &blocks[3][..200]
        );
        // Each document has its own budget.
        let again = to_html(
            "```rust\nlet y = 2;\n```",
            &ToHtmlOptions::default(),
            &DiagramAttachments::new(),
        );
        assert!(highlighted(&again));
    }

    /// Once the document's time is up, the code blocks after it stay plain.
    #[cfg(feature = "highlight")]
    #[test]
    fn highlighting_has_a_time_limit_per_document() {
        let markdown = "```rust\nlet x = 1;\n```\n\n```rust\nlet y = 2;\n```";
        let pre = |content: &str| {
            format!(
                "<pre class=\"language-rust\" style=\"{}\">{content}</pre>",
                styles::PRE
            )
        };
        let no_time = highlight::Budget {
            bytes: highlight::Budget::BYTES,
            time: std::time::Duration::ZERO,
        };
        assert_eq!(
            with_budget(markdown, no_time),
            format!("{}<br/>{}", pre("let x = 1;"), pre("let y = 2;"))
        );
        // The block that runs out of time stays plain, and so does the rest.
        let lines = "let x = 1;\n".repeat(100);
        let markdown = format!("```rust\n{lines}```\n\n```rust\nlet y = 2;\n```");
        let html = with_budget(
            &markdown,
            highlight::Budget {
                bytes: highlight::Budget::BYTES,
                time: std::time::Duration::from_micros(1),
            },
        );
        assert_eq!(
            html,
            format!(
                "{}<br/>{}",
                pre(lines.strip_suffix('\n').unwrap()),
                pre("let y = 2;")
            )
        );
    }

    /// Any tree, valid or not, of any depth: no panic, and nothing comes out of a script.
    #[test]
    fn random_trees_never_break_the_emitter() {
        use comrak::nodes::{
            AlertType, ListType, NodeAlert, NodeBlockDirective, NodeCode, NodeCodeBlock,
            NodeDescriptionItem, NodeFootnoteDefinition, NodeFootnoteReference, NodeHeading,
            NodeHtmlBlock, NodeLink, NodeMath, NodeTable, NodeTaskItem, TableAlignment,
        };

        struct Rng(u64);
        impl Rng {
            fn below(&mut self, n: usize) -> usize {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                (self.0 % n as u64) as usize
            }
        }

        fn value(rng: &mut Rng) -> NodeValue {
            let words = [
                "PROJ-1", "a", "\\-", "\\", "<script>", "", " ", "MY_X-2", "\r\n", "\u{1b}", "\0",
            ];
            let word = words[rng.below(words.len())].to_owned();
            match rng.below(36) {
                0 => NodeValue::Document,
                1 => NodeValue::Paragraph,
                2 => NodeValue::Heading(NodeHeading {
                    level: rng.below(9) as u8,
                    ..Default::default()
                }),
                3 | 4 => NodeValue::Text(word.into()),
                5 | 6 => NodeValue::Raw(word),
                7 => NodeValue::Escaped,
                8 => NodeValue::Strong,
                9 => NodeValue::Emph,
                10 => NodeValue::Link(Box::new(NodeLink {
                    url: ["u", "javascript:x"][rng.below(2)].into(),
                    title: word,
                })),
                11 => NodeValue::Image(Box::new(NodeLink {
                    url: ["i.png", "data:text/html,x"][rng.below(2)].into(),
                    title: String::new(),
                })),
                12 => NodeValue::Code(NodeCode {
                    num_backticks: 1,
                    literal: word,
                }),
                13 => NodeValue::CodeBlock(Box::new(NodeCodeBlock {
                    info: ["", "rust", "mermaid", "math"][rng.below(4)].into(),
                    literal: word,
                    ..Default::default()
                })),
                14 => NodeValue::HtmlInline(
                    ["<a href=\"u\">", "</a>", "<b onclick=x>"][rng.below(3)].into(),
                ),
                15 => NodeValue::HtmlBlock(NodeHtmlBlock {
                    block_type: 6,
                    literal: ["<div>", "</div>", "<iframe>", "<!-- x"][rng.below(4)].into(),
                }),
                16 => NodeValue::List(NodeList {
                    list_type: [ListType::Bullet, ListType::Ordered][rng.below(2)],
                    start: rng.below(3),
                    ..Default::default()
                }),
                17 => NodeValue::Item(NodeList::default()),
                18 => NodeValue::TaskItem(NodeTaskItem {
                    symbol: [None, Some('x')][rng.below(2)],
                    symbol_sourcepos: (0, 0, 0, 0).into(),
                }),
                19 => NodeValue::Table(Box::new(NodeTable {
                    alignments: vec![TableAlignment::Center; rng.below(3)],
                    ..Default::default()
                })),
                20 => NodeValue::TableRow(rng.below(2) == 0),
                21 => NodeValue::TableCell,
                22 => NodeValue::BlockQuote,
                23 => NodeValue::Alert(Box::new(NodeAlert {
                    alert_type: AlertType::Tip,
                    title: Some(word),
                    multiline: false,
                    fence_length: 0,
                    fence_offset: 0,
                })),
                24 => NodeValue::BlockDirective(Box::new(NodeBlockDirective {
                    fence_length: 3,
                    fence_offset: 0,
                    info: ["note x", "other", ""][rng.below(3)].into(),
                })),
                25 => NodeValue::FootnoteDefinition(NodeFootnoteDefinition {
                    name: word,
                    total_references: 0,
                }),
                26 => NodeValue::FootnoteReference(Box::new(NodeFootnoteReference {
                    name: word,
                    ix: u32::MAX,
                    ..Default::default()
                })),
                27 => NodeValue::DescriptionList,
                28 => NodeValue::DescriptionItem(NodeDescriptionItem::default()),
                29 => [NodeValue::DescriptionTerm, NodeValue::DescriptionDetails][rng.below(2)]
                    .clone(),
                30 => NodeValue::Math(NodeMath {
                    dollar_math: true,
                    display_math: rng.below(2) == 0,
                    literal: word,
                }),
                31 => [NodeValue::SoftBreak, NodeValue::LineBreak][rng.below(2)].clone(),
                32 => NodeValue::ThematicBreak,
                33 => NodeValue::FrontMatter(word),
                34 => NodeValue::EscapedTag("~"),
                _ => [
                    NodeValue::Superscript,
                    NodeValue::Subscript,
                    NodeValue::Highlight,
                    NodeValue::Insert,
                    NodeValue::Strikethrough,
                    NodeValue::Underline,
                ][rng.below(6)]
                .clone(),
            }
        }

        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let diagrams: DiagramAttachments =
            [(mermaid::diagram("PROJ-1").file_name, "1-a b.svg".to_owned())]
                .into_iter()
                .collect();
        let mut deepest = 0;
        for _ in 0..500 {
            let arena = Arena::new();
            let root = node(&arena, NodeValue::Document);
            let mut nodes = vec![root];
            for _ in 0..rng.below(300) {
                // Mostly deepen the newest nodes, so some trees exceed the depth limit.
                let parent = nodes[nodes.len() - 1 - rng.below(nodes.len().min(3))];
                let child = node(&arena, value(&mut rng));
                parent.append(child);
                nodes.push(child);
                deepest = deepest.max(child.ancestors().count());
            }
            let html = emit_document(root, &ToHtmlOptions::default(), &diagrams);
            assert!(
                !html.contains("<script") && !html.contains("<iframe"),
                "{html}"
            );
            assert!(!html.contains("onclick"), "{html}");
            assert!(html.chars().all(html::xml_allows), "{html:?}");
        }
        assert!(deepest > 2 * MAX_DEPTH, "{deepest}");
    }

    #[test]
    fn work_item_setting_applies() {
        let arena = Arena::new();
        let root = paragraph(&arena, vec![text("PROJ-12 MP-1")]);
        let options = ToHtmlOptions {
            work_items: WorkItemIds::Prefixes(vec!["MP".into()]),
            ..ToHtmlOptions::default()
        };
        let html = emit_document(root, &options, &DiagramAttachments::new());
        assert!(html.starts_with("PROJ-12 <span"), "{html}");
        assert!(html.contains(r#"data-item-id="MP-1""#), "{html}");
        let options = ToHtmlOptions {
            work_items: WorkItemIds::Off,
            ..ToHtmlOptions::default()
        };
        assert_eq!(
            emit_document(root, &options, &DiagramAttachments::new()),
            "PROJ-12 MP-1"
        );
    }
}
