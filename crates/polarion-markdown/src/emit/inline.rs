//! Inlines: styled spans, links, images, formulas, and the work item ids in runs of text.

use std::fmt::Write as _;

use comrak::arena_tree::NodeEdge;
use comrak::nodes::{AstNode, NodeLink, NodeValue};

use super::{Emitter, MAX_DEPTH, unix_lines, without_cr};
use crate::html::{self, push_attr, push_text};
use crate::{ids, styles};

/// What inlines are inside of.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Ctx {
    /// Inside a Markdown link: ids are not linked (inside a raw `<a>` neither, see
    /// `Emitter::raw_link`).
    pub(super) in_link: bool,
    /// Inside a heading: line breaks become spaces.
    pub(super) heading: bool,
}

/// Whether a node takes part in a run of text that work item ids are detected in.
fn in_run(value: &NodeValue) -> bool {
    matches!(
        value,
        NodeValue::Text(_) | NodeValue::Escaped | NodeValue::Raw(_)
    )
}

/// A `Raw` that stands for one escaped character: `\` and an ASCII punctuation character.
fn raw_escape(literal: &str) -> Option<&str> {
    let b = literal.as_bytes();
    (b.len() == 2 && b[0] == b'\\' && b[1].is_ascii_punctuation()).then(|| &literal[1..])
}

impl<'a> Emitter<'a, '_> {
    /// The inline children of `parent` (at `depth`) as HTML.
    pub(super) fn inlines(&mut self, parent: &'a AstNode<'a>, depth: usize, ctx: Ctx) -> String {
        let children: Vec<&'a AstNode<'a>> = parent.children().collect();
        let mut out = String::new();
        self.inline_seq(&children, depth + 1, ctx, &mut out);
        out
    }

    /// Sibling inlines at `depth`.
    pub(super) fn inline_seq(
        &mut self,
        nodes: &[&'a AstNode<'a>],
        depth: usize,
        ctx: Ctx,
        out: &mut String,
    ) {
        let mut i = 0;
        while i < nodes.len() {
            if in_run(&nodes[i].data.borrow().value) {
                let end = (i..nodes.len())
                    .find(|&j| !in_run(&nodes[j].data.borrow().value))
                    .unwrap_or(nodes.len());
                self.run(&nodes[i..end], !ctx.in_link && !self.raw_link, out);
                i = end;
                continue;
            }
            let node = nodes[i];
            i += 1;
            if depth > MAX_DEPTH {
                push_text(out, &plain_text(node));
                continue;
            }
            let ast = node.data.borrow();
            match &ast.value {
                NodeValue::SoftBreak | NodeValue::LineBreak => {
                    out.push_str(if ctx.heading { " " } else { styles::BR });
                }
                NodeValue::Code(code) => {
                    let _ = write!(out, "<span style=\"{}\">", styles::MONO);
                    push_text(out, &without_cr(&code.literal));
                    out.push_str("</span>");
                }
                NodeValue::HtmlInline(raw) => {
                    if raw.starts_with("<!--") {
                        continue;
                    }
                    let clean = html::sanitize_raw_html(raw);
                    self.raw_link = html::link_open_after(&clean, self.raw_link);
                    out.push_str(&clean);
                }
                NodeValue::Emph => self.styled(node, styles::ITALIC, depth, ctx, out),
                NodeValue::Strong => self.styled(node, styles::BOLD, depth, ctx, out),
                NodeValue::Strikethrough => self.styled(node, styles::STRIKE, depth, ctx, out),
                NodeValue::Insert | NodeValue::Underline => {
                    self.styled(node, styles::UNDERLINE, depth, ctx, out)
                }
                NodeValue::Highlight => self.styled(node, styles::HIGHLIGHT, depth, ctx, out),
                NodeValue::Superscript => self.tagged(node, "sup", depth, ctx, out),
                NodeValue::Subscript => self.tagged(node, "sub", depth, ctx, out),
                NodeValue::Link(link) => self.link(node, link, depth, ctx, out),
                NodeValue::Image(image) => image_html(node, image, out),
                // Inline formulas are normalized like code spans, display formulas are raw.
                NodeValue::Math(math) if math.display_math => {
                    formula(out, &unix_lines(&math.literal), true);
                }
                NodeValue::Math(math) => formula(out, &without_cr(&math.literal), false),
                NodeValue::FootnoteReference(reference) => {
                    let _ = write!(out, "<sup>[{}]</sup>", reference.ix);
                }
                // Inline extensions this crate does not enable, block nodes in an inline
                // container (hand-built trees): their content.
                _ => {
                    let children: Vec<&'a AstNode<'a>> = node.children().collect();
                    self.inline_seq(&children, depth + 1, ctx, out);
                }
            }
        }
    }

    fn styled(
        &mut self,
        node: &'a AstNode<'a>,
        style: &str,
        depth: usize,
        ctx: Ctx,
        out: &mut String,
    ) {
        let _ = write!(out, "<span style=\"{style}\">");
        out.push_str(&self.inlines(node, depth, ctx));
        out.push_str("</span>");
    }

    fn tagged(
        &mut self,
        node: &'a AstNode<'a>,
        tag: &str,
        depth: usize,
        ctx: Ctx,
        out: &mut String,
    ) {
        let _ = write!(out, "<{tag}>");
        out.push_str(&self.inlines(node, depth, ctx));
        let _ = write!(out, "</{tag}>");
    }

    /// A link; one to a dangerous URL is only its content. Ids in the label are not linked. The
    /// `<a>` closes a raw one still open (as a browser does), so none is open after the link.
    fn link(
        &mut self,
        node: &'a AstNode<'a>,
        link: &NodeLink,
        depth: usize,
        ctx: Ctx,
        out: &mut String,
    ) {
        let ctx = Ctx {
            in_link: true,
            ..ctx
        };
        if html::dangerous_url(&link.url) {
            out.push_str(&self.inlines(node, depth, ctx));
            return;
        }
        out.push_str("<a href=\"");
        push_attr(out, &link.url);
        out.push('"');
        if !link.title.is_empty() {
            out.push_str(" title=\"");
            push_attr(out, &unix_lines(&link.title));
            out.push('"');
        }
        let _ = write!(
            out,
            " target=\"{}\" class=\"{}\">",
            styles::LINK_TARGET,
            styles::LINK_CLASS
        );
        out.push_str(&self.inlines(node, depth, ctx));
        out.push_str("</a>");
        self.raw_link = false;
    }

    /// A run of `Text`, `Escaped` and `Raw` siblings: escaped text with the work item ids linked.
    fn run(&self, nodes: &[&'a AstNode<'a>], link_ids: bool, out: &mut String) {
        let mut text = String::new();
        // Per byte of `text`: escaped in the Markdown. Filled up to the last escaped byte.
        let mut escaped: Vec<bool> = Vec::new();
        let mut push = |piece: &str, is_escaped: bool| {
            if is_escaped {
                escaped.resize(text.len(), false);
                escaped.resize(text.len() + piece.len(), true);
            }
            text.push_str(piece);
        };
        for node in nodes {
            match &node.data.borrow().value {
                NodeValue::Text(t) => push(t, false),
                NodeValue::Raw(literal) => match raw_escape(literal) {
                    Some(c) => push(c, true),
                    None => push(literal, false),
                },
                NodeValue::Escaped => {
                    for child in node.children() {
                        if let NodeValue::Text(t) = &child.data.borrow().value {
                            push(t, true);
                        }
                    }
                }
                _ => {}
            }
        }
        let found = if link_ids {
            ids::find(&text, &escaped, &self.options.work_items)
        } else {
            Vec::new()
        };
        let mut last = 0;
        for id in found {
            push_text(out, &text[last..id.start]);
            work_item_link(out, &text[id.clone()]);
            last = id.end;
        }
        push_text(out, &text[last..]);
    }
}

/// `<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="ID" data-option-id="long"></span>`
fn work_item_link(out: &mut String, id: &str) {
    let _ = write!(
        out,
        "<span class=\"{}\" data-type=\"{}\" id=\"fake\" data-item-id=\"",
        styles::WORK_ITEM_CLASS,
        styles::WORK_ITEM_TYPE
    );
    push_attr(out, id);
    let _ = write!(
        out,
        "\" data-option-id=\"{}\"></span>",
        styles::WORK_ITEM_OPTION
    );
}

/// `<span data-source="SOURCE"[ data-inline="false"] class="polarion-rte-formula"></span>`
pub(super) fn formula(out: &mut String, source: &str, display: bool) {
    out.push_str("<span data-source=\"");
    push_attr(out, source);
    out.push('"');
    if display {
        out.push_str(" data-inline=\"false\"");
    }
    let _ = write!(out, " class=\"{}\"></span>", styles::FORMULA_CLASS);
}

/// An image; one with a dangerous source is only its alt text.
fn image_html<'a>(node: &'a AstNode<'a>, image: &NodeLink, out: &mut String) {
    let alt = plain_text_of_children(node);
    if html::dangerous_url(&image.url) {
        push_text(out, &alt);
        return;
    }
    out.push_str("<img src=\"");
    push_attr(out, &image.url);
    out.push('"');
    if !image.title.is_empty() {
        out.push_str(" title=\"");
        push_attr(out, &unix_lines(&image.title));
        out.push('"');
    }
    if !alt.is_empty() {
        out.push_str(" alt=\"");
        push_attr(out, &alt);
        out.push('"');
    }
    let _ = write!(out, " style=\"{}\"/>", styles::IMG);
}

fn plain_text_of_children<'a>(node: &'a AstNode<'a>) -> String {
    let mut text = String::new();
    for child in node.children() {
        text.push_str(&plain_text(child));
    }
    text
}

/// The text of a subtree, without markup (iterative: any depth). Blocks are separated by a
/// space, line breaks become spaces, raw HTML is left out.
pub(super) fn plain_text<'a>(node: &'a AstNode<'a>) -> String {
    let mut text = String::new();
    for edge in node.traverse() {
        let NodeEdge::Start(n) = edge else {
            continue;
        };
        let ast = n.data.borrow();
        let value = &ast.value;
        if value.block() && !text.is_empty() && !text.ends_with(' ') {
            text.push(' ');
        }
        match value {
            NodeValue::Text(t) => text.push_str(t),
            NodeValue::Raw(literal) => text.push_str(raw_escape(literal).unwrap_or(literal)),
            NodeValue::Code(code) => text.push_str(&code.literal),
            NodeValue::Math(math) => text.push_str(&math.literal),
            NodeValue::CodeBlock(code) => text.push_str(code.literal.trim_end_matches('\n')),
            NodeValue::SoftBreak | NodeValue::LineBreak => text.push(' '),
            _ => {}
        }
    }
    text
}
