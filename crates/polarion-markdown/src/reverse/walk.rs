//! The DOM walk: Polarion HTML to blocks and styled inline atoms.
//!
//! Each container (the document, a quote, a list item, a table cell, ...) becomes a flow of items:
//! inline atoms with their style, blocks, and boundaries where a block element without a Markdown
//! form (a plain `div`, `p`, ...) starts or ends. Elements nested deeper than [`MAX_DEPTH`] become
//! their text; Markdown containers nested deeper than [`MAX_BLOCKS`] are flattened, so the
//! conversion back (which writes nesting deeper than 64 levels as text) keeps all of it. Before
//! parsing, tags nested deeper than [`MAX_NESTING`] are left out (parsing deep nesting takes
//! quadratic time).
//!
//! Block elements pass on to their text only the styles it inherits and shows (fonts, colors,
//! decorations); hidden elements stay hidden; carriage returns in code and formulas are line
//! breaks, tabs and line breaks in URLs are left out (as browsers do).

use std::borrow::Cow;

use comrak::nodes::{AlertType, TableAlignment};
use scraper::{CaseSensitivity, ElementRef, Html, Node};

use super::css::{self, Decl, Style, Wrapper};
use super::raw;
use crate::styles::{self, Align};
use crate::{DiagramSources, RawHtml, ToMarkdownOptions, html, ids, mermaid, refs};

/// Elements nested deeper than this become their text.
pub(super) const MAX_DEPTH: usize = 64;
/// Markdown block nesting (quote levels, two per list level, alerts) deeper than this is
/// flattened.
pub(super) const MAX_BLOCKS: usize = 40;

/// Spans and colspans larger than this count as this (HTML's own limit for colspans).
const MAX_SPAN: usize = 1000;

/// Elements nested deeper than this are left out before parsing (their text stays): the HTML
/// parser takes time quadratic in the nesting depth, and the walk writes anything below
/// [`MAX_DEPTH`] as text anyway.
const MAX_NESTING: usize = 512;

/// The HTML with the tags of elements nested deeper than [`MAX_NESTING`] left out (a left-out
/// `<br>` or block-level element becomes a space). The nesting is followed with the end tags the
/// HTML parser implies for the elements that are commonly left open (`li`, `p`, table parts, ...);
/// where it differs from the parser's, it errs towards less nesting. Linear time.
fn cap_nesting(html: &str) -> Cow<'_, str> {
    if html.bytes().filter(|&b| b == b'<').count() <= MAX_NESTING {
        return Cow::Borrowed(html);
    }
    let mut open = OpenElements::default();
    let mut out = String::with_capacity(html.len());
    let mut changed = false;
    let left_out = |out: &mut String, name: &str| {
        if name == "br" || html::is_block_element(name) {
            out.push(' ');
        }
    };
    for token in html::tokens(html) {
        let tag = match token {
            html::Token::Tag(tag) => tag,
            html::Token::Text(src) | html::Token::Bogus(src) | html::Token::Unclosed(src) => {
                out.push_str(src);
                continue;
            }
            html::Token::Comment { src, .. } => {
                out.push_str(src);
                continue;
            }
        };
        let name = tag.name.to_ascii_lowercase();
        if tag.closing {
            match open.last(&name) {
                Some(at) => {
                    let kept = open.kept[at];
                    open.truncate(at);
                    if kept {
                        out.push_str(tag.src);
                    } else {
                        changed = true;
                        left_out(&mut out, &name);
                    }
                }
                // A stray end tag: the parser ignores it (or reads a break, a paragraph).
                None if open.len() < MAX_NESTING => out.push_str(tag.src),
                None => {
                    changed = true;
                    left_out(&mut out, &name);
                }
            }
            continue;
        }
        open.close_implied(&name);
        let kept = open.len() < MAX_NESTING;
        if kept {
            out.push_str(tag.src);
        } else {
            changed = true;
            left_out(&mut out, &name);
        }
        if !(raw::is_void(&name) || tag.self_closing) {
            open.push(name, kept);
        }
    }
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(html)
    }
}

/// The open elements while [`cap_nesting`] reads tags, with the positions of each name.
#[derive(Default)]
struct OpenElements {
    names: Vec<String>,
    /// Whether the element's tags are kept.
    kept: Vec<bool>,
    positions: std::collections::HashMap<String, Vec<usize>>,
}

impl OpenElements {
    fn len(&self) -> usize {
        self.names.len()
    }

    fn push(&mut self, name: String, kept: bool) {
        self.positions
            .entry(name.clone())
            .or_default()
            .push(self.names.len());
        self.names.push(name);
        self.kept.push(kept);
    }

    /// The position of the innermost open element of this name.
    fn last(&self, name: &str) -> Option<usize> {
        self.positions.get(name).and_then(|p| p.last().copied())
    }

    /// Closes the element at `at` and the elements inside it.
    fn truncate(&mut self, at: usize) {
        while self.names.len() > at {
            if let Some(name) = self.names.pop()
                && let Some(positions) = self.positions.get_mut(&name)
            {
                positions.pop();
            }
            self.kept.pop();
        }
    }

    /// Closes the open elements a start tag of `name` ends implicitly: the innermost element
    /// it closes, unless an element bounding the search is open inside it.
    fn close_implied(&mut self, name: &str) {
        let (closes, bounds): (&[&str], &[&str]) = match name {
            "li" => (&["li"], &["ul", "ol", "menu", "dir", "table"]),
            "dt" | "dd" => (&["dt", "dd"], &["dl", "table"]),
            "tr" => (&["tr"], &["table", "tbody", "thead", "tfoot"]),
            "td" | "th" => (&["td", "th"], &["tr", "table"]),
            "tbody" | "thead" | "tfoot" => (&["tbody", "thead", "tfoot"], &["table"]),
            "option" => (&["option"], &["select"]),
            _ if name == "p" || html::is_block_element(name) => {
                (&["p"], &["table", "td", "th", "caption", "button"])
            }
            _ => return,
        };
        let innermost = |names: &[&str]| names.iter().filter_map(|n| self.last(n)).max();
        if let Some(at) = innermost(closes)
            && innermost(bounds).is_none_or(|bound| bound < at)
        {
            self.truncate(at);
        }
    }
}

/// One item of a container's flow.
#[derive(Debug)]
pub(super) enum Item {
    Atom(Atom),
    Block(Block),
    /// A block element with vertical margins and without a Markdown form (`p`, ...) starts or
    /// ends: the paragraph ends.
    Boundary,
    /// A block element without margins and without a Markdown form (a plain `div`, ...) starts or
    /// ends: the line ends, as at a `<br>`, if it has content.
    LineEnd,
}

/// A piece of inline content with its style.
#[derive(Debug, Clone)]
pub(super) struct Atom {
    pub(super) kind: AtomKind,
    pub(super) style: Style,
}

#[derive(Debug, Clone)]
pub(super) enum AtomKind {
    Text(String),
    /// `<br>`.
    Br,
    /// A `<br>` beyond the separators (keep mode), written as raw HTML.
    RawBr,
    /// An element without content: only its style counts (for empty headings).
    Empty,
    Image {
        url: String,
        title: String,
        alt: String,
    },
    /// Raw inline HTML (keep mode, or what Markdown cannot hold in either mode).
    Html(String),
    /// A work item link: the bare id where the conversion back links it, else the fallback.
    Id {
        id: String,
        fallback: IdFallback,
    },
    /// A formula; `raw` is its span, written where Markdown cannot hold it exactly.
    Math {
        source: String,
        display: bool,
        raw: String,
    },
}

/// What a work item link is written as where the conversion back would not link its id.
#[derive(Debug, Clone)]
pub(super) enum IdFallback {
    /// The link span: raw in keep mode, the id as text in drop mode.
    Span(String),
    /// An anchor without a label: a link without text to this URL and title.
    Link { url: String, title: String },
}

/// A link of the document.
#[derive(Debug, Clone)]
pub(super) struct LinkInfo {
    pub(super) url: String,
    pub(super) title: String,
    /// The id of a link to a work item: with the id as its label, the bare id where the conversion
    /// back links it.
    pub(super) work_item: Option<String>,
}

/// The flow of a container; `in_li` selects the separators of list items.
#[derive(Debug, Default)]
pub(super) struct Container {
    pub(super) items: Vec<Item>,
    pub(super) in_li: bool,
}

#[derive(Debug)]
pub(super) enum Block {
    /// A quote nested this deep.
    Quote(usize, Container),
    Alert(AlertType, Option<String>, Container),
    /// A code block: language (maybe empty) and text.
    Code(String, String),
    /// A mermaid diagram (its source).
    Diagram(String),
    List {
        ordered: bool,
        start: usize,
        /// Each item: whether it is a task (and done), its content.
        items: Vec<(Option<bool>, Container)>,
    },
    /// A GFM table: column alignments, the header row first, every row as wide as the header;
    /// cells hold inline content only.
    Table {
        aligns: Vec<TableAlignment>,
        rows: Vec<Vec<Container>>,
    },
    /// A raw HTML block (keep mode).
    Raw(String),
    /// A styled `div` kept raw around its blocks (keep mode): its start tag.
    RawWrap(String, Container),
    Rule,
}

/// The walk's result: the document's flow and the links and raw wrappers its atoms refer to.
#[derive(Debug)]
pub(super) struct Walked {
    pub(super) body: Container,
    pub(super) links: Vec<LinkInfo>,
    pub(super) wrappers: Vec<Wrapper>,
}

pub(super) fn walk(html: &str, options: &ToMarkdownOptions, sources: &DiagramSources) -> Walked {
    let dom = Html::parse_fragment(&cap_nesting(html));
    let mut walker = Walker {
        keep: options.raw_html == RawHtml::Keep,
        sources,
        links: Vec::new(),
        wrappers: Vec::new(),
    };
    let mut items = Vec::new();
    walker.children(dom.root_element(), &Ctx::default(), &mut items);
    Walked {
        body: Container {
            items,
            in_li: false,
        },
        links: walker.links,
        wrappers: walker.wrappers,
    }
}

struct Walker<'o> {
    keep: bool,
    sources: &'o DiagramSources,
    links: Vec<LinkInfo>,
    wrappers: Vec<Wrapper>,
}

#[derive(Debug, Clone, Default)]
struct Ctx {
    style: Style,
    /// Element nesting.
    depth: usize,
    /// Markdown block nesting.
    blocks: usize,
    /// Inside a table cell: no diagram blocks.
    cell: bool,
    /// The container is a list item (for wrappers kept around blocks).
    in_li: bool,
}

fn atom(kind: AtomKind, style: &Style) -> Item {
    Item::Atom(Atom {
        kind,
        style: style.clone(),
    })
}

fn style_of(el: ElementRef<'_>) -> Vec<Decl> {
    style_attr(el.value())
        .map(css::declarations)
        .unwrap_or_default()
}

/// The `style` of an element, unless the forward sanitizer drops it (it could run script).
fn style_attr(element: &scraper::node::Element) -> Option<&str> {
    element.attr("style").filter(|s| !html::dangerous_style(s))
}

/// The declarations of a block element that reach its text: properties CSS inherits and that
/// show on inline content (fonts, colors, decorations, ...). The others (margins, alignment,
/// backgrounds, list markers, ...) shape the element's box, which a span around its text cannot
/// do; they are left out.
fn text_style_of(el: ElementRef<'_>) -> Vec<Decl> {
    let mut decls = style_of(el);
    decls.retain(|d| {
        matches!(
            d.prop.as_str(),
            "color"
                | "font"
                | "font-family"
                | "font-size"
                | "font-style"
                | "font-variant"
                | "font-weight"
                | "letter-spacing"
                | "line-height"
                | "text-decoration"
                | "text-decoration-line"
                | "text-shadow"
                | "text-transform"
                | "visibility"
                | "word-spacing"
        )
    });
    decls
}

/// Whether an element is not shown: the `hidden` attribute or `display: none`.
fn is_hidden(el: ElementRef<'_>) -> bool {
    el.value().attr("hidden").is_some()
        || style_of(el)
            .iter()
            .any(|d| d.prop == "display" && css::norm(&d.value) == "none")
}

/// A URL as browsers read it: without ASCII tabs and line breaks, without control characters and
/// spaces at its edges, with characters XML does not allow replaced.
fn url_of(value: &str) -> String {
    let url: String = value
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    html::xml_safe(url.trim_matches(|c: char| c <= ' ')).into_owned()
}

/// Whether browsers give a block element vertical margins by default (a blank line around it).
fn has_margins(name: &str) -> bool {
    matches!(
        name,
        "p" | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "dl"
            | "figure"
            | "blockquote"
            | "ul"
            | "ol"
            | "menu"
            | "dir"
            | "pre"
            | "listing"
            | "xmp"
            | "fieldset"
            | "hr"
    )
}

fn has_class(el: ElementRef<'_>, class: &str) -> bool {
    el.value()
        .has_class(class, CaseSensitivity::AsciiCaseInsensitive)
}

fn blank(value: Option<&str>) -> bool {
    value.is_none_or(|v| v.trim().is_empty())
}

/// Whether a string is exactly one work item id (`PROJ-123`).
pub(super) fn is_id(s: &str) -> bool {
    !s.is_empty() && matches!(ids::tokens(s, &[]).as_slice(), [token] if *token == (0..s.len()))
}

/// The work item id a URL to a work item names (`…workitem?id=PROJ-7`).
fn work_item_of(url: &str) -> Option<String> {
    const KEY: &str = "workitem?id=";
    let start = url.find(KEY)? + KEY.len();
    let id = url[start..].split(['&', '#']).next()?;
    let id = html::percent_decode(id);
    is_id(&id).then(|| id.into_owned())
}

/// The only block of a flow that holds nothing else but breaks and whitespace, if it is a code
/// block (taken out of the flow).
fn lone_code(items: &mut [Item]) -> Option<Block> {
    let mut code = None;
    for (i, item) in items.iter().enumerate() {
        match item {
            Item::Block(Block::Code(..) | Block::Diagram(_)) if code.is_none() => code = Some(i),
            Item::Atom(Atom {
                kind: AtomKind::Br | AtomKind::Empty,
                ..
            })
            | Item::Boundary
            | Item::LineEnd => {}
            Item::Atom(Atom {
                kind: AtomKind::Text(t),
                ..
            }) if t.chars().all(|c| c.is_ascii_whitespace()) => {}
            _ => return None,
        }
    }
    let item = std::mem::replace(&mut items[code?], Item::Boundary);
    match item {
        Item::Block(block) => Some(block),
        _ => None,
    }
}

/// Whether the only element child is a `pre` and all other children are whitespace.
fn only_pre(el: ElementRef<'_>) -> bool {
    let mut pre = false;
    for child in el.children() {
        match child.value() {
            Node::Element(e) if e.name() == "pre" && !pre => pre = true,
            Node::Element(_) => return false,
            Node::Text(t) if !t.trim().is_empty() => return false,
            _ => {}
        }
    }
    pre
}

/// The only child element of a quote div, if it is a monospace element containing a `<br>`
/// (and everything else is whitespace): the code block older converters wrote.
fn older_code(el: ElementRef<'_>) -> Option<String> {
    let mut found = None;
    for child in el.children() {
        match child.value() {
            Node::Element(_) if found.is_none() => found = ElementRef::wrap(child),
            Node::Element(_) => return None,
            Node::Text(t) if !t.trim().is_empty() => return None,
            _ => {}
        }
    }
    let span = found?;
    let name = span.value().name();
    let mono = match name {
        "code" | "tt" | "kbd" | "samp" => true,
        "span" => style_of(span)
            .iter()
            .any(|d| d.prop == "font-family" && css::is_mono(&d.value)),
        _ => false,
    };
    let has_br = span
        .descendants()
        .any(|n| matches!(n.value(), Node::Element(e) if e.name() == "br"));
    (mono && has_br).then(|| raw::code_lines(span))
}

/// The CSS size of a `<font size>` value: 1 to 7, or relative to 3 with a sign.
fn font_size_keyword(value: &str) -> Option<&'static str> {
    let value = value.trim();
    let size = if let Some(up) = value.strip_prefix('+') {
        3 + up.trim().parse::<i64>().ok()?
    } else if let Some(down) = value.strip_prefix('-') {
        3 - down.trim().parse::<i64>().ok()?
    } else {
        value.parse::<i64>().ok()?
    };
    let sizes = [
        "x-small",
        "small",
        "medium",
        "large",
        "x-large",
        "xx-large",
        "xxx-large",
    ];
    Some(sizes[usize::try_from(size.clamp(1, 7) - 1).unwrap_or(0)])
}

/// A quote div's depth: its only declaration is `margin-left: Npx`, one level per 40px.
fn quote_depth(decls: &[Decl]) -> Option<usize> {
    let [decl] = decls else {
        return None;
    };
    if decl.prop != "margin-left" {
        return None;
    }
    let depth = (css::px(&decl.value)? / 40.0).round();
    (1.0..=8.0).contains(&depth).then_some(depth as usize)
}

/// The language of a `pre`: a `language-x` or `lang-x` class or a `data-lang`/`data-language`
/// attribute, on the `pre` or its `code` child.
fn language(pre: ElementRef<'_>) -> String {
    let code = pre.child_elements().find(|c| c.value().name() == "code");
    for el in std::iter::once(pre).chain(code) {
        let element = el.value();
        // In the order written (`classes()` sorts them).
        for class in element.attr("class").unwrap_or("").split_ascii_whitespace() {
            for prefix in [styles::LANGUAGE_CLASS_PREFIX, "lang-"] {
                if let Some(lang) = class.strip_prefix(prefix)
                    && !lang.is_empty()
                {
                    return html::xml_safe(lang).into_owned();
                }
            }
        }
        for attr in ["data-lang", "data-language"] {
            if let Some(lang) = element.attr(attr)
                && let Some(word) = lang.split_whitespace().next()
            {
                return html::xml_safe(word).into_owned();
            }
        }
    }
    String::new()
}

impl<'a> Walker<'_> {
    fn children(&mut self, el: ElementRef<'a>, ctx: &Ctx, out: &mut Vec<Item>) {
        for child in el.children() {
            self.child(child.value(), ElementRef::wrap(child), ctx, out);
        }
    }

    fn child(&mut self, node: &Node, el: Option<ElementRef<'a>>, ctx: &Ctx, out: &mut Vec<Item>) {
        match (node, el) {
            (Node::Text(text), _) => {
                if !text.is_empty() {
                    out.push(atom(
                        AtomKind::Text(html::xml_safe(text).into_owned()),
                        &ctx.style,
                    ));
                }
            }
            (Node::Element(_), Some(el)) => self.element(el, ctx, out),
            _ => {}
        }
    }

    fn element(&mut self, el: ElementRef<'a>, ctx: &Ctx, out: &mut Vec<Item>) {
        let name = el.value().name();
        if raw::is_dropped(name) {
            return;
        }
        // Hidden content stays hidden: kept where the element's style is written raw (keep
        // mode: inline elements, styled divs and paragraphs, tables, images), left out elsewhere.
        if is_hidden(el)
            && (!self.keep
                || el.value().attr("hidden").is_some()
                || (html::is_block_element(name) && !matches!(name, "div" | "p" | "table")))
        {
            return;
        }
        if ctx.depth >= MAX_DEPTH {
            let block = html::is_block_element(name);
            if block {
                out.push(Item::LineEnd);
            }
            let text = raw::flat_text(el);
            if !text.is_empty() {
                out.push(atom(AtomKind::Text(text), &ctx.style));
            }
            if block {
                out.push(Item::LineEnd);
            }
            return;
        }
        let mut inner = ctx.clone();
        inner.depth += 1;
        match name {
            "br" => out.push(atom(AtomKind::Br, &ctx.style)),
            // A task list as Markdown converters write it: the checkbox is the task marker.
            "input"
                if el
                    .value()
                    .attr("type")
                    .is_some_and(|t| t.trim().eq_ignore_ascii_case("checkbox")) =>
            {
                let marker = match el.value().attr("checked") {
                    Some(_) => styles::TASK_DONE,
                    None => styles::TASK_OPEN,
                };
                out.push(atom(AtomKind::Text(marker.to_owned()), &ctx.style));
            }
            "img" => self.image(el, ctx, out),
            "a" => self.anchor(el, inner, out),
            "span" if has_class(el, styles::WORK_ITEM_CLASS) => self.link_span(el, &inner, out),
            "span" if has_class(el, styles::FORMULA_CLASS) => self.formula(el, &inner, out),
            "font" => {
                let element = el.value();
                let mut decls = Vec::new();
                if let Some(color) = element.attr("color") {
                    decls.push(Decl {
                        prop: "color".into(),
                        value: color.trim().to_owned(),
                    });
                }
                if let Some(face) = element.attr("face") {
                    decls.push(Decl {
                        prop: "font-family".into(),
                        value: face.trim().to_owned(),
                    });
                }
                if let Some(size) = element.attr("size").and_then(font_size_keyword) {
                    decls.push(Decl {
                        prop: "font-size".into(),
                        value: size.to_owned(),
                    });
                }
                decls.retain(|d| !d.value.is_empty() && css::balanced(&d.value));
                css::apply(&mut inner.style, &decls, &mut self.wrappers);
                self.inline(el, inner, out);
            }
            "big" | "small" => {
                let decls = [Decl {
                    prop: "font-size".into(),
                    value: if name == "big" { "larger" } else { "smaller" }.into(),
                }];
                css::apply(&mut inner.style, &decls, &mut self.wrappers);
                self.inline(el, inner, out);
            }
            "b" | "strong" => {
                inner.style.set(css::Layer::Bold, true);
                self.inline(el, inner, out);
            }
            "i" | "em" | "cite" | "var" | "dfn" => {
                inner.style.set(css::Layer::Italic, true);
                self.inline(el, inner, out);
            }
            "u" | "ins" => {
                inner.style.set(css::Layer::Underline, true);
                self.inline(el, inner, out);
            }
            "s" | "strike" | "del" => {
                inner.style.set(css::Layer::Strike, true);
                self.inline(el, inner, out);
            }
            "mark" => {
                inner.style.set(css::Layer::Highlight, true);
                self.inline(el, inner, out);
            }
            "sup" => {
                inner.style.set(css::Layer::Sup, true);
                self.inline(el, inner, out);
            }
            "sub" => {
                inner.style.set(css::Layer::Sub, true);
                self.inline(el, inner, out);
            }
            "code" | "tt" | "kbd" | "samp" => {
                inner.style.mono = true;
                self.inline(el, inner, out);
            }
            "pre" => {
                let mut lang = language(el);
                // A `math` block is a formula in Markdown.
                if lang.eq_ignore_ascii_case("math") {
                    lang.clear();
                }
                out.push(Item::Block(Block::Code(lang, raw::pre_text(el))));
            }
            "div" => self.div(el, style_of(el), inner, out),
            "center" => {
                let mut decls = style_of(el);
                decls.push(Decl {
                    prop: "text-align".into(),
                    value: "center".into(),
                });
                self.div(el, decls, inner, out);
            }
            "p" => {
                let element = el.value();
                if self.keep && !(blank(element.attr("class")) && blank(style_attr(element))) {
                    out.push(Item::Block(Block::Raw(raw::block(el))));
                } else {
                    self.boundary(el, inner, out);
                }
            }
            "ul" | "ol" => self.list(el, inner, out),
            "table" => self.table(el, inner, out),
            "hr" => out.push(Item::Block(Block::Rule)),
            "blockquote" | "dd" => self.quote(el, inner, 1, out),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                // As the bold sized span of a heading: outside a heading line the size is
                // written as a raw size.
                let level = usize::from(name.as_bytes()[1] - b'0');
                let decls = [
                    Decl {
                        prop: "font-weight".into(),
                        value: "bold".into(),
                    },
                    Decl {
                        prop: "font-size".into(),
                        value: format!("{}pt", styles::HEADING_SIZES[level - 1]),
                    },
                ];
                css::apply(&mut inner.style, &decls, &mut self.wrappers);
                self.boundary(el, inner, out);
            }
            "dt" => {
                inner.style.set(css::Layer::Bold, true);
                self.boundary(el, inner, out);
            }
            _ if html::is_block_element(name) => self.boundary(el, inner, out),
            _ => self.inline(el, inner, out),
        }
    }

    /// An inline element: its style applies to its content; without content it leaves its style
    /// (an empty heading).
    fn inline(&mut self, el: ElementRef<'a>, mut ctx: Ctx, out: &mut Vec<Item>) {
        css::apply(&mut ctx.style, &style_of(el), &mut self.wrappers);
        let before = out.len();
        self.children(el, &ctx, out);
        if out.len() == before && ctx.style.size.is_some() {
            out.push(atom(AtomKind::Empty, &ctx.style));
        }
    }

    /// A block element without a Markdown form: its content between boundaries (paragraph
    /// boundaries for elements with vertical margins, line ends for the others).
    fn boundary(&mut self, el: ElementRef<'a>, mut ctx: Ctx, out: &mut Vec<Item>) {
        css::apply(&mut ctx.style, &text_style_of(el), &mut self.wrappers);
        let paragraph = has_margins(el.value().name());
        let edge = || {
            if paragraph {
                Item::Boundary
            } else {
                Item::LineEnd
            }
        };
        out.push(edge());
        let before = out.len();
        self.children(el, &ctx, out);
        if out.len() == before && ctx.style.size.is_some() {
            out.push(atom(AtomKind::Empty, &ctx.style));
        }
        out.push(edge());
    }

    /// The content of an element as a container of its own.
    fn container(&mut self, el: ElementRef<'a>, ctx: &Ctx, skip: usize) -> Vec<Item> {
        let mut items = Vec::new();
        for child in el.children().skip(skip) {
            self.child(child.value(), ElementRef::wrap(child), ctx, &mut items);
        }
        items
    }

    fn diagram_source(&self, src: &str) -> Option<String> {
        let source = refs::attachment_id(src)
            .and_then(|id| self.sources.get(&id).cloned())
            .or_else(|| mermaid::decode_mermaid_ink(src))?;
        let source = mermaid::normalize(&source);
        (!source.is_empty()).then_some(source)
    }

    fn image(&mut self, el: ElementRef<'a>, ctx: &Ctx, out: &mut Vec<Item>) {
        let element = el.value();
        let src = url_of(element.attr("src").unwrap_or(""));
        if !ctx.cell
            && ctx.style.link.is_none()
            && let Some(source) = self.diagram_source(&src)
        {
            out.push(Item::Block(Block::Diagram(source)));
            return;
        }
        let alt = raw::collapse(element.attr("alt").unwrap_or(""));
        if html::dangerous_url(&src) {
            // The conversion back shows only the alternative text.
            if !alt.is_empty() {
                out.push(atom(AtomKind::Text(alt), &ctx.style));
            }
            return;
        }
        // Judged by the attributes the sanitizer keeps: kept raw, the image reads back with those.
        let raw = raw::inline(el);
        let kept: Vec<String> = match html::tokens(&raw).next() {
            Some(html::Token::Tag(tag)) => tag
                .attrs
                .iter()
                .map(|a| a.name.to_ascii_lowercase())
                .collect(),
            _ => Vec::new(),
        };
        let plain = element
            .attrs()
            .filter(|(name, _)| kept.iter().any(|k| k == name))
            .all(|(name, value)| match name {
                "src" | "title" | "alt" | "class" | "data-lastmodified" => true,
                "style" => {
                    let decls = css::declarations(value);
                    decls.is_empty()
                        || matches!(decls.as_slice(), [d] if d.prop == "max-width" && css::norm(&d.value) == "650px")
                }
                _ => false,
            });
        if plain || !self.keep {
            let title = raw::collapse(element.attr("title").unwrap_or(""));
            out.push(atom(
                AtomKind::Image {
                    url: src,
                    title,
                    alt,
                },
                &ctx.style,
            ));
        } else {
            out.push(atom(AtomKind::Html(raw), &ctx.style));
        }
    }

    fn anchor(&mut self, el: ElementRef<'a>, mut ctx: Ctx, out: &mut Vec<Item>) {
        let element = el.value();
        let href = element.attr("href").map(url_of);
        let Some(href) = href.filter(|h| !html::dangerous_url(h) && ctx.style.link.is_none())
        else {
            return self.inline(el, ctx, out);
        };
        let label = raw::text(el);
        let work_item = work_item_of(&href);
        if let Some(id) = &work_item
            && label.is_empty()
            // Images, formulas and Polarion links write something without text.
            && !el.descendants().any(|n| {
                ElementRef::wrap(n).is_some_and(|e| {
                    e.value().name() == "img"
                        || has_class(e, styles::FORMULA_CLASS)
                        || has_class(e, styles::WORK_ITEM_CLASS)
                })
            })
        {
            out.push(atom(
                AtomKind::Id {
                    id: id.clone(),
                    fallback: IdFallback::Link {
                        url: href,
                        title: raw::collapse(element.attr("title").unwrap_or("")),
                    },
                },
                &ctx.style,
            ));
            return;
        }
        let title = raw::collapse(element.attr("title").unwrap_or(""));
        self.links.push(LinkInfo {
            url: href,
            title,
            work_item,
        });
        ctx.style.set_link(self.links.len() - 1);
        self.inline(el, ctx, out);
    }

    /// A Polarion link span (work items, pages, ...).
    fn link_span(&mut self, el: ElementRef<'a>, ctx: &Ctx, out: &mut Vec<Item>) {
        let element = el.value();
        let id = element.attr("data-item-id").map_or("", str::trim);
        let long = element.attr("data-type") == Some(styles::WORK_ITEM_TYPE)
            && element.attr("data-option-id") == Some(styles::WORK_ITEM_OPTION)
            && blank(element.attr("data-scope"))
            && blank(element.attr("data-revision"))
            && is_id(id);
        // The span is a widget that Polarion shows from its attributes: written empty, content
        // inside it (a label rendered into it) is not shown.
        let raw = raw::empty(el);
        let kind = if long {
            AtomKind::Id {
                id: id.to_owned(),
                fallback: IdFallback::Span(raw),
            }
        } else if self.keep {
            AtomKind::Html(raw)
        } else if ctx.style.link.is_none()
            && let Some(url) = self.url_link(el)
        {
            // A link to a URL: a link with the URL as its text.
            let mut style = ctx.style.clone();
            style.set_link(url);
            let url = self.links[url].url.clone();
            out.push(atom(AtomKind::Text(url), &style));
            return;
        } else {
            // The id; for links to other things (pages, ...) their name, label or URL.
            let text = [
                Some(id),
                element.attr("data-item-name"),
                element.attr("data-custom-label"),
                element.attr("data-url"),
            ]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|t| !t.is_empty())
            .unwrap_or_default();
            AtomKind::Text(html::xml_safe(text).into_owned())
        };
        out.push(atom(kind, &ctx.style));
    }

    /// The link of a link span that names nothing but a URL (`data-url`), outside other links,
    /// for drop mode (an index into the links).
    fn url_link(&mut self, el: ElementRef<'a>) -> Option<usize> {
        let element = el.value();
        let names = ["data-item-id", "data-item-name", "data-custom-label"]
            .iter()
            .any(|name| !blank(element.attr(name)));
        let url = url_of(element.attr("data-url")?);
        if names || url.is_empty() || html::dangerous_url(&url) {
            return None;
        }
        self.links.push(LinkInfo {
            work_item: work_item_of(&url),
            url,
            title: String::new(),
        });
        Some(self.links.len() - 1)
    }

    fn formula(&mut self, el: ElementRef<'a>, ctx: &Ctx, out: &mut Vec<Item>) {
        let element = el.value();
        let source = raw::unix_newlines(&html::xml_safe(element.attr("data-source").unwrap_or("")))
            .into_owned();
        let display = element
            .attr("data-inline")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("false"));
        out.push(atom(
            AtomKind::Math {
                source,
                display,
                raw: raw::empty(el),
            },
            &ctx.style,
        ));
        // Content inside the formula span (a rendering of it) is not shown.
    }

    fn quote(&mut self, el: ElementRef<'a>, mut ctx: Ctx, depth: usize, out: &mut Vec<Item>) {
        if ctx.blocks + depth > MAX_BLOCKS {
            return self.boundary(el, ctx, out);
        }
        if el.value().name() != "div" {
            css::apply(&mut ctx.style, &text_style_of(el), &mut self.wrappers);
        }
        ctx.blocks += depth;
        ctx.in_li = false;
        let items = self.container(el, &ctx, 0);
        out.push(Item::Block(Block::Quote(
            depth,
            Container {
                items,
                in_li: false,
            },
        )));
    }

    /// A `div` (or an element shown as one) with these declarations.
    fn div(&mut self, el: ElementRef<'a>, decls: Vec<Decl>, mut ctx: Ctx, out: &mut Vec<Item>) {
        if decls.is_empty() {
            return self.boundary(el, ctx, out);
        }
        if ctx.blocks < MAX_BLOCKS
            && let Some((alert, title, skip)) = alert_of(el, &decls)
        {
            ctx.blocks += 1;
            ctx.in_li = false;
            let items = self.container(el, &ctx, skip);
            out.push(Item::Block(Block::Alert(
                alert,
                title,
                Container {
                    items,
                    in_li: false,
                },
            )));
            return;
        }
        if let Some(depth) = quote_depth(&decls)
            && ctx.blocks + depth <= MAX_BLOCKS
        {
            if let Some(code) = older_code(el) {
                out.push(Item::Block(Block::Code(String::new(), code)));
                return;
            }
            return self.quote(el, ctx, depth, out);
        }
        // A frame shows around a code block; hidden, the code block is not shown.
        let framed = is_hidden(el)
            || decls.iter().any(|d| {
                d.prop.starts_with("margin")
                    || d.prop.starts_with("border")
                    || d.prop.starts_with("padding")
            });
        if !framed && only_pre(el) {
            // A highlighter's wrapper: only the code block counts.
            return self.children(el, &ctx, out);
        }
        if self.keep {
            let mut items = self.container(el, &ctx, 0);
            // Kept around a code block only, the div would read back as a highlighter's wrapper.
            if !framed && let Some(code) = lone_code(&mut items) {
                out.push(Item::Block(code));
                return;
            }
            let open = if el.value().name() == "div" {
                raw::start_tag(el)
            } else {
                raw::div_tag(&decls)
            };
            out.push(Item::Block(Block::RawWrap(
                open,
                Container {
                    items,
                    in_li: ctx.in_li,
                },
            )));
        } else {
            self.boundary(el, ctx, out);
        }
    }

    fn list(&mut self, el: ElementRef<'a>, mut ctx: Ctx, out: &mut Vec<Item>) {
        if ctx.blocks + 2 > MAX_BLOCKS {
            return self.boundary(el, ctx, out);
        }
        let element = el.value();
        let ordered = element.name() == "ol";
        let start = element
            .attr("start")
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|&s| s <= 999_999_999)
            .unwrap_or(1);
        css::apply(&mut ctx.style, &text_style_of(el), &mut self.wrappers);
        ctx.blocks += 2;
        ctx.in_li = true;
        let mut items: Vec<(Option<bool>, Container)> = Vec::new();
        self.list_items(el, &ctx, &mut items);
        // A list without items shows nothing.
        if items.is_empty() {
            return;
        }
        out.push(Item::Block(Block::List {
            ordered,
            start,
            items,
        }));
    }

    /// The items of a list element. An element between the list and its items (an `<a>` the HTML
    /// parser put there) passes its style on; other content continues the last item.
    fn list_items(
        &mut self,
        el: ElementRef<'a>,
        ctx: &Ctx,
        items: &mut Vec<(Option<bool>, Container)>,
    ) {
        for child in el.children() {
            let Some(child_el) = ElementRef::wrap(child) else {
                match child.value() {
                    Node::Text(t) if !t.trim().is_empty() => {
                        let mut stray = Vec::new();
                        self.child(child.value(), None, ctx, &mut stray);
                        continue_last(items, stray, self.keep);
                    }
                    _ => {}
                }
                continue;
            };
            let name = child_el.value().name();
            if name == "li" {
                // A hidden item is not shown (its style cannot be written on a Markdown item).
                if is_hidden(child_el) {
                    continue;
                }
                let mut item_ctx = ctx.clone();
                item_ctx.depth += 1;
                css::apply(
                    &mut item_ctx.style,
                    &text_style_of(child_el),
                    &mut self.wrappers,
                );
                let mut content = self.container(child_el, &item_ctx, 0);
                let task = task_prefix(&mut content, self.keep);
                items.push((
                    task,
                    Container {
                        items: content,
                        in_li: true,
                    },
                ));
            } else if ctx.depth < MAX_DEPTH
                && !raw::is_dropped(name)
                && child_el.child_elements().any(|c| c.value().name() == "li")
            {
                let mut inner = ctx.clone();
                inner.depth += 1;
                if name == "a"
                    && inner.style.link.is_none()
                    && let Some(href) = child_el.value().attr("href").map(url_of)
                    && !html::dangerous_url(&href)
                {
                    self.links.push(LinkInfo {
                        url: href,
                        title: raw::collapse(child_el.value().attr("title").unwrap_or("")),
                        work_item: None,
                    });
                    inner.style.set_link(self.links.len() - 1);
                }
                css::apply(&mut inner.style, &style_of(child_el), &mut self.wrappers);
                self.list_items(child_el, &inner, items);
            } else {
                let mut stray = Vec::new();
                self.element(child_el, ctx, &mut stray);
                continue_last(items, stray, self.keep);
            }
        }
    }

    fn table(&mut self, el: ElementRef<'a>, ctx: Ctx, out: &mut Vec<Item>) {
        let mut standard = table_style_ok(el);
        let mut rows: Vec<ElementRef<'a>> = Vec::new();
        for child in el.children() {
            match child.value() {
                Node::Element(e) => match e.name() {
                    "tr" => rows.extend(ElementRef::wrap(child)),
                    // Left out of raw HTML and the conversion back alike.
                    name if raw::is_dropped(name) => {}
                    "thead" | "tbody" | "tfoot" => {
                        standard &= blank(style_attr(e));
                        for row in child.children() {
                            match row.value() {
                                Node::Element(r) if r.name() == "tr" => {
                                    rows.extend(ElementRef::wrap(row));
                                }
                                Node::Element(r) if raw::is_dropped(r.name()) => {}
                                Node::Text(t) if t.trim().is_empty() => {}
                                Node::Comment(_) => {}
                                _ => standard = false,
                            }
                        }
                    }
                    _ => standard = false,
                },
                Node::Text(t) if t.trim().is_empty() => {}
                Node::Comment(_) => {}
                _ => standard = false,
            }
        }
        if rows.is_empty() {
            return;
        }
        let mut cell_ctx = ctx.clone();
        // Formatting around a table reaches its cells (raw table HTML cannot carry it: kept raw,
        // the cells are not walked). A size there is no heading size.
        cell_ctx.style.size = None;
        cell_ctx.cell = true;
        cell_ctx.blocks += 3;
        cell_ctx.in_li = false;
        let mut grid: Vec<Vec<Cell>> = Vec::with_capacity(rows.len());
        let mut sources: Vec<ElementRef<'a>> = Vec::new();
        for row in &rows {
            standard &= blank(style_attr(row.value()));
            let mut cells = Vec::new();
            for child in row.children() {
                match child.value() {
                    Node::Element(c) if matches!(c.name(), "th" | "td") => {
                        let Some(cell) = ElementRef::wrap(child) else {
                            continue;
                        };
                        let span = |attr: &str| {
                            c.attr(attr)
                                .and_then(|s| s.trim().parse::<usize>().ok())
                                .map_or(1, |s| s.clamp(1, MAX_SPAN))
                        };
                        let header = c.name() == "th";
                        let align = cell_style(cell, header);
                        sources.push(cell);
                        cells.push(Cell {
                            header,
                            colspan: span("colspan"),
                            rowspan: span("rowspan"),
                            align,
                            items: Vec::new(),
                        });
                    }
                    Node::Element(c) if raw::is_dropped(c.name()) => {}
                    Node::Text(t) if t.trim().is_empty() => {}
                    Node::Comment(_) => {}
                    _ => standard = false,
                }
            }
            grid.push(cells);
        }
        // A table without cells shows nothing.
        if grid.iter().all(Vec::is_empty) {
            return;
        }
        let width = grid[0].len();
        let shape = standard
            && width > 0
            && grid[0].iter().all(|c| c.header)
            && grid[1..].iter().flatten().all(|c| !c.header)
            && grid.iter().all(|row| row.len() <= width)
            && grid
                .iter()
                .flatten()
                .all(|c| c.colspan == 1 && c.rowspan == 1 && c.align.is_some())
            && grid[1..].iter().all(|row| {
                row.iter()
                    .zip(&grid[0])
                    .all(|(cell, head)| cell.align == head.align)
            });
        // Kept as raw HTML, the cells' content need not be walked (nested tables would be walked
        // once per level otherwise).
        if !shape && self.keep {
            out.push(Item::Block(Block::Raw(raw::block(el))));
            return;
        }
        for (cell, source) in grid.iter_mut().flatten().zip(sources) {
            cell.items = self.container(source, &cell_ctx, 0);
        }
        let gfm = shape
            && grid
                .iter()
                .flatten()
                .all(|c| c.items.iter().all(|item| matches!(item, Item::Atom(_))));
        if gfm {
            let aligns = grid[0]
                .iter()
                .map(|c| TableAlignment::from(c.align.unwrap_or(Align::Left)))
                .collect();
            let rows = grid
                .into_iter()
                .map(|row| {
                    let mut cells: Vec<Container> = row
                        .into_iter()
                        .map(|c| Container {
                            items: c.items,
                            in_li: false,
                        })
                        .collect();
                    cells.resize_with(width, Container::default);
                    cells
                })
                .collect();
            out.push(Item::Block(Block::Table { aligns, rows }));
        } else if self.keep {
            out.push(Item::Block(Block::Raw(raw::block(el))));
        } else {
            out.push(Item::Block(flat_table(grid)));
        }
    }
}

/// Content of a list outside its items: it continues the last item (or makes one). The item's
/// content may then start with a task marker, as it reads back.
fn continue_last(items: &mut Vec<(Option<bool>, Container)>, stray: Vec<Item>, keep: bool) {
    if stray.is_empty() {
        return;
    }
    if items.is_empty() {
        items.push((
            None,
            Container {
                items: Vec::new(),
                in_li: true,
            },
        ));
    }
    if let Some((task, last)) = items.last_mut() {
        // Inline content after the item's own shows on a line of its own.
        let after_text = last
            .items
            .iter()
            .rev()
            .find(|item| !matches!(item, Item::Atom(a) if is_blank(a)))
            .is_some_and(|item| matches!(item, Item::Atom(_)));
        if after_text && matches!(stray.first(), Some(Item::Atom(_))) {
            last.items.push(Item::LineEnd);
        }
        last.items.extend(stray);
        if task.is_none() {
            *task = task_prefix(&mut last.items, keep);
        }
    }
}

/// Whether an atom shows nothing: whitespace, an empty element.
fn is_blank(atom: &Atom) -> bool {
    match &atom.kind {
        AtomKind::Text(t) => t.chars().all(|c| c.is_ascii_whitespace()),
        AtomKind::Empty => true,
        _ => false,
    }
}

/// A table cell as walked.
struct Cell {
    header: bool,
    colspan: usize,
    rowspan: usize,
    /// The alignment, `None` when the cell's style is not the standard one.
    align: Option<Align>,
    items: Vec<Item>,
}

/// Whether a table uses only the standard table style (and no other presentational attribute).
fn table_style_ok(el: ElementRef<'_>) -> bool {
    let element = el.value();
    if element
        .attrs()
        .any(|(name, _)| !matches!(name, "id" | "class" | "style"))
    {
        return false;
    }
    style_of(el).iter().all(|d| {
        let v = css::norm(&d.value);
        match d.prop.as_str() {
            "width" => v == "100%",
            "max-width" => v == "1280px",
            "margin-left" => v == "0",
            "margin-right" => v == "auto",
            "border" => v == "1px solid #cccccc",
            "empty-cells" => v == "show",
            "border-collapse" => v == "collapse",
            _ => false,
        }
    })
}

/// The alignment of a cell whose style is the standard one (and with no presentational
/// attribute), `None` otherwise.
fn cell_style(el: ElementRef<'_>, header: bool) -> Option<Align> {
    let element = el.value();
    if element.attrs().any(|(name, value)| match name {
        "id" | "class" | "style" | "scope" => false,
        "colspan" | "rowspan" => value.trim() != "1",
        _ => true,
    }) {
        return None;
    }
    let mut align = Align::Left;
    for d in style_of(el) {
        let v = css::norm(&d.value);
        let ok = match d.prop.as_str() {
            "text-align" => match Align::from_css(&v) {
                Some(a) => {
                    align = a;
                    true
                }
                None => false,
            },
            "font-weight" => header && v == "bold",
            "background-color" => header && v == "#f0f0f0",
            "vertical-align" => v == "top",
            "height" => v == "12px",
            "border" | "border-top" | "border-bottom" | "border-left" | "border-right" => {
                v == "1px solid #cccccc"
            }
            "padding" => v == "5px",
            _ => false,
        };
        if !ok {
            return None;
        }
    }
    Some(align)
}

/// Cells a table may get by expanding merged cells; beyond that merged cells count as one.
const MAX_EXPANDED: usize = 100_000;

/// A table Markdown cannot hold, for drop mode: merged cells expanded into empty ones, block
/// content flattened to lines, the first row as the header.
fn flat_table(mut grid: Vec<Vec<Cell>>) -> Block {
    let widest = grid
        .iter()
        .map(|row| row.iter().map(|c| c.colspan).sum::<usize>())
        .max()
        .unwrap_or(0);
    let tallest = grid.iter().flatten().map(|c| c.rowspan).max().unwrap_or(1);
    if widest.saturating_mul(grid.len().saturating_add(tallest)) > MAX_EXPANDED {
        for cell in grid.iter_mut().flatten() {
            cell.colspan = 1;
            cell.rowspan = 1;
        }
    }
    let mut rows: Vec<Vec<Option<Container>>> = Vec::with_capacity(grid.len());
    let mut aligns = Vec::new();
    // Columns of each row taken by cells spanning down from rows above.
    let mut taken: Vec<Vec<bool>> = vec![Vec::new(); grid.len()];
    for (r, row) in grid.into_iter().enumerate() {
        let mut cells: Vec<Option<Container>> = Vec::new();
        let mut col = 0;
        for cell in row {
            while taken[r].get(col).copied().unwrap_or(false) {
                if cells.len() <= col {
                    cells.resize_with(col + 1, || None);
                }
                col += 1;
            }
            if r == 0 {
                aligns.resize(col, TableAlignment::None);
                aligns.push(TableAlignment::from(cell.align.unwrap_or(Align::Left)));
                aligns.resize(col + cell.colspan, TableAlignment::None);
            }
            if cells.len() < col {
                cells.resize_with(col, || None);
            }
            cells.push(Some(Container {
                items: flatten(cell.items),
                in_li: false,
            }));
            cells.resize_with(col + cell.colspan, || None);
            for below in taken.iter_mut().skip(r + 1).take(cell.rowspan - 1) {
                if below.len() < col + cell.colspan {
                    below.resize(col + cell.colspan, false);
                }
                for slot in &mut below[col..col + cell.colspan] {
                    *slot = true;
                }
            }
            col += cell.colspan;
        }
        rows.push(cells);
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    aligns.resize(width, TableAlignment::None);
    let rows = rows
        .into_iter()
        .map(|row| {
            let mut cells: Vec<Container> =
                row.into_iter().map(Option::unwrap_or_default).collect();
            cells.resize_with(width, Container::default);
            cells
        })
        .collect();
    Block::Table { aligns, rows }
}

/// Block content as inline content: blocks become lines (a `<br>` between them).
fn flatten(items: Vec<Item>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    let mut pending_break = false;
    let push_break = |out: &mut Vec<Item>, pending: &mut bool| {
        if *pending && !out.is_empty() {
            out.push(atom(AtomKind::Br, &Style::default()));
        }
        *pending = false;
    };
    for item in items {
        match item {
            Item::Atom(a) => {
                push_break(&mut out, &mut pending_break);
                out.push(Item::Atom(a));
            }
            Item::Boundary | Item::LineEnd => pending_break = true,
            Item::Block(block) => {
                pending_break = true;
                let parts: Vec<Vec<Item>> = match block {
                    Block::Quote(_, c) | Block::Alert(_, _, c) | Block::RawWrap(_, c) => {
                        vec![flatten(c.items)]
                    }
                    Block::Code(_, text) | Block::Diagram(text) => text
                        .split('\n')
                        .map(|line| vec![atom(AtomKind::Text(line.to_owned()), &Style::default())])
                        .collect(),
                    Block::List { items, .. } => {
                        items.into_iter().map(|(_, c)| flatten(c.items)).collect()
                    }
                    Block::Table { rows, .. } => rows
                        .into_iter()
                        .map(|row| {
                            let mut line = Vec::new();
                            for (i, cell) in row.into_iter().enumerate() {
                                if i > 0 {
                                    line.push(atom(AtomKind::Text(" ".into()), &Style::default()));
                                }
                                line.extend(flatten(cell.items).into_iter().map(
                                    |item| match item {
                                        Item::Atom(Atom {
                                            kind: AtomKind::Br,
                                            style,
                                        }) => Item::Atom(Atom {
                                            kind: AtomKind::Text(" ".into()),
                                            style,
                                        }),
                                        other => other,
                                    },
                                ));
                            }
                            line
                        })
                        .collect(),
                    Block::Raw(_) | Block::Rule => Vec::new(),
                };
                for part in parts {
                    if part.is_empty() {
                        continue;
                    }
                    push_break(&mut out, &mut pending_break);
                    out.extend(part);
                    pending_break = true;
                }
            }
        }
    }
    out
}

/// Takes the task marker (`☐ `, `☑ `, `☒ `, the space a no-break one too) off the first text of a
/// list item. Without the space after it the symbol is text (a marker kept before a block).
/// Breaks before it count only when they are kept (`keep`; drop mode removes them).
fn task_prefix(items: &mut [Item], keep: bool) -> Option<bool> {
    for item in items.iter_mut() {
        let atom = match item {
            Item::Atom(atom) => atom,
            // Nothing is written for a boundary before any content.
            Item::Boundary | Item::LineEnd => continue,
            Item::Block(_) => return None,
        };
        match &mut atom.kind {
            AtomKind::Text(text) if text.chars().all(|c| c.is_ascii_whitespace()) => continue,
            AtomKind::Empty => continue,
            AtomKind::Br | AtomKind::RawBr if !keep => continue,
            AtomKind::Text(text) => {
                let trimmed = text.trim_start_matches(|c: char| c.is_ascii_whitespace());
                let markers = [
                    (styles::TASK_OPEN, false),
                    (styles::TASK_DONE, true),
                    (styles::TASK_CROSSED, true),
                ];
                for (marker, done) in markers {
                    // The marker's space (any whitespace, it collapses to one), or a no-break
                    // space.
                    let symbol = marker.trim_end();
                    if let Some(rest) = trimmed.strip_prefix(symbol)
                        && let Some(rest) =
                            rest.strip_prefix(|c: char| c == '\u{a0}' || c.is_ascii_whitespace())
                    {
                        *text = rest.to_owned();
                        return Some(done);
                    }
                }
                return None;
            }
            _ => return None,
        }
    }
    None
}

/// The forward direction's alert: `<div style="margin-left: 40px; border-left: 3px solid C;
/// padding-left: 10px;"><span style="font-weight: bold; color: C;">Title</span><br/>…</div>`.
/// Returns the alert, its title when it is not the default one, and how many children the title
/// takes (the span and the `<br>` after it).
fn alert_of(el: ElementRef<'_>, decls: &[Decl]) -> Option<(AlertType, Option<String>, usize)> {
    if decls.len() != 3 {
        return None;
    }
    let mut color = None;
    for d in decls {
        let v = css::norm(&d.value);
        match d.prop.as_str() {
            "margin-left" if v == "40px" => {}
            "padding-left" if v == "10px" => {}
            "border-left" => color = v.strip_prefix("3px solid ").map(str::to_owned),
            _ => return None,
        }
    }
    let color = color?;
    let alert = styles::ALERTS
        .into_iter()
        .find(|a| a.color.eq_ignore_ascii_case(&color))?;
    let mut title = None;
    let mut taken = 0;
    for (i, child) in el.children().enumerate() {
        match child.value() {
            Node::Text(t) if t.trim().is_empty() => {}
            Node::Element(e) if title.is_none() && e.name() == "span" => {
                let span = ElementRef::wrap(child)?;
                // The title is text (other content would not survive as a title).
                if span.children().any(|c| !matches!(c.value(), Node::Text(_))) {
                    return None;
                }
                let span_decls = style_of(span);
                let ok = span_decls.len() == 2
                    && span_decls.iter().all(|d| match d.prop.as_str() {
                        "font-weight" => css::norm(&d.value) == "bold",
                        "color" => d.value.eq_ignore_ascii_case(alert.color),
                        _ => false,
                    });
                if !ok {
                    return None;
                }
                title = Some(raw::text(span));
                taken = i + 1;
            }
            Node::Element(e) if title.is_some() && e.name() == "br" => {
                taken = i + 1;
                break;
            }
            _ => break,
        }
    }
    let title = title?;
    let title = (!title.is_empty() && title != alert.title).then_some(title);
    Some((alert.kind, title, taken))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walked(html: &str) -> Walked {
        walk(html, &ToMarkdownOptions::default(), &DiagramSources::new())
    }

    fn texts(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .filter_map(|item| match item {
                Item::Atom(Atom {
                    kind: AtomKind::Text(t),
                    ..
                }) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn styles_inherit_and_reset() {
        let w =
            walked("<b>a<span style=\"font-weight: normal;\">b</span><i>c</i></b><u><s>d</s></u>");
        let atoms: Vec<&Atom> = w
            .body
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Atom(a) => Some(a),
                _ => None,
            })
            .collect();
        assert_eq!(atoms.len(), 4);
        assert!(atoms[0].style.bold);
        assert!(!atoms[1].style.bold);
        assert!(atoms[2].style.bold && atoms[2].style.italic);
        assert!(atoms[3].style.underline && atoms[3].style.strike);
    }

    #[test]
    fn filtered_elements_and_comments_are_dropped() {
        let w = walked("a<script>x</script><!-- c --><style>y</style><iframe>z</iframe>b");
        assert_eq!(texts(&w.body.items), ["a", "b"]);
    }

    #[test]
    fn deep_nesting_becomes_text() {
        let html = "<div>".repeat(10_000) + "deep" + &"</div>".repeat(10_000);
        let w = walked(&html);
        let mut found = false;
        let mut stack: Vec<&[Item]> = vec![&w.body.items];
        while let Some(items) = stack.pop() {
            for item in items {
                match item {
                    Item::Atom(Atom {
                        kind: AtomKind::Text(t),
                        ..
                    }) if t.contains("deep") => found = true,
                    Item::Block(Block::Quote(_, c)) => stack.push(&c.items),
                    _ => {}
                }
            }
        }
        assert!(found);
    }

    #[test]
    fn work_item_urls() {
        assert_eq!(
            work_item_of("https://x/polarion/#/project/P/workitem?id=PROJ-7"),
            Some("PROJ-7".into())
        );
        assert_eq!(
            work_item_of("/polarion/#/project/P/workitem?id=PROJ-7&revision=1"),
            Some("PROJ-7".into())
        );
        assert_eq!(work_item_of("/workitem?id=proj-7"), None);
        assert_eq!(work_item_of("/x?id=PROJ-7"), None);
    }

    #[test]
    fn task_markers() {
        let mut items = vec![atom(AtomKind::Text(" ☑ done".into()), &Style::default())];
        assert_eq!(task_prefix(&mut items, true), Some(true));
        assert_eq!(texts(&items), ["done"]);
        let mut items = vec![atom(AtomKind::Text("☐ ".into()), &Style::default())];
        assert_eq!(task_prefix(&mut items, true), Some(false));
        let mut items = vec![atom(AtomKind::Text("☐x".into()), &Style::default())];
        assert_eq!(task_prefix(&mut items, true), None);
        let mut items = vec![atom(AtomKind::Text("☑".into()), &Style::default())];
        assert_eq!(task_prefix(&mut items, true), None);
        let mut items = vec![atom(AtomKind::Text("☐\u{a0}x".into()), &Style::default())];
        assert_eq!(task_prefix(&mut items, true), Some(false));
        assert_eq!(texts(&items), ["x"]);
    }

    #[test]
    fn flattened_tables_expand_merged_cells() {
        let options = ToMarkdownOptions {
            raw_html: RawHtml::Drop,
            ..ToMarkdownOptions::default()
        };
        let w = walk(
            "<table><tr><td colspan=\"2\">a</td><td rowspan=\"2\">b</td></tr><tr><td>c</td></tr></table>",
            &options,
            &DiagramSources::new(),
        );
        let Some(Item::Block(Block::Table { aligns, rows })) = w.body.items.first() else {
            panic!("{:?}", w.body.items);
        };
        assert_eq!(aligns.len(), 3);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.len() == 3));
        assert_eq!(texts(&rows[0][0].items), ["a"]);
        assert!(rows[0][1].items.is_empty());
        assert_eq!(texts(&rows[1][0].items), ["c"]);
        assert!(rows[1][2].items.is_empty());
    }
}
