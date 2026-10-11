//! The diagram source inside the SVG's `<metadata>`, the markup scanner this module needs, and the
//! well-formedness check of a rendered SVG ([`check_well_formed`]).
//!
//! [`embed_source`] writes
//! `<metadata><mermaid xmlns="urn:polarion-rs:mermaid" version="1">SOURCE</mermaid></metadata>`
//! right after the root start tag; [`source_of`] finds that element by string search, so it also
//! works when the rest of the SVG is not well-formed XML.

use std::borrow::Cow;

use super::NAMESPACE;
use crate::html::xml_allows;

/// The `version` attribute of the `<mermaid>` element: the embedding format. A new format needs a
/// new version and a new [`super::PROFILE`].
const VERSION: &str = "1";

/// The SVG with `source` embedded in a `<metadata>` element right after the root start tag (a
/// self-closing root `<svg …/>` becomes `<svg …>…</svg>`).
///
/// The source is escaped (`&`, `<`, `>`, and a carriage return as `&#13;`); characters XML 1.0 does
/// not allow (C0 controls other than tab and line breaks, U+FFFE, U+FFFF) become U+FFFD, so only a
/// source without them comes back unchanged from [`source_of`]. An input without a complete root
/// start tag is returned unchanged.
pub fn embed_source(svg: &str, source: &str) -> String {
    let Some(root) = Markup::new(svg).find_map(|item| match item {
        Item::Start(tag) => Some(tag),
        _ => None,
    }) else {
        return svg.to_owned();
    };
    if !root.closed {
        return svg.to_owned();
    }
    let metadata = format!(
        "<metadata><mermaid xmlns=\"{NAMESPACE}\" version=\"{VERSION}\">{}</mermaid></metadata>",
        escape_text(source)
    );
    let mut out = String::with_capacity(svg.len() + metadata.len() + root.name.len() + 3);
    if root.self_closing {
        // `<svg …/>`: the tag without its `/>`, then `>`.
        out.push_str(&svg[..root.end - 2]);
        out.push('>');
        out.push_str(&metadata);
        out.push_str("</");
        out.push_str(root.name);
        out.push('>');
    } else {
        out.push_str(&svg[..root.end]);
        out.push_str(&metadata);
    }
    out.push_str(&svg[root.end..]);
    out
}

/// The source embedded by [`embed_source`], if the SVG has one: the content of the first
/// `<mermaid xmlns="urn:polarion-rs:mermaid" version="1">` element, unescaped (`&lt;` `&gt;`
/// `&amp;` `&quot;` `&apos;` `&#N;` `&#xN;`, CDATA sections).
pub fn source_of(svg: &str) -> Option<String> {
    let mut from = 0;
    while let Some(found) = svg[from..].find("<mermaid") {
        let start = from + found;
        from = start + 1;
        let tag = parse_start_tag(svg, start);
        if tag.name != "mermaid"
            || !tag.closed
            || tag.attr(svg, "xmlns").as_deref() != Some(NAMESPACE)
            || tag.attr(svg, "version").as_deref() != Some(VERSION)
        {
            continue;
        }
        if tag.self_closing {
            return Some(String::new());
        }
        return element_text(svg, tag.end, "mermaid");
    }
    None
}

/// The text of an element from `from` (after its start tag) to its end tag `</name>`: character
/// data unescaped, CDATA sections as they are. `None` when the element holds other markup or does
/// not end.
fn element_text(svg: &str, from: usize, name: &str) -> Option<String> {
    let mut text = String::new();
    let mut rest = &svg[from..];
    loop {
        let lt = rest.find('<')?;
        text.push_str(&unescape(&rest[..lt]));
        rest = &rest[lt..];
        if let Some(cdata) = rest.strip_prefix("<![CDATA[") {
            let end = cdata.find("]]>")?;
            text.push_str(&cdata[..end]);
            rest = &cdata[end + 3..];
        } else {
            let after = rest.strip_prefix("</")?.strip_prefix(name)?;
            return after
                .trim_start_matches(is_xml_space)
                .starts_with('>')
                .then_some(text);
        }
    }
}

/// Text as XML character data: `&`, `<` and `>` escaped, a carriage return as `&#13;` (an XML
/// parser turns a literal one into a line feed), characters XML does not allow as U+FFFD.
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c if xml_allows(c) => out.push(c),
            _ => out.push('\u{FFFD}'),
        }
    }
    out
}

fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// Text or an attribute value with its character references resolved: `&lt;` `&gt;` `&amp;`
/// `&quot;` `&apos;`, `&#N;` and `&#xN;` (a reference to a character XML does not allow becomes
/// U+FFFD). Anything else, an unknown entity or a bare `&`, stays as it is.
pub(super) fn unescape(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        match reference(rest) {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// The character the reference at the start of `s` (which starts with `&`) stands for, and the
/// reference's length; `None` unless `s` starts with a well-formed reference to one of XML's five
/// entities or to a character.
pub(super) fn reference(s: &str) -> Option<(char, usize)> {
    let b = s.as_bytes();
    if b.get(1) == Some(&b'#') {
        let (radix, digits) = if matches!(b.get(2), Some(b'x' | b'X')) {
            (16, 3)
        } else {
            (10, 2)
        };
        let len = b[digits.min(b.len())..]
            .iter()
            .take_while(|&&d| (d as char).is_digit(radix))
            .count();
        if len == 0 || b.get(digits + len) != Some(&b';') {
            return None;
        }
        let c = u32::from_str_radix(&s[digits..digits + len], radix)
            .ok()
            .and_then(char::from_u32)
            .filter(|&c| xml_allows(c))
            .unwrap_or('\u{FFFD}');
        return Some((c, digits + len + 1));
    }
    let len = b[1..]
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric())
        .count();
    if b.get(1 + len) != Some(&b';') {
        return None;
    }
    let c = match &s[1..1 + len] {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "quot" => '"',
        "apos" => '\'',
        _ => return None,
    };
    Some((c, len + 2))
}

/// A piece of an SVG document ([`Markup`]); positions are byte offsets, `end` exclusive.
#[derive(Debug)]
#[cfg_attr(not(feature = "mermaid"), allow(dead_code))]
pub(super) enum Item<'a> {
    /// Character data, also a `<` that starts no markup.
    Text,
    /// A start tag `<name …>` or an empty-element tag `<name …/>`.
    Start(StartTag<'a>),
    /// An end tag `</name>`.
    End {
        start: usize,
        end: usize,
        name: &'a str,
    },
    /// A comment, CDATA section, processing instruction or declaration (`<!DOCTYPE …>`).
    Special { start: usize, end: usize },
}

/// The markup of an XML (or HTML) document, read the way an HTML tokenizer reads tags: a quote only
/// delimits an attribute value, a tag ends at the first `>` outside one, `/` between attributes is
/// a separator. Comments, CDATA sections and processing instructions end at their first
/// terminator, declarations at the first `>` outside quotes and brackets; an unterminated
/// construct extends to the end of the input. For well-formed XML this is how an XML parser reads
/// it, too.
#[derive(Debug)]
pub(super) struct Markup<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Markup<'a> {
    pub(super) fn new(s: &'a str) -> Self {
        Self { s, pos: 0 }
    }
}

impl<'a> Iterator for Markup<'a> {
    type Item = Item<'a>;

    fn next(&mut self) -> Option<Item<'a>> {
        let s = self.s;
        let start = self.pos;
        let rest = s.get(start..).filter(|rest| !rest.is_empty())?;
        let (item, end) = if !rest.starts_with('<') {
            (Item::Text, rest.find('<').map_or(s.len(), |lt| start + lt))
        } else if rest.starts_with("<!--") {
            special(s, start, 4, "-->")
        } else if rest.starts_with("<![CDATA[") {
            special(s, start, 9, "]]>")
        } else if rest.starts_with("<?") {
            special(s, start, 2, "?>")
        } else if rest.starts_with("<!") {
            let end = start + declaration_len(rest);
            (Item::Special { start, end }, end)
        } else if rest.starts_with("</") && starts_name(&rest[2..]) {
            let name_end = start + 2 + name_len(&rest[2..]);
            let end = parse_tail(s, name_end).end;
            let name = &s[start + 2..name_end];
            (Item::End { start, end, name }, end)
        } else if starts_name(&rest[1..]) {
            let tag = parse_start_tag(s, start);
            let end = tag.end;
            (Item::Start(tag), end)
        } else {
            let end = rest[1..].find('<').map_or(s.len(), |lt| start + 1 + lt);
            (Item::Text, end)
        };
        self.pos = end;
        Some(item)
    }
}

/// A comment, CDATA section or processing instruction: from `start` past its terminator.
fn special<'a>(s: &str, start: usize, open: usize, close: &str) -> (Item<'a>, usize) {
    let end = s[start + open..]
        .find(close)
        .map_or(s.len(), |at| start + open + at + close.len());
    (Item::Special { start, end }, end)
}

/// The length of a declaration such as `<!DOCTYPE svg [ … ]>` at the start of `s`.
fn declaration_len(s: &str) -> usize {
    let mut quote = None;
    let mut depth = 0usize;
    for (i, b) in s.bytes().enumerate().skip(2) {
        match (quote, b) {
            (Some(q), _) if b == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(b),
            (None, b'[') => depth += 1,
            (None, b']') => depth = depth.saturating_sub(1),
            (None, b'>') if depth == 0 => return i + 1,
            _ => {}
        }
    }
    s.len()
}

/// Whether a tag name starts here: an ASCII letter, `_`, `:` or a non-ASCII character (wider than
/// both XML's and HTML's rule, so nothing either of them reads as a tag is taken for text).
fn starts_name(s: &str) -> bool {
    s.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_' || b == b':' || !b.is_ascii())
}

/// The length of a tag name: up to whitespace, `/` or `>`.
fn name_len(s: &str) -> usize {
    s.bytes()
        .take_while(|&b| !(b.is_ascii_whitespace() || b == b'/' || b == b'>'))
        .count()
}

/// A start tag ([`parse_start_tag`]).
#[derive(Debug)]
#[cfg_attr(not(feature = "mermaid"), allow(dead_code))]
pub(super) struct StartTag<'a> {
    /// The offset of the `<`.
    pub(super) start: usize,
    /// The offset after the `>` (the end of the input when the tag does not end).
    pub(super) end: usize,
    /// The qualified name, e.g. `svg` or `svg:a`.
    pub(super) name: &'a str,
    pub(super) attrs: Vec<Attr<'a>>,
    /// An empty-element tag `<name …/>`.
    pub(super) self_closing: bool,
    /// Whether the tag ends with `>`.
    pub(super) closed: bool,
}

impl StartTag<'_> {
    /// The unescaped value of the first attribute with this name.
    fn attr<'s>(&self, s: &'s str, name: &str) -> Option<Cow<'s, str>> {
        self.attrs
            .iter()
            .find(|a| a.name == name)
            .and_then(|a| a.value)
            .map(|(from, to)| unescape(&s[from..to]))
    }
}

/// An attribute of a [`StartTag`].
#[derive(Debug)]
#[cfg_attr(not(feature = "mermaid"), allow(dead_code))]
pub(super) struct Attr<'a> {
    /// From the separator before the name (whitespace or `/`) to the end of the value, after its
    /// closing quote: removing `start..end` removes the attribute.
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) name: &'a str,
    /// Where the raw value is (still escaped, without its quotes); `None` without `=`.
    pub(super) value: Option<(usize, usize)>,
}

/// The start tag whose `<` is at `start`; a tag name must start right after it.
pub(super) fn parse_start_tag(s: &str, start: usize) -> StartTag<'_> {
    let name_end = start + 1 + name_len(&s[start + 1..]);
    let tail = parse_tail(s, name_end);
    StartTag {
        start,
        end: tail.end,
        name: &s[start + 1..name_end],
        attrs: tail.attrs,
        self_closing: tail.self_closing,
        closed: tail.closed,
    }
}

/// The attributes after a tag name and the end of the tag.
struct Tail<'a> {
    attrs: Vec<Attr<'a>>,
    end: usize,
    self_closing: bool,
    closed: bool,
}

/// The attributes from `from` (after the tag name) to the `>` ending the tag, as an HTML tokenizer
/// reads them. Every byte the scan stops at is ASCII, so all offsets are character boundaries.
fn parse_tail(s: &str, from: usize) -> Tail<'_> {
    let b = s.as_bytes();
    let mut attrs = Vec::new();
    let mut self_closing = false;
    let mut i = from;
    loop {
        let sep = i;
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            self_closing = b[i] == b'/' && b.get(i + 1) == Some(&b'>');
            i += 1;
        }
        if i >= b.len() {
            return Tail {
                attrs,
                end: b.len(),
                self_closing: false,
                closed: false,
            };
        }
        if b[i] == b'>' {
            return Tail {
                attrs,
                end: i + 1,
                self_closing,
                closed: true,
            };
        }
        self_closing = false;
        // The name: its first character may be anything but whitespace, `/` and `>` (even `=`),
        // then it runs up to whitespace, `/`, `>` or `=`.
        let name_start = i;
        i += 1;
        while i < b.len() && !(b[i].is_ascii_whitespace() || matches!(b[i], b'/' | b'>' | b'=')) {
            i += 1;
        }
        let name = &s[name_start..i];
        let mut j = i;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut value = None;
        if b.get(j) == Some(&b'=') {
            j += 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if let Some(&quote @ (b'"' | b'\'')) = b.get(j) {
                let from = j + 1;
                let to = s[from..]
                    .find(quote as char)
                    .map_or(b.len(), |at| from + at);
                value = Some((from, to));
                i = (to + 1).min(b.len());
            } else {
                let from = j;
                while j < b.len() && !(b[j].is_ascii_whitespace() || b[j] == b'>') {
                    j += 1;
                }
                value = Some((from, j));
                i = j;
            }
        }
        attrs.push(Attr {
            start: sep,
            end: i,
            name,
            value,
        });
    }
}

/// The namespace the prefix `xml` is bound to.
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
/// The namespace of namespace declarations, which no prefix may be bound to.
const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

/// Why `doc` is not a well-formed XML 1.0 document with namespaces, if it is not (the message
/// names a byte offset). Browsers and exporters do not display an SVG that is not, so a rendered
/// SVG must pass this. Comments, CDATA sections, processing instructions and declarations count
/// as errors: the sanitizer removes them all, so no DTD declares entities either, and only XML's
/// five entities and character references are references.
#[cfg_attr(not(feature = "mermaid"), allow(dead_code))]
pub(super) fn check_well_formed(doc: &str) -> Result<(), String> {
    if let Some((at, c)) = doc.char_indices().find(|&(_, c)| !xml_allows(c)) {
        return Err(format!(
            "U+{:04X} at byte {at} is a character XML does not allow",
            u32::from(c)
        ));
    }
    let b = doc.as_bytes();
    // The open elements: the name and how many prefix bindings were in scope before it.
    let mut open: Vec<(&str, usize)> = Vec::new();
    // The prefixes in scope with their namespaces, innermost last.
    let mut bindings: Vec<(&str, Cow<'_, str>)> = Vec::new();
    let mut root = false;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            let end = doc[i..].find('<').map_or(b.len(), |lt| i + lt);
            let text = &doc[i..end];
            if open.is_empty() {
                if !text.chars().all(is_xml_space) {
                    return Err(format!("text outside the root element at byte {i}"));
                }
            } else {
                check_references(text, i)?;
                if let Some(at) = text.find("]]>") {
                    return Err(format!("`]]>` in text at byte {}", i + at));
                }
            }
            i = end;
        } else if b.get(i + 1) == Some(&b'/') {
            let name = xml_name(&doc[i + 2..])
                .ok_or_else(|| format!("an end tag without a name at byte {i}"))?;
            let close = skip_xml_space(doc, i + 2 + name.len());
            if b.get(close) != Some(&b'>') {
                return Err(format!("the end tag at byte {i} does not end with `>`"));
            }
            match open.pop() {
                Some((started, scope)) if started == name => bindings.truncate(scope),
                Some((started, _)) => {
                    return Err(format!("`</{name}>` at byte {i} ends `<{started}>`"));
                }
                None => return Err(format!("`</{name}>` at byte {i} ends no element")),
            }
            i = close + 1;
        } else if matches!(b.get(i + 1), Some(b'!' | b'?')) {
            return Err(format!(
                "a comment, CDATA section, declaration or processing instruction at byte {i}"
            ));
        } else {
            if root && open.is_empty() {
                return Err(format!("a second root element at byte {i}"));
            }
            let tag = xml_start_tag(doc, i)?;
            let scope = bindings.len();
            for &(attr, value) in &tag.attrs {
                let prefix = match attr.strip_prefix("xmlns") {
                    Some("") => None,
                    Some(declared) => match declared.strip_prefix(':') {
                        Some(prefix) => Some(prefix),
                        None => continue,
                    },
                    None => continue,
                };
                let namespace = unescape(value);
                check_binding(prefix, &namespace)
                    .map_err(|why| format!("{why} in the tag at byte {i}"))?;
                if let Some(prefix) = prefix {
                    bindings.push((prefix, namespace));
                }
            }
            namespace_of(tag.name, &bindings, true)
                .map_err(|why| format!("{why} in the tag at byte {i}"))?;
            // Prefixed attributes, by namespace and local name: they must differ, too.
            let mut expanded: Vec<(&str, &str)> = Vec::new();
            for &(attr, _) in &tag.attrs {
                if attr == "xmlns" || attr.starts_with("xmlns:") {
                    continue;
                }
                if let Some(name) = namespace_of(attr, &bindings, false)
                    .map_err(|why| format!("{why} in the tag at byte {i}"))?
                {
                    if expanded.contains(&name) {
                        return Err(format!(
                            "the attribute `{attr}` twice in the tag at byte {i}"
                        ));
                    }
                    expanded.push(name);
                }
            }
            if tag.empty {
                bindings.truncate(scope);
            } else {
                open.push((tag.name, scope));
            }
            root = true;
            i = tag.end;
        }
    }
    if let Some((name, _)) = open.last() {
        return Err(format!("`<{name}>` does not end"));
    }
    if !root {
        return Err("no root element".to_owned());
    }
    Ok(())
}

/// A start tag as XML reads it ([`xml_start_tag`]).
struct XmlTag<'a> {
    name: &'a str,
    /// Names and raw values (still escaped, without quotes).
    attrs: Vec<(&'a str, &'a str)>,
    /// The offset after the `>`.
    end: usize,
    /// An empty-element tag `<name …/>`.
    empty: bool,
}

/// The start tag whose `<` is at `i`, if it is one XML accepts: quoted values without `<` and with
/// well-formed references, space between attributes, each name once.
fn xml_start_tag(doc: &str, i: usize) -> Result<XmlTag<'_>, String> {
    let b = doc.as_bytes();
    let name =
        xml_name(&doc[i + 1..]).ok_or_else(|| format!("a `<` that starts no tag at byte {i}"))?;
    let mut attrs: Vec<(&str, &str)> = Vec::new();
    let mut at = i + 1 + name.len();
    loop {
        let next = skip_xml_space(doc, at);
        match b.get(next) {
            Some(b'>') => {
                return Ok(XmlTag {
                    name,
                    attrs,
                    end: next + 1,
                    empty: false,
                });
            }
            Some(b'/') if b.get(next + 1) == Some(&b'>') => {
                return Ok(XmlTag {
                    name,
                    attrs,
                    end: next + 2,
                    empty: true,
                });
            }
            None => return Err(format!("the start tag at byte {i} does not end")),
            _ if next == at => return Err(format!("no space before byte {at} in a start tag")),
            _ => {}
        }
        let attr =
            xml_name(&doc[next..]).ok_or_else(|| format!("no attribute name at byte {next}"))?;
        let eq = skip_xml_space(doc, next + attr.len());
        if b.get(eq) != Some(&b'=') {
            return Err(format!(
                "the attribute `{attr}` at byte {next} has no value"
            ));
        }
        let open_quote = skip_xml_space(doc, eq + 1);
        let quote = match b.get(open_quote) {
            Some(&quote @ (b'"' | b'\'')) => char::from(quote),
            _ => {
                return Err(format!(
                    "the value of the attribute `{attr}` at byte {next} is not quoted"
                ));
            }
        };
        let from = open_quote + 1;
        let to = doc[from..]
            .find(quote)
            .map(|len| from + len)
            .ok_or_else(|| {
                format!("the value of the attribute `{attr}` at byte {next} does not end")
            })?;
        let value = &doc[from..to];
        if let Some(lt) = value.find('<') {
            return Err(format!(
                "`<` in the value of the attribute `{attr}` at byte {}",
                from + lt
            ));
        }
        check_references(value, from)?;
        if attrs.iter().any(|&(seen, _)| seen == attr) {
            return Err(format!(
                "the attribute `{attr}` twice in the tag at byte {i}"
            ));
        }
        attrs.push((attr, value));
        at = to + 1;
    }
}

/// Whether a namespace declaration is allowed: `prefix` is `None` for `xmlns="…"`.
fn check_binding(prefix: Option<&str>, namespace: &str) -> Result<(), String> {
    let reserved = namespace == XML_NAMESPACE || namespace == XMLNS_NAMESPACE;
    match prefix {
        None if reserved => Err(format!("`{namespace}` as the default namespace")),
        None => Ok(()),
        Some(prefix) if prefix.is_empty() || prefix.contains(':') => {
            Err(format!("`xmlns:{prefix}` declares no prefix"))
        }
        Some("xmlns") => Err("a declaration of the prefix `xmlns`".to_owned()),
        Some("xml") if namespace == XML_NAMESPACE => Ok(()),
        Some(prefix) if prefix == "xml" || reserved => {
            Err(format!("the prefix `{prefix}` bound to `{namespace}`"))
        }
        Some(prefix) if namespace.is_empty() => {
            Err(format!("the prefix `{prefix}` bound to no namespace"))
        }
        Some(_) => Ok(()),
    }
}

/// The namespace and local name of a prefixed element or attribute name, `None` without a prefix;
/// an error when the name is not a qualified name or its prefix is not bound.
fn namespace_of<'a>(
    name: &'a str,
    bindings: &'a [(&str, Cow<'_, str>)],
    element: bool,
) -> Result<Option<(&'a str, &'a str)>, String> {
    let Some((prefix, local)) = name.split_once(':') else {
        return Ok(None);
    };
    if prefix.is_empty() || local.is_empty() || local.contains(':') {
        return Err(format!("`{name}` is not a qualified name"));
    }
    match prefix {
        "xml" => Ok(Some((XML_NAMESPACE, local))),
        "xmlns" if element => Err(format!("the element `{name}` has the prefix `xmlns`")),
        _ => bindings
            .iter()
            .rev()
            .find(|(bound, _)| *bound == prefix)
            .map(|(_, namespace)| Some((namespace.as_ref(), local)))
            .ok_or_else(|| format!("the prefix of `{name}` is not declared")),
    }
}

/// That every `&` in `text` (at `offset` in the document) starts a reference to one of XML's five
/// entities, or a character reference (`&#N;`, `&#xN;`) to a character XML allows.
fn check_references(text: &str, offset: usize) -> Result<(), String> {
    match text
        .match_indices('&')
        .find(|&(at, _)| !is_xml_reference(&text[at + 1..]))
    {
        Some((at, _)) => Err(format!(
            "the `&` at byte {} starts no reference XML defines",
            offset + at
        )),
        None => Ok(()),
    }
}

/// Whether `s` (what follows a `&`) starts with the rest of a reference XML defines (no DTD) to a
/// character it allows. Unlike HTML, XML has no `&#X…;`.
fn is_xml_reference(s: &str) -> bool {
    let Some((body, _)) = s.split_once(';') else {
        return false;
    };
    let Some(number) = body.strip_prefix('#') else {
        return matches!(body, "lt" | "gt" | "amp" | "quot" | "apos");
    };
    let code = match number.strip_prefix('x') {
        Some(hex) if !hex.is_empty() && hex.bytes().all(|d| d.is_ascii_hexdigit()) => {
            u32::from_str_radix(hex, 16).ok()
        }
        None if !number.is_empty() && number.bytes().all(|d| d.is_ascii_digit()) => {
            number.parse().ok()
        }
        _ => None,
    };
    code.and_then(char::from_u32).is_some_and(xml_allows)
}

/// The XML `Name` at the start of `s`.
fn xml_name(s: &str) -> Option<&str> {
    let mut chars = s.char_indices();
    let (_, first) = chars.next()?;
    if !is_name_start(first) {
        return None;
    }
    let end = chars
        .find(|&(_, c)| !is_name_char(c))
        .map_or(s.len(), |(at, _)| at);
    Some(&s[..end])
}

/// XML's `NameStartChar`.
fn is_name_start(c: char) -> bool {
    matches!(c,
        ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// XML's `NameChar`.
fn is_name_char(c: char) -> bool {
    is_name_start(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// The offset after the XML white space (space, tab, line feed, carriage return) at `from`.
fn skip_xml_space(s: &str, from: usize) -> usize {
    from + s[from..]
        .bytes()
        .take_while(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_and_unescape_are_inverse() {
        let text = "a < b && c > d ]]> \"q\" 'a' \r\n tab\t ünï 😀 &amp;";
        assert_eq!(unescape(&escape_text(text)), text);
        assert_eq!(escape_text("\u{0}\u{1}\u{B}\u{FFFE}"), "\u{FFFD}".repeat(4));
    }

    #[test]
    fn references() {
        assert_eq!(unescape("&lt;&gt;&amp;&quot;&apos;"), "<>&\"'");
        assert_eq!(unescape("&#65;&#x42;&#X43;&#0000068;"), "ABCD");
        assert_eq!(
            unescape("&#0;&#xD800;&#x110000;&#99999999999;"),
            "\u{FFFD}".repeat(4)
        );
        let kept = "&nbsp; &#65 & &; &#; &#x; &amp &";
        assert_eq!(unescape(kept), kept);
        assert_eq!(reference("&colon;"), None);
        assert_eq!(reference("&#58;x"), Some((':', 5)));
        assert_eq!(reference("&#"), None);
    }

    #[test]
    fn markup_items() {
        let s = "<?xml version=\"1.0\"?><!-- c --><!DOCTYPE svg [<!ENTITY e \">\">]>\n\
                 <svg a=\">\" b='x'>t < u<![CDATA[<x>]]><g/></svg ><!-- open";
        let items: Vec<String> = Markup::new(s)
            .map(|item| match item {
                Item::Text => "text".to_owned(),
                Item::Start(tag) => format!(
                    "<{} {}{}>",
                    tag.name,
                    tag.attrs.len(),
                    if tag.self_closing { "/" } else { "" }
                ),
                Item::End { name, .. } => format!("</{name}>"),
                Item::Special { start, end } => s[start..end].to_owned(),
            })
            .collect();
        assert_eq!(
            items,
            [
                "<?xml version=\"1.0\"?>",
                "<!-- c -->",
                "<!DOCTYPE svg [<!ENTITY e \">\">]>",
                "text",
                "<svg 2>",
                "text",
                "text",
                "<![CDATA[<x>]]>",
                "<g 0/>",
                "</svg>",
                "<!-- open",
            ]
        );
    }

    #[test]
    fn attributes_as_an_html_tokenizer_reads_them() {
        let s = "<a/x=\"1\"y='2' z=3 w on=\"a>b\" é=\"\" />";
        let tag = parse_start_tag(s, 0);
        let attrs: Vec<(&str, Option<&str>)> = tag
            .attrs
            .iter()
            .map(|a| (a.name, a.value.map(|(f, t)| &s[f..t])))
            .collect();
        assert_eq!(
            attrs,
            [
                ("x", Some("1")),
                ("y", Some("2")),
                ("z", Some("3")),
                ("w", None),
                ("on", Some("a>b")),
                ("é", Some(""))
            ]
        );
        assert!(tag.closed && tag.self_closing);
        assert_eq!(tag.end, s.len());
        let open = parse_start_tag("<a b=\"1>", 0);
        assert!(!open.closed);
    }

    #[test]
    fn well_formed_documents() {
        for doc in [
            "<svg/>",
            " \r\n<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>\n",
            "<svg a = '1'\tb=\"x'y\"\n/>",
            "<svg xmlns:xlink=\"http://www.w3.org/1999/xlink\"><a xlink:href=\"#n\">t</a></svg>",
            "<s><g xmlns:p=\"urn:a\" p:x=\"1\"/><g xmlns:p=\"urn:b\" p:x=\"2\" xml:space=\"preserve\"/></s>",
            "<s>&lt;&gt;&amp;&quot;&apos;&#65;&#x1F600;&#x9;é 😀 ]] &gt; \"q\" 'a'</s>",
            "<s a=\"&#13;&amp;>\">a>b</s>",
            "<Ünï-c.ode_1 x·y=\"\"></Ünï-c.ode_1 >",
            "<a><b><c/></b><b></b></a>",
            "<metadata><mermaid xmlns=\"urn:polarion-rs:mermaid\" version=\"1\">x</mermaid></metadata>",
        ] {
            assert_eq!(check_well_formed(doc), Ok(()), "{doc}");
        }
    }

    #[test]
    fn documents_that_are_not_well_formed() {
        for (doc, why) in [
            ("", "no root element"),
            ("  ", "no root element"),
            ("text", "text outside the root element"),
            ("<a/>x", "text outside the root element"),
            ("<a/><b/>", "a second root element"),
            ("<a>", "`<a>` does not end"),
            ("<a></b>", "`</b>` at byte 3 ends `<a>`"),
            ("<a/></a>", "`</a>` at byte 4 ends no element"),
            ("<a></a", "the end tag at byte 3 does not end"),
            ("<a></ a>", "an end tag without a name"),
            ("<a b=\"1\"", "the start tag at byte 0 does not end"),
            ("< a/>", "a `<` that starts no tag"),
            ("<1a/>", "a `<` that starts no tag"),
            ("<a>1 < 2</a>", "a `<` that starts no tag"),
            ("<a b=\"1\"c=\"2\"/>", "no space before byte 8"),
            ("<a/b/>", "no space before byte 2"),
            ("<a b/>", "the attribute `b` at byte 3 has no value"),
            ("<a b=1/>", "is not quoted"),
            ("<a b=\"1/>", "does not end"),
            ("<a b=\"1\" b='2'/>", "the attribute `b` twice"),
            (
                "<a b=\"x<y\"/>",
                "`<` in the value of the attribute `b` at byte 7",
            ),
            ("<a b=\"x&y\"/>", "the `&` at byte 7 starts no reference"),
            ("<a>&nbsp;</a>", "the `&` at byte 3"),
            ("<a>&#11;</a>", "the `&` at byte 3"),
            ("<a>&#X41;</a>", "the `&` at byte 3"),
            ("<a>&#;&#x;</a>", "the `&` at byte 3"),
            ("<a>&lt</a>", "the `&` at byte 3"),
            ("<a>]]></a>", "`]]>` in text at byte 3"),
            ("<a>\u{b}</a>", "U+000B at byte 3"),
            ("<a>\u{FFFF}</a>", "U+FFFF at byte 3"),
            ("<a><!-- c --></a>", "a comment, CDATA section"),
            ("<a><![CDATA[x]]></a>", "a comment, CDATA section"),
            (
                "<?xml version=\"1.0\"?><a/>",
                "processing instruction at byte 0",
            ),
            ("<!DOCTYPE a><a/>", "declaration"),
            ("<a p:b=\"1\"/>", "the prefix of `p:b` is not declared"),
            ("<p:a/>", "the prefix of `p:a` is not declared"),
            (
                "<a><b xmlns:p=\"urn:x\"/><c p:d=\"1\"/></a>",
                "the prefix of `p:d`",
            ),
            ("<a xmlns:p=\"\"/>", "the prefix `p` bound to no namespace"),
            ("<a xmlns:=\"urn:x\"/>", "`xmlns:` declares no prefix"),
            (
                "<a xmlns:xmlns=\"urn:x\"/>",
                "a declaration of the prefix `xmlns`",
            ),
            (
                "<a xmlns:xml=\"urn:x\"/>",
                "the prefix `xml` bound to `urn:x`",
            ),
            (
                "<a xmlns:p=\"http://www.w3.org/2000/xmlns/\"/>",
                "the prefix `p` bound to",
            ),
            (
                "<a xmlns=\"http://www.w3.org/XML/1998/namespace\"/>",
                "as the default namespace",
            ),
            ("<a :b=\"1\"/>", "`:b` is not a qualified name"),
            (
                "<a b:c:d=\"1\" xmlns:b=\"urn:x\"/>",
                "`b:c:d` is not a qualified name",
            ),
            ("<xmlns:a/>", "has the prefix `xmlns`"),
            (
                "<a xmlns:p=\"urn:x\" xmlns:q=\"urn:x\" p:b=\"1\" q:b=\"2\"/>",
                "the attribute `q:b` twice",
            ),
        ] {
            match check_well_formed(doc) {
                Err(found) => assert!(found.contains(why), "{doc:?}: {found}"),
                Ok(()) => panic!("{doc:?} passed"),
            }
        }
    }
}
