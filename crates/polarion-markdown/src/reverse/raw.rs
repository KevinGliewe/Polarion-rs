//! Raw HTML written from the DOM (formatting Markdown cannot express, kept in keep mode) and the
//! text of subtrees.
//!
//! What is written passes the conversion back unchanged: comments, processing instructions and
//! the elements the forward sanitizer filters are left out, attribute values are escaped, void
//! elements are self-closed (`<img …/>`, as the forward direction writes them), the result is
//! sanitized (idempotent). Subtrees are visited iteratively, so any depth is fine.

use std::borrow::Cow;

use scraper::{ElementRef, Node};

use super::css::Decl;
use crate::html;

/// Elements without content (no end tag).
const VOID: [&str; 16] = [
    "area", "base", "br", "col", "embed", "frame", "hr", "img", "input", "keygen", "link", "meta",
    "param", "source", "track", "wbr",
];

pub(super) fn is_void(name: &str) -> bool {
    VOID.contains(&name)
}

/// Elements left out with their content: those the forward sanitizer filters, and `template`
/// (its content is never shown).
pub(super) fn is_dropped(name: &str) -> bool {
    html::is_filtered(name) || name.eq_ignore_ascii_case("template")
}

/// Line breaks as `\n`: a CR (from `&#13;`) or CRLF would read back from Markdown as a line
/// break anyway.
pub(super) fn unix_newlines(s: &str) -> Cow<'_, str> {
    if s.contains('\r') {
        Cow::Owned(s.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(s)
    }
}

/// The start tag of a `div` with these declarations (an element shown as such a `div`).
pub(super) fn div_tag(decls: &[Decl]) -> String {
    let style: Vec<String> = decls
        .iter()
        .map(|d| format!("{}: {};", d.prop, d.value))
        .collect();
    let mut out = String::from("<div style=\"");
    push_value(&mut out, &style.join(" "));
    out.push_str("\">");
    html::sanitize_raw_html(&out).into_owned()
}

/// What [`visit`] reports, in document order.
pub(super) enum Event<'a> {
    Open(ElementRef<'a>),
    Close(ElementRef<'a>),
    Text(&'a str),
}

/// The subtree of `root` (itself included) in document order, without comments, processing
/// instructions and filtered elements (with their content).
pub(super) fn visit<'a>(root: ElementRef<'a>, mut f: impl FnMut(Event<'a>)) {
    f(Event::Open(root));
    // Open nodes (the root first): the element, if reported (filtered elements and the
    // fragments holding template content are not).
    let mut stack = vec![(root.id(), Some(root))];
    let mut skipping = 0usize;
    for node in root.descendants().skip(1) {
        let parent = node.parent().map(|p| p.id());
        while let Some(&(top, reported)) = stack.last() {
            if Some(top) == parent {
                break;
            }
            stack.pop();
            match reported {
                Some(el) => f(Event::Close(el)),
                None => skipping -= 1,
            }
        }
        match node.value() {
            Node::Element(element) => {
                let Some(el) = ElementRef::wrap(node) else {
                    continue;
                };
                if skipping > 0 || is_dropped(element.name()) {
                    skipping += 1;
                    stack.push((node.id(), None));
                } else {
                    f(Event::Open(el));
                    stack.push((node.id(), Some(el)));
                }
            }
            Node::Fragment => {
                skipping += 1;
                stack.push((node.id(), None));
            }
            Node::Text(text) if skipping == 0 => f(Event::Text(text)),
            _ => {}
        }
    }
    while let Some((_, reported)) = stack.pop() {
        if let Some(el) = reported {
            f(Event::Close(el));
        }
    }
}

/// Whether an attribute name can be written as it is (anything else would change how the tag is
/// read).
fn plain_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_' || b == b':')
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
}

/// Text escaped as HTML parsers expect it back (`& < >` and U+00A0), characters XML does not
/// allow replaced; line breaks as `newline`.
fn push_text(out: &mut String, text: &str, newline: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '\n' => out.push_str(newline),
            '\r' => out.push_str("&#13;"),
            c if !html::xml_allows(c) => out.push(if c == '\u{c}' { ' ' } else { '\u{fffd}' }),
            c => out.push(c),
        }
    }
}

/// An attribute value escaped for double quotes, line breaks as `&#10;`.
fn push_value(out: &mut String, value: &str) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if !html::xml_allows(c) => out.push(if c == '\u{c}' { ' ' } else { '\u{fffd}' }),
            c => out.push(c),
        }
    }
}

fn push_start_tag(out: &mut String, el: ElementRef<'_>) {
    let element = el.value();
    out.push('<');
    out.push_str(element.name());
    for (name, value) in element.attrs() {
        if !plain_name(name) {
            continue;
        }
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        push_value(out, value);
        out.push('"');
    }
    if is_void(element.name()) {
        out.push('/');
    }
    out.push('>');
}

/// The start tag of an element, sanitized.
pub(super) fn start_tag(el: ElementRef<'_>) -> String {
    let mut out = String::new();
    push_start_tag(&mut out, el);
    html::sanitize_raw_html(&out).into_owned()
}

/// An element without its content (Polarion's link and formula spans are empty), sanitized.
pub(super) fn empty(el: ElementRef<'_>) -> String {
    let mut out = String::new();
    push_start_tag(&mut out, el);
    let name = el.value().name();
    if !is_void(name) {
        out.push_str("</");
        out.push_str(name);
        out.push('>');
    }
    html::sanitize_raw_html(&out).into_owned()
}

/// `root` serialized, then reparsed and serialized again until that changes nothing: a DOM the
/// HTML parser built from misnested tags (a link inside a link, a paragraph inside inline
/// formatting) is not always what parsing its serialization builds.
fn serialize(root: ElementRef<'_>, newline: &str) -> String {
    let mut html = html::sanitize_raw_html(&write(root, newline, false)).into_owned();
    for _ in 0..4 {
        let doc = scraper::Html::parse_fragment(&html);
        let written = write(doc.root_element(), newline, true);
        let inner = written
            .strip_prefix("<html>")
            .and_then(|s| s.strip_suffix("</html>"))
            .unwrap_or(&written);
        let again = html::sanitize_raw_html(inner);
        if again == html {
            break;
        }
        html = again.into_owned();
    }
    html
}

/// Elements nested deeper than this below the root of raw HTML are written as their text (with
/// a space for a `<br>` or a block-level element): the conversion back leaves out elements nested
/// deeper than its own limit, so the raw HTML stays within it.
const MAX_RAW_DEPTH: usize = 256;

/// `wrapper`: the root is the `<html>` element parsing a fragment puts around it (no level).
fn write(root: ElementRef<'_>, newline: &str, wrapper: bool) -> String {
    let mut out = String::new();
    let mut in_pre = 0usize;
    // Right after a `<pre>` start tag, where HTML parsers drop a line break.
    let mut pre_start = false;
    let mut depth = 0usize;
    let limit = MAX_RAW_DEPTH + usize::from(wrapper);
    visit(root, |event| match event {
        Event::Open(el) => {
            let name = el.value().name();
            depth += 1;
            if depth > limit {
                if name == "br" || html::is_block_element(name) {
                    out.push(' ');
                }
                return;
            }
            pre_start = matches!(name, "pre" | "listing");
            if pre_start {
                in_pre += 1;
            }
            push_start_tag(&mut out, el);
        }
        Event::Close(el) => {
            depth -= 1;
            if depth >= limit {
                return;
            }
            pre_start = false;
            let name = el.value().name();
            if matches!(name, "pre" | "listing") {
                in_pre -= 1;
            }
            if !is_void(name) {
                out.push_str("</");
                out.push_str(name);
                out.push('>');
            }
        }
        Event::Text(text) => {
            if std::mem::take(&mut pre_start) && text.starts_with('\n') {
                out.push_str("&#10;");
            }
            push_text(&mut out, text, if in_pre > 0 { "&#10;" } else { newline });
        }
    });
    out
}

/// An element as raw inline HTML, on one line (line breaks in text as spaces, in values and
/// `pre` text as `&#10;`).
pub(super) fn inline(el: ElementRef<'_>) -> String {
    serialize(el, " ")
}

/// An element as a raw HTML block: without lines that are only whitespace (a blank line would end
/// the block), line breaks in values and `pre` text as `&#10;`.
pub(super) fn block(el: ElementRef<'_>) -> String {
    let html = serialize(el, "\n");
    if !html.contains('\n') {
        return html;
    }
    html.split('\n')
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text of a `pre`: `<br>` as a line break, a line break before a nested block element.
pub(super) fn pre_text(el: ElementRef<'_>) -> String {
    let mut out = String::new();
    visit(el, |event| match event {
        Event::Open(e) if e != el => {
            let name = e.value().name();
            let block = html::is_block_element(name) && !out.is_empty() && !out.ends_with('\n');
            if name == "br" || block {
                out.push('\n');
            }
        }
        Event::Text(text) => out.push_str(text),
        _ => {}
    });
    unix_newlines(&html::xml_safe(&out)).into_owned()
}

/// The text of a monospace element older converters wrote as a code block (lines separated by
/// `<br>`), as a browser shows it: whitespace collapsed and trimmed per line, no-break spaces as
/// spaces, a last empty line dropped.
pub(super) fn code_lines(el: ElementRef<'_>) -> String {
    let mut raw = String::new();
    visit(el, |event| match event {
        Event::Open(e) if e.value().name() == "br" => raw.push('\n'),
        Event::Text(text) => {
            for c in text.chars() {
                raw.push(if c == '\n' || c == '\r' { ' ' } else { c });
            }
        }
        _ => {}
    });
    let mut lines: Vec<String> = raw
        .split('\n')
        .map(|line| {
            let mut out = String::new();
            let mut space = false;
            for c in line.trim_matches(|c: char| c.is_ascii_whitespace()).chars() {
                if c.is_ascii_whitespace() {
                    if !space {
                        out.push(' ');
                    }
                    space = true;
                } else {
                    out.push(if c == '\u{a0}' { ' ' } else { c });
                    space = false;
                }
            }
            out
        })
        .collect();
    if lines.len() > 1 && lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    html::xml_safe(&lines.join("\n")).into_owned()
}

/// The text of a subtree with whitespace collapsed and trimmed; `<br>` and block elements count
/// as spaces.
pub(super) fn text(el: ElementRef<'_>) -> String {
    let mut raw = String::new();
    visit(el, |event| match event {
        Event::Open(e) if e != el => {
            let name = e.value().name();
            if name == "br" || html::is_block_element(name) {
                raw.push(' ');
            }
        }
        Event::Text(text) => raw.push_str(text),
        _ => {}
    });
    collapse(&raw)
}

/// The text of a subtree with whitespace collapsed but not trimmed (a space at an edge stays
/// one); `<br>` and block elements count as spaces.
pub(super) fn flat_text(el: ElementRef<'_>) -> String {
    let mut raw = String::new();
    visit(el, |event| match event {
        Event::Open(e) if e != el => {
            let name = e.value().name();
            if name == "br" || html::is_block_element(name) {
                raw.push(' ');
            }
        }
        Event::Text(text) => raw.push_str(text),
        _ => {}
    });
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c.is_ascii_whitespace() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    html::xml_safe(&out).into_owned()
}

/// Runs of ASCII whitespace as one space, trimmed.
pub(super) fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s.split(|c: char| c.is_ascii_whitespace()) {
        if word.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    html::xml_safe(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use scraper::Html;

    use super::*;

    fn first(html: &str, f: impl Fn(ElementRef<'_>) -> String) -> String {
        let dom = Html::parse_fragment(html);
        let root = dom.root_element();
        let el = root.child_elements().next().unwrap();
        f(el)
    }

    #[test]
    fn inline_html_round_trips_clean_tags() {
        let span = r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-option-id="short"></span>"#;
        assert_eq!(first(span, inline), span);
        assert_eq!(
            first(
                "<span data-source=\"a\nb &amp; c\" class=\"polarion-rte-formula\"></span>",
                inline
            ),
            "<span data-source=\"a&#10;b &amp; c\" class=\"polarion-rte-formula\"></span>"
        );
        assert_eq!(
            first(
                r#"<img src="a.png" style="width: 100px;height: 74px;">"#,
                inline
            ),
            r#"<img src="a.png" style="width: 100px;height: 74px;"/>"#
        );
        // Comments and filtered elements are left out, dangerous attributes removed.
        assert_eq!(
            first(
                "<span title=\"t\">a<!-- c --><script>x</script>b</span>",
                inline
            ),
            "<span title=\"t\">ab</span>"
        );
        assert_eq!(
            first(r#"<img src="x" onerror="alert(1)">"#, inline),
            r#"<img src="x"/>"#
        );
    }

    #[test]
    fn blocks_have_no_blank_lines() {
        let table = "<table>\n  <tbody>\n\n    <tr><td>a\n\n b</td></tr>\n  </tbody>\n</table>";
        let html = first(table, block);
        assert!(!html.contains("\n\n"), "{html}");
        assert!(
            html.starts_with("<table>") && html.ends_with("</table>"),
            "{html}"
        );
        let pre = first("<div><pre>a\n\nb</pre></div>", block);
        assert_eq!(pre, "<div><pre>a&#10;&#10;b</pre></div>");
        // HTML parsers drop a line break right after `<pre>`: a leading one gets another.
        let pre = first("<div><pre>\n\nb</pre></div>", block);
        assert_eq!(pre, "<div><pre>&#10;&#10;b</pre></div>");
        assert_eq!(first(&pre, block), pre);
    }

    #[test]
    fn widget_spans_are_written_empty() {
        assert_eq!(
            first(
                "<span class=\"polarion-rte-formula\" data-source=\"x\"><b>inside</b></span>",
                empty
            ),
            "<span class=\"polarion-rte-formula\" data-source=\"x\"></span>"
        );
        assert_eq!(first("<img src=\"a.png\">", empty), "<img src=\"a.png\"/>");
    }

    #[test]
    fn texts() {
        assert_eq!(
            first("<pre>a<br>b<span>c</span>\n</pre>", pre_text),
            "a\nbc\n"
        );
        assert_eq!(
            first(
                "<span>line1<br/>&nbsp;&nbsp;line2  x<br/>line3<br/></span>",
                code_lines
            ),
            "line1\n  line2 x\nline3"
        );
        assert_eq!(first("<a> a <b>b</b><br>c </a>", text), "a b c");
    }

    #[test]
    fn deep_subtrees() {
        let html = "<span>".repeat(10_000) + "x" + &"</span>".repeat(10_000);
        assert_eq!(first(&html, text), "x");
        // Raw HTML nested deeper than its limit is text.
        assert_eq!(
            first(&html, inline).matches("<span>").count(),
            MAX_RAW_DEPTH
        );
    }
}
