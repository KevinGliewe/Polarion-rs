//! The comrak options: one value for parsing and for formatting, because some extensions
//! (`autolink`, `block_directive`) change what comrak's CommonMark writer escapes.

use comrak::nodes::AstNode;
use comrak::{Arena, Options};

/// GitHub flavored Markdown plus the extensions both directions map.
pub(crate) fn options() -> Options<'static> {
    let mut o = Options::default();
    let e = &mut o.extension;
    e.strikethrough = true;
    e.table = true;
    e.autolink = true;
    e.tasklist = true;
    e.superscript = true;
    e.subscript = true;
    e.highlight = true;
    e.insert = true;
    e.math_dollars = true;
    e.math_code = true;
    e.footnotes = true;
    e.description_lists = true;
    e.alerts = true;
    e.block_directive = true;
    // `PROJ\-1` must stay distinguishable from `PROJ-1` (only the latter is a link).
    o.parse.escaped_char_spans = true;
    o.parse.smart = false;
    o.render.prefer_fenced = true;
    o.render.width = 0;
    o
}

pub(crate) fn parse<'a>(arena: &'a Arena<'a>, markdown: &str) -> &'a AstNode<'a> {
    comrak::parse_document(arena, markdown, &options())
}
