//! HTML helpers: escaping, URL safety, a tag scanner, the raw HTML sanitizer, percent coding of
//! attachment ids, appending.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::ops::Range;

use crate::styles::{BR, BlockKind};

/// Whether XML 1.0 allows the character. Rich text travels to Polarion inside a SOAP (XML)
/// message, which NUL, the other C0 controls except tab and line breaks, U+FFFE and U+FFFF make
/// malformed (not even a character reference may name them).
pub(crate) fn xml_allows(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | ' '..='\u{fffd}' | '\u{10000}'..)
}

/// What [`xml_safe`] writes for a character XML does not allow: a space for the form feed, which
/// is whitespace to HTML (it separates attributes), U+FFFD for the others, which are not. So the
/// replacement never changes how a browser splits the HTML into tags and attributes.
fn xml_replacement(c: char) -> char {
    if c == '\u{c}' { ' ' } else { '\u{fffd}' }
}

/// `s` with every character XML does not allow ([`xml_allows`]) replaced: the form feed by a
/// space, the others by U+FFFD.
pub(crate) fn xml_safe(s: &str) -> Cow<'_, str> {
    if s.chars().all(xml_allows) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(
        s.chars()
            .map(|c| if xml_allows(c) { c } else { xml_replacement(c) })
            .collect(),
    )
}

/// Text content escaped: `& < >`, U+00A0 as `&nbsp;`, characters XML does not allow (NUL, ESC,
/// ...) replaced as by [`xml_safe`].
pub(crate) fn escape_text(s: &str) -> Cow<'_, str> {
    escape(s, false)
}

/// An attribute value (written in double quotes) escaped: as [`escape_text`] plus `"`.
pub(crate) fn escape_attr(s: &str) -> Cow<'_, str> {
    escape(s, true)
}

/// [`escape_text`] appended to `out`.
pub(crate) fn push_text(out: &mut String, s: &str) {
    out.push_str(&escape(s, false));
}

/// [`escape_attr`] appended to `out`.
pub(crate) fn push_attr(out: &mut String, s: &str) {
    out.push_str(&escape(s, true));
}

fn escape(s: &str, attr: bool) -> Cow<'_, str> {
    let needs =
        |c: char| matches!(c, '&' | '<' | '>' | '\u{a0}') || (attr && c == '"') || !xml_allows(c);
    let Some(first) = s.find(needs) else {
        return Cow::Borrowed(s);
    };
    let mut out = String::with_capacity(s.len() + 16);
    out.push_str(&s[..first]);
    for c in s[first..].chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '"' if attr => out.push_str("&quot;"),
            c if !xml_allows(c) => out.push(xml_replacement(c)),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// Whether a URL would run script or embed content when followed: `javascript:`, `vbscript:`,
/// `file:` and `data:` (except `data:image/png|gif|jpeg|webp`), ignoring case, after removing
/// what browsers ignore (leading spaces and control characters, tabs and line breaks anywhere).
pub(crate) fn dangerous_url(url: &str) -> bool {
    let url = url.trim_start_matches(|c: char| c <= ' ');
    if url.contains(['\t', '\n', '\r']) {
        comrak::html::dangerous_url(&url.replace(['\t', '\n', '\r'], ""))
    } else {
        comrak::html::dangerous_url(url)
    }
}

/// An attachment id for a URL (`workitemimg:ID`): everything but `A-Z a-z 0-9 - . _ ~`
/// percent-encoded.
pub(crate) fn percent_encode(id: &str) -> Cow<'_, str> {
    let keep = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~');
    if id.bytes().all(keep) {
        return Cow::Borrowed(id);
    }
    let mut out = String::with_capacity(id.len() + 8);
    for b in id.bytes() {
        if keep(b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    Cow::Owned(out)
}

/// `%XX` sequences decoded (others kept as they are), invalid UTF-8 replaced.
pub(crate) fn percent_decode(s: &str) -> Cow<'_, str> {
    if !s.contains('%') {
        return Cow::Borrowed(s);
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |j: usize| bytes.get(j).and_then(|&b| char::from(b).to_digit(16));
        match (bytes[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    Cow::Owned(String::from_utf8_lossy(&out).into_owned())
}

/// A piece of raw HTML, as a browser's tokenizer splits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Token<'a> {
    /// Text, including a `<` that starts no tag.
    Text(&'a str),
    /// `<!-- … -->`; an unclosed one runs to the end of the input.
    Comment { src: &'a str, closed: bool },
    /// `<!…>`, `<?…>` and `</…>` not followed by a letter: comments to a browser.
    Bogus(&'a str),
    /// A start or end tag.
    Tag(Tag<'a>),
    /// A tag or `<!…>` / `<?…>` that does not end before the input does (to the end).
    Unclosed(&'a str),
}

/// A start or end tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tag<'a> {
    /// The whole tag, `<` to `>`.
    pub(crate) src: &'a str,
    /// The name as written (compare ignoring ASCII case).
    pub(crate) name: &'a str,
    pub(crate) closing: bool,
    /// Ends with `/>`.
    pub(crate) self_closing: bool,
    pub(crate) attrs: Vec<Attr<'a>>,
}

/// An attribute of a [`Tag`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attr<'a> {
    /// Where it is in [`Tag::src`], including the whitespace before its name.
    pub(crate) span: Range<usize>,
    /// The name as written (compare ignoring ASCII case).
    pub(crate) name: &'a str,
    /// The value as written, without quotes; character references are not decoded.
    pub(crate) value: Option<&'a str>,
    /// The value is in quotes.
    pub(crate) quoted: bool,
}

impl Tag<'_> {
    /// Whether the tag has this name (ignoring ASCII case).
    pub(crate) fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }
}

/// The tokens of raw HTML in order; their sources concatenated are the input.
pub(crate) fn tokens(html: &str) -> Tokens<'_> {
    Tokens { src: html, pos: 0 }
}

/// Iterator returned by [`tokens`].
#[derive(Debug)]
pub(crate) struct Tokens<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        let (s, start) = (self.src, self.pos);
        if start >= s.len() {
            return None;
        }
        if s.as_bytes()[start] == b'<'
            && let Some((token, end)) = markup_at(s, start)
        {
            self.pos = end;
            return Some(token);
        }
        // Text up to the next `<` (a `<` at the start here starts no markup).
        let end = s.as_bytes()[start + 1..]
            .iter()
            .position(|&b| b == b'<')
            .map_or(s.len(), |i| start + 1 + i);
        self.pos = end;
        Some(Token::Text(&s[start..end]))
    }
}

fn is_space(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

/// The markup starting with the `<` at `i`, and where it ends; `None` if the `<` is text.
fn markup_at(s: &str, i: usize) -> Option<(Token<'_>, usize)> {
    let b = s.as_bytes();
    let rest = &b[i..];
    if rest.starts_with(b"<!--") {
        let body = i + 4;
        let end = if b.get(body) == Some(&b'>') {
            Some(body + 1)
        } else if b[body..].starts_with(b"->") {
            Some(body + 2)
        } else {
            (body..b.len()).find_map(|j| {
                let tail = &b[j..];
                if tail.starts_with(b"-->") {
                    Some(j + 3)
                } else if tail.starts_with(b"--!>") {
                    Some(j + 4)
                } else {
                    None
                }
            })
        };
        return Some(match end {
            Some(end) => (
                Token::Comment {
                    src: &s[i..end],
                    closed: true,
                },
                end,
            ),
            None => (
                Token::Comment {
                    src: &s[i..],
                    closed: false,
                },
                s.len(),
            ),
        });
    }
    let bogus = |from: usize| match s[from..].find('>') {
        Some(j) => (Token::Bogus(&s[i..from + j + 1]), from + j + 1),
        None => (Token::Unclosed(&s[i..]), s.len()),
    };
    match (rest.get(1), rest.get(2)) {
        (Some(b'!' | b'?'), _) => Some(bogus(i + 2)),
        (Some(b'/'), Some(c)) if c.is_ascii_alphabetic() => Some(tag_at(s, i, true)),
        (Some(b'/'), Some(_)) => Some(bogus(i + 2)),
        // Whatever follows would continue it.
        (Some(b'/'), None) => Some((Token::Unclosed(&s[i..]), s.len())),
        (Some(c), _) if c.is_ascii_alphabetic() => Some(tag_at(s, i, false)),
        _ => None,
    }
}

/// The tag starting at `i` (a `<` followed by a letter, or `</` and a letter).
fn tag_at(s: &str, i: usize, closing: bool) -> (Token<'_>, usize) {
    let b = s.as_bytes();
    let unclosed = (Token::Unclosed(&s[i..]), s.len());
    let mut p = i + 1 + usize::from(closing);
    let name_start = p;
    while p < b.len() && !is_space(b[p]) && b[p] != b'/' && b[p] != b'>' {
        p += 1;
    }
    let name = &s[name_start..p];
    let mut attrs = Vec::new();
    loop {
        let attr_start = p;
        let mut slash = false;
        while p < b.len() && (is_space(b[p]) || b[p] == b'/') {
            slash = b[p] == b'/';
            p += 1;
        }
        let Some(&c) = b.get(p) else {
            return unclosed;
        };
        if c == b'>' {
            let end = p + 1;
            let tag = Tag {
                src: &s[i..end],
                name,
                closing,
                self_closing: slash,
                attrs,
            };
            return (Token::Tag(tag), end);
        }
        // The first character of a name may be `=`.
        let name_start = p;
        p += 1;
        while p < b.len() && !is_space(b[p]) && !matches!(b[p], b'/' | b'>' | b'=') {
            p += 1;
        }
        let attr_name = &s[name_start..p];
        let mut q = p;
        while q < b.len() && is_space(b[q]) {
            q += 1;
        }
        let mut value = None;
        let mut quoted = false;
        if b.get(q) == Some(&b'=') {
            q += 1;
            while q < b.len() && is_space(b[q]) {
                q += 1;
            }
            match b.get(q) {
                None => return unclosed,
                Some(&quote @ (b'"' | b'\'')) => {
                    let Some(len) = s[q + 1..].find(char::from(quote)) else {
                        return unclosed;
                    };
                    value = Some(&s[q + 1..q + 1 + len]);
                    quoted = true;
                    p = q + 1 + len + 1;
                }
                Some(b'>') => {
                    value = Some("");
                    p = q;
                }
                Some(_) => {
                    let v = q;
                    while q < b.len() && !is_space(b[q]) && b[q] != b'>' {
                        q += 1;
                    }
                    value = Some(&s[v..q]);
                    p = q;
                }
            }
        }
        attrs.push(Attr {
            span: attr_start - i..p - i,
            name: attr_name,
            value,
            quoted,
        });
    }
}

/// Elements filtered as GitHub's tag filter does (their `<` becomes `&lt;`): those whose content a
/// browser does not parse as HTML (the scanner could not see what follows), plus those that embed
/// other documents or change how the page loads and resolves URLs.
const FILTERED: [&str; 17] = [
    "base",
    "embed",
    "frame",
    "frameset",
    "iframe",
    "link",
    "meta",
    "noembed",
    "noframes",
    "noscript",
    "object",
    "plaintext",
    "script",
    "style",
    "textarea",
    "title",
    "xmp",
];

/// Attributes holding a URL that is followed or loaded.
const URL_ATTRS: [&str; 11] = [
    "action",
    "background",
    "codebase",
    "data",
    "dynsrc",
    "formaction",
    "href",
    "lowsrc",
    "poster",
    "src",
    "xlink:href",
];

/// SVG animation attributes, which can set another attribute (an `href`) to their value.
const ANIMATION_ATTRS: [&str; 4] = ["by", "from", "to", "values"];

/// Whether [`sanitize_raw_html`] filters elements of this name (ignoring ASCII case): raw HTML
/// kept for the forward direction must not contain them, or they come back as text.
pub(crate) fn is_filtered(name: &str) -> bool {
    FILTERED.iter().any(|f| f.eq_ignore_ascii_case(name))
}

/// Raw HTML made safe to pass on, scanned as a browser does:
/// - tags filtered as by GitHub's tag filter (`<` becomes `&lt;`): `script`, `style`, `iframe`,
///   `textarea`, `title`, `xmp`, `noembed`, `noframes`, `noscript`, `plaintext`, and also
///   `object`, `embed`, `frame`, `frameset`, `base`, `link`, `meta`. The whole tag becomes text,
///   tags in its quoted values included;
/// - `on*` and `srcdoc` attributes removed;
/// - attributes holding a dangerous URL ([`dangerous_url`]: `href`, `src`, `xlink:href`, `action`,
///   `formaction`, `data`, ...) removed, as are styles with `expression(`, `javascript:` and the
///   like, and SVG animations setting such values;
/// - a tag or comment not closed before the end becomes text, to the end.
///
/// Tags that need no change stay byte-identical; sanitizing is idempotent and takes linear time.
pub(crate) fn sanitize_raw_html(html: &str) -> Cow<'_, str> {
    let mut out: Option<String> = None;
    let mut done = 0;
    let mut tokens = tokens(html);
    loop {
        let pos = tokens.pos;
        let Some(token) = tokens.next() else {
            break;
        };
        let fixed = match &token {
            Token::Tag(tag) if is_filtered(tag.name) => Some(tag.src.replace('<', "&lt;")),
            Token::Unclosed(src) | Token::Comment { src, closed: false } => {
                Some(src.replace('<', "&lt;"))
            }
            Token::Tag(tag) => without_dangerous_attrs(tag),
            _ => None,
        };
        if let Some(fixed) = fixed {
            let out = out.get_or_insert_with(|| String::with_capacity(html.len() + 16));
            out.push_str(&html[done..pos]);
            out.push_str(&fixed);
            done = tokens.pos;
        }
    }
    match out {
        None => Cow::Borrowed(html),
        Some(mut out) => {
            out.push_str(&html[done..]);
            Cow::Owned(out)
        }
    }
}

/// The `src` of an `<img>` tag as written in raw HTML: character references decoded, without the
/// control characters and spaces at its edges.
fn src_key(value: &str) -> String {
    decode_entities(value)
        .trim_matches(|c: char| c <= ' ')
        .to_owned()
}

/// The first `src` attribute of an `<img>` start tag.
fn img_src<'t, 'a>(tag: &'t Tag<'a>) -> Option<&'t Attr<'a>> {
    if tag.closing || !tag.is("img") {
        return None;
    }
    tag.attrs
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case("src") && a.value.is_some())
}

/// The sources of the `<img>` tags in HTML ([`tokens`]), in order, repeats included: character
/// references decoded, without the control characters and spaces at their edges.
pub(crate) fn img_srcs(html: &str) -> Vec<String> {
    tokens(html)
        .filter_map(|token| match token {
            Token::Tag(tag) => img_src(&tag).and_then(|a| a.value).map(src_key),
            _ => None,
        })
        .collect()
}

/// HTML with the source of each `<img>` tag for which `new` returns one replaced (given the
/// source as [`img_srcs`] returns it). The attribute is written as `src="…"`; everything else
/// stays byte-identical.
pub(crate) fn rewrite_img_src<'h>(
    html: &'h str,
    new: &dyn Fn(&str) -> Option<String>,
) -> Cow<'h, str> {
    let mut out: Option<String> = None;
    let mut done = 0;
    let mut tokens = tokens(html);
    loop {
        let pos = tokens.pos;
        let Some(token) = tokens.next() else {
            break;
        };
        let Token::Tag(tag) = &token else {
            continue;
        };
        let Some(attr) = img_src(tag) else {
            continue;
        };
        let Some(src) = new(&src_key(attr.value.unwrap_or(""))) else {
            continue;
        };
        let written = &tag.src[attr.span.clone()];
        let lead = written.len()
            - written
                .trim_start_matches(|c: char| c == '/' || (c.is_ascii() && is_space(c as u8)))
                .len();
        let out = out.get_or_insert_with(|| String::with_capacity(html.len() + 16));
        out.push_str(&html[done..pos + attr.span.start + lead]);
        out.push_str("src=\"");
        push_attr(out, &src);
        out.push('"');
        done = pos + attr.span.end;
    }
    match out {
        None => Cow::Borrowed(html),
        Some(mut out) => {
            out.push_str(&html[done..]);
            Cow::Owned(out)
        }
    }
}

/// What text written into a tag ends with, for what may follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagEnd {
    /// The tag's name: a name character would continue it.
    Name,
    /// An attribute without a value: a name character continues its name, `=` after whitespace
    /// gives it a value.
    Bare,
    /// An unquoted value: anything but whitespace and `>` (`/` too) continues it.
    Unquoted,
    /// A quoted value: anything may follow.
    Quoted,
}

/// The tag without its dangerous attributes, if it has any.
///
/// The attributes kept are copied as written. Where removing attributes brings two parts of the
/// tag together, a space keeps them apart as a browser read them before: `<a href=x
/// onclick='y'title=z>` must not become `<a href=xtitle=z>`, nor `<a href=x onclick="y"/title=z>`
/// become `<a href=x/title=z>` (`title` in the value of `href`). An attribute named `=…` that
/// would then follow an attribute without a value is removed too: after `<a b onclick=y =c>`
/// lost `onclick`, `=c` would be the value of `b`.
fn without_dangerous_attrs(tag: &Tag<'_>) -> Option<String> {
    if !tag.attrs.iter().any(dangerous_attr) {
        return None;
    }
    let src = tag.src;
    let mut out = String::with_capacity(src.len());
    out.push_str(&src[..tag.attrs[0].span.start]);
    let mut end = TagEnd::Name;
    // Something was removed since the last part written.
    let mut joined = false;
    for attr in &tag.attrs {
        if dangerous_attr(attr) || (joined && end == TagEnd::Bare && attr.name.starts_with('=')) {
            joined = true;
            continue;
        }
        let text = &src[attr.span.clone()];
        if joined && !text.as_bytes().first().is_some_and(|&b| is_space(b)) {
            // It followed a quoted value directly or after a `/`: that no longer separates it.
            out.push(' ');
        }
        out.push_str(text);
        end = match (attr.value, attr.quoted) {
            (None, _) => TagEnd::Bare,
            (Some(_), false) => TagEnd::Unquoted,
            (Some(_), true) => TagEnd::Quoted,
        };
        joined = false;
    }
    // Whitespace, slashes and the `>`.
    let tail = &src[tag.attrs[tag.attrs.len() - 1].span.end..];
    if joined && end == TagEnd::Unquoted && tail.starts_with('/') {
        out.push(' ');
    }
    out.push_str(tail);
    Some(out)
}

fn dangerous_attr(attr: &Attr<'_>) -> bool {
    let name = attr.name.to_ascii_lowercase();
    if name.starts_with("on") || name == "srcdoc" {
        return true;
    }
    let Some(value) = attr.value else {
        return false;
    };
    let value = decode_entities(value);
    if URL_ATTRS.contains(&name.as_str()) {
        dangerous_url(&value)
    } else if name == "style" {
        dangerous_style(&value)
    } else if ANIMATION_ATTRS.contains(&name.as_str()) {
        let squeezed = squeeze(&value);
        squeezed.contains("javascript:")
            || squeezed.contains("vbscript:")
            || value.split(';').any(dangerous_url)
    } else if name == "attributename" {
        value.trim().to_ascii_lowercase().starts_with("on")
    } else {
        false
    }
}

/// Whether a (decoded) `style` value could run script: `expression(`, `javascript:` and the like.
/// The sanitizer drops such a `style` attribute.
pub(crate) fn dangerous_style(value: &str) -> bool {
    let css = squeeze(&css_unescape(value));
    [
        "expression(",
        "javascript:",
        "vbscript:",
        "behavior:",
        "-moz-binding",
    ]
    .iter()
    .any(|bad| css.contains(bad))
}

/// Lowercase, without whitespace, control characters and CSS comments.
fn squeeze(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .find("*/")
            .map_or("", |end| &rest[start + 2 + end + 2..]);
    }
    out.push_str(rest);
    out.retain(|c| !c.is_whitespace() && !c.is_control());
    out.to_lowercase()
}

/// CSS escapes decoded: `\` + up to six hex digits (and one following whitespace) is that
/// character, `\` + a line break nothing, `\` + any other character that character.
fn css_unescape(s: &str) -> Cow<'_, str> {
    if !s.contains('\\') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut code = 0u32;
        let mut digits = 0;
        while digits < 6
            && let Some(d) = chars.peek().and_then(|c| c.to_digit(16))
        {
            code = code * 16 + d;
            digits += 1;
            chars.next();
        }
        if digits > 0 {
            out.push(
                char::from_u32(code)
                    .filter(|&c| c != '\0')
                    .unwrap_or('\u{fffd}'),
            );
            if chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
        } else if let Some(next) = chars.next()
            && next != '\n'
        {
            out.push(next);
        }
    }
    Cow::Owned(out)
}

/// Named character references decoded when checking attribute values: the ASCII punctuation,
/// whitespace and `&nbsp;`. Over-decoding is harmless there (the value is only inspected).
const NAMED: [(&str, char); 39] = [
    ("amp", '&'),
    ("apos", '\''),
    ("ast", '*'),
    ("bsol", '\\'),
    ("colon", ':'),
    ("comma", ','),
    ("commat", '@'),
    ("dollar", '$'),
    ("equals", '='),
    ("excl", '!'),
    ("grave", '`'),
    ("gt", '>'),
    ("hat", '^'),
    ("lbrace", '{'),
    ("lbrack", '['),
    ("lcub", '{'),
    ("lowbar", '_'),
    ("lpar", '('),
    ("lsqb", '['),
    ("lt", '<'),
    ("midast", '*'),
    ("nbsp", '\u{a0}'),
    ("newline", '\n'),
    ("num", '#'),
    ("percnt", '%'),
    ("period", '.'),
    ("plus", '+'),
    ("quest", '?'),
    ("quot", '"'),
    ("rbrace", '}'),
    ("rbrack", ']'),
    ("rcub", '}'),
    ("rpar", ')'),
    ("rsqb", ']'),
    ("semi", ';'),
    ("sol", '/'),
    ("tab", '\t'),
    ("verbar", '|'),
    ("vert", '|'),
];

/// Character references in an attribute value decoded: numeric ones (with or without `;`) and
/// the [`NAMED`] ones (ignoring case). Others stay as written.
pub(crate) fn decode_entities(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        match decode_reference(after) {
            Some((c, len)) => {
                out.push(c);
                rest = &after[len..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// The character of the reference at the start of `s` (after its `&`) and its length.
fn decode_reference(s: &str) -> Option<(char, usize)> {
    let b = s.as_bytes();
    if b.first() == Some(&b'#') {
        let (radix, start) = match b.get(1) {
            Some(b'x' | b'X') => (16, 2),
            _ => (10, 1),
        };
        let digits = s[start..]
            .bytes()
            .take_while(|d| char::from(*d).is_digit(radix))
            .count();
        if digits == 0 {
            return None;
        }
        let code = u32::from_str_radix(&s[start..start + digits], radix).unwrap_or(u32::MAX);
        let c = char::from_u32(code)
            .filter(|&c| c != '\0')
            .unwrap_or('\u{fffd}');
        let mut len = start + digits;
        if b.get(len) == Some(&b';') {
            len += 1;
        }
        return Some((c, len));
    }
    let name_len = s.bytes().take_while(u8::is_ascii_alphanumeric).count();
    let name = &s[..name_len];
    let (_, c) = NAMED.iter().find(|(n, _)| n.eq_ignore_ascii_case(name))?;
    let len = name_len + usize::from(b.get(name_len) == Some(&b';'));
    Some((*c, len))
}

/// HTML comments removed (an unclosed one to the end).
pub(crate) fn strip_comments(html: &str) -> Cow<'_, str> {
    if !html.contains("<!--") {
        return Cow::Borrowed(html);
    }
    let mut out = String::with_capacity(html.len());
    let mut changed = false;
    for token in tokens(html) {
        match token {
            Token::Comment { .. } => changed = true,
            token => out.push_str(token_src(&token)),
        }
    }
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(html)
    }
}

fn token_src<'a>(token: &Token<'a>) -> &'a str {
    match token {
        Token::Text(s) | Token::Bogus(s) | Token::Unclosed(s) | Token::Comment { src: s, .. } => s,
        Token::Tag(tag) => tag.src,
    }
}

/// Elements that start and end their own line (block-level by default).
const BLOCK_ELEMENTS: [&str; 57] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "legend",
    "li",
    "main",
    "menu",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "pre",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
    "listing",
];

/// Elements without content (no closing tag).
const VOID_ELEMENTS: [&str; 15] = [
    "area", "base", "br", "col", "embed", "frame", "hr", "img", "input", "keygen", "link", "meta",
    "param", "source", "track",
];

/// Whether an element is block-level (ignoring ASCII case).
pub(crate) fn is_block_element(name: &str) -> bool {
    BLOCK_ELEMENTS.iter().any(|e| e.eq_ignore_ascii_case(name))
}

fn is_void_element(name: &str) -> bool {
    VOID_ELEMENTS.iter().any(|e| e.eq_ignore_ascii_case(name)) || name.eq_ignore_ascii_case("wbr")
}

/// What a raw HTML block is for the separators: the kind it starts with and the kind it ends
/// with, from the first and last tag or non-blank text of the HTML the forward direction writes
/// for it (comments removed, then sanitized; so any raw HTML can be passed).
///
/// | First or last | Start | End |
/// |---|---|---|
/// | closing tag of a block-level element (`</div>`) | [`BlockKind::Close`] | [`BlockKind::Box`] |
/// | closing tag of another element (`</span>`) | alone: `Close`, else [`BlockKind::Text`] | `Text` |
/// | opening tag of a block-level element with content (`<div …>`, `<div/>`) | `Box` | [`BlockKind::Open`] |
/// | opening tag of another element with content (`<span …>`, `<a/>`) | `Text` | alone: `Open`, else `Text` |
/// | void or self-closed element (`<hr>`, `<img>`, `<svg/>`) | block-level: `Box`, else `Text` | the same |
/// | `<br>` (`</br>` too, which is one to a browser) | `Text` | `Box` |
/// | text | `Text` | `Text` |
///
/// So no break is written just inside a wrapper: after the tag that opens it, before the one
/// that closes it. An inline tag next to other content of its block counts as text: with
/// `</span>\nmore` the block before and `more` would otherwise run together on one line. HTML
/// ignores `/>` on elements with content (`<div/>` opens a div), except inside `svg` and `math`.
pub(crate) fn raw_block_kinds(html: &str) -> (BlockKind, BlockKind) {
    let stripped = strip_comments(html);
    let clean = sanitize_raw_html(&stripped);
    let mut first = None;
    let mut last = Role::Text;
    let mut count = 0;
    // `svg` and `math` elements opened and not closed yet: inside them `/>` closes an element.
    let mut foreign = 0usize;
    for token in tokens(&clean) {
        let role = match &token {
            Token::Comment { .. } | Token::Bogus(_) => continue,
            Token::Text(s) if s.trim().is_empty() => continue,
            Token::Tag(tag) => {
                let role = Role::of(tag, foreign > 0);
                if tag.is("svg") || tag.is("math") {
                    if tag.closing {
                        foreign = foreign.saturating_sub(1);
                    } else if !tag.self_closing {
                        foreign += 1;
                    }
                }
                role
            }
            Token::Text(_) | Token::Unclosed(_) => Role::Text,
        };
        count += 1;
        first.get_or_insert(role);
        last = role;
    }
    let alone = count == 1;
    (first.unwrap_or(Role::Text).start(alone), last.end(alone))
}

/// What a token of a raw block is for [`raw_block_kinds`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Text,
    Br,
    Closing {
        block: bool,
    },
    /// `opens`: the element takes the following content (not void, not self-closed).
    Opening {
        block: bool,
        opens: bool,
    },
}

impl Role {
    fn of(tag: &Tag<'_>, in_foreign: bool) -> Role {
        let block = is_block_element(tag.name);
        if tag.is("br") {
            Role::Br
        } else if tag.closing {
            Role::Closing { block }
        } else {
            let self_closed = tag.self_closing && (in_foreign || tag.is("svg") || tag.is("math"));
            Role::Opening {
                block,
                opens: !is_void_element(tag.name) && !self_closed,
            }
        }
    }

    /// The kind of a block starting with this token (`alone`: the block's only one).
    fn start(self, alone: bool) -> BlockKind {
        match self {
            Role::Closing { block } if block || alone => BlockKind::Close,
            Role::Opening { block: true, .. } => BlockKind::Box,
            _ => BlockKind::Text,
        }
    }

    /// The kind of a block ending with this token.
    fn end(self, alone: bool) -> BlockKind {
        match self {
            Role::Opening { block, opens: true } if block || alone => BlockKind::Open,
            Role::Br
            | Role::Closing { block: true }
            | Role::Opening {
                block: true,
                opens: false,
            } => BlockKind::Box,
            _ => BlockKind::Text,
        }
    }
}

/// Whether a link is open after `html`, given whether one was open before it: `<a …>` opens one
/// (`<a/>` too, HTML ignores the `/` there; an open one is closed first), `</a>` closes it.
pub(crate) fn link_open_after(html: &str, open: bool) -> bool {
    tokens(html).fold(open, |open, token| match token {
        Token::Tag(tag) if tag.is("a") => !tag.closing,
        _ => open,
    })
}

/// Append HTML to existing rich text as a new paragraph: trailing whitespace, `&nbsp;` and `<br>`
/// of `existing` are dropped, then one `<br/>` follows a closing block tag or `<hr>`, two follow
/// anything else. Existing rich text that is blank that way is replaced; a blank `addition`
/// changes nothing.
pub(crate) fn append_html(existing: &str, addition: &str) -> String {
    if addition.trim().is_empty() {
        return existing.to_owned();
    }
    let existing = trim_trailing_breaks(existing);
    if existing.is_empty() {
        return addition.to_owned();
    }
    let separator = if ends_with_block(existing) {
        BR
    } else {
        "<br/><br/>"
    };
    let mut out = String::with_capacity(existing.len() + separator.len() + addition.len());
    out.push_str(existing);
    out.push_str(separator);
    out.push_str(addition);
    out
}

/// The tag `html` ends with, if it ends with one.
fn last_tag(html: &str) -> Option<Tag<'_>> {
    let start = html.rfind('<')?;
    match markup_at(html, start) {
        Some((Token::Tag(tag), end)) if end == html.len() => Some(tag),
        _ => None,
    }
}

/// `html` without trailing whitespace (U+00A0 included), non-breaking space references and
/// `<br>` tags.
fn trim_trailing_breaks(mut html: &str) -> &str {
    loop {
        let trimmed = html.trim_end();
        let nbsp = ["&nbsp;", "&#160;", "&#xa0;"].into_iter().find(|nbsp| {
            trimmed.len() >= nbsp.len()
                && trimmed
                    .get(trimmed.len() - nbsp.len()..)
                    .is_some_and(|t| t.eq_ignore_ascii_case(nbsp))
        });
        let trimmed = if let Some(nbsp) = nbsp {
            &trimmed[..trimmed.len() - nbsp.len()]
        } else if let Some(tag) = last_tag(trimmed).filter(|t| t.is("br")) {
            &trimmed[..trimmed.len() - tag.src.len()]
        } else {
            trimmed
        };
        if trimmed.len() == html.len() {
            return html;
        }
        html = trimmed;
    }
}

fn ends_with_block(html: &str) -> bool {
    last_tag(html)
        .is_some_and(|t| (t.closing && is_block_element(t.name)) || (!t.closing && t.is("hr")))
}

/// Plain text (a `text/plain` description) as HTML: escaped, line breaks as `<br/>`.
pub(crate) fn text_to_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push_str(BR);
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        for (j, part) in line.split('\r').enumerate() {
            if j > 0 {
                out.push_str(BR);
            }
            push_text(&mut out, part);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping() {
        assert_eq!(escape_text("plain"), "plain");
        assert!(matches!(escape_text("plain"), Cow::Borrowed(_)));
        assert_eq!(
            escape_text("a & b < c > d \"e\" 'f'"),
            "a &amp; b &lt; c &gt; d \"e\" 'f'"
        );
        assert_eq!(
            escape_attr("a & b < c > d \"e\" 'f'"),
            "a &amp; b &lt; c &gt; d &quot;e&quot; 'f'"
        );
        assert_eq!(escape_text("x\u{a0}y\0"), "x&nbsp;y\u{fffd}");
        assert_eq!(escape_attr("\u{a0}"), "&nbsp;");
        assert_eq!(escape_text("äöü € 😀"), "äöü € 😀");
    }

    #[test]
    fn dangerous_urls() {
        for url in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "  javascript:alert(1)",
            "\u{1}javascript:x",
            "java\tscript:x",
            "java\nscript:x",
            "vbscript:x",
            "file:///etc/passwd",
            "data:text/html,x",
            "data:image/svg+xml,x",
            "java\nscr\ript:x",
            "FILE:x",
            "data:",
        ] {
            assert!(dangerous_url(url), "{url:?}");
        }
        for url in [
            "http://example.com",
            "https://x/javascript:",
            "workitemimg:1-a.png",
            "data:image/png;base64,AAAA",
            "DATA:IMAGE/WEBP,x",
            "data:image/jpeg,x",
            "data:image/gif,x",
            "mailto:a@b.c",
            "#x",
            "relative/path",
            "relative/file:x",
            "\u{a0}javascript:x",
            "",
        ] {
            assert!(!dangerous_url(url), "{url:?}");
        }
    }

    #[test]
    fn percent_coding() {
        assert_eq!(
            percent_encode("1-mermaid-0123456789abcdef.svg"),
            "1-mermaid-0123456789abcdef.svg"
        );
        assert_eq!(
            percent_encode("2-my image (1).png"),
            "2-my%20image%20%281%29.png"
        );
        assert_eq!(percent_encode("3-ä\"&.png"), "3-%C3%A4%22%26.png");
        assert_eq!(
            percent_decode("2-my%20image%20%281%29.png"),
            "2-my image (1).png"
        );
        assert_eq!(percent_decode("3-%C3%A4%22%26.png"), "3-ä\"&.png");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
        assert_eq!(percent_decode("%FF"), "\u{fffd}");
        for id in ["a b", "ä/ö?#%", "😀", "%20"] {
            assert_eq!(percent_decode(&percent_encode(id)), id);
        }
    }

    fn tag(html: &str) -> Tag<'_> {
        match tokens(html).next() {
            Some(Token::Tag(tag)) => tag,
            other => panic!("{html}: {other:?}"),
        }
    }

    #[test]
    fn tokenizer() {
        let html = "a<b>c</b><!-- x --><!x><?y?></ z>< d<e";
        let src: Vec<String> = tokens(html).map(|t| token_src(&t).to_owned()).collect();
        assert_eq!(
            src,
            [
                "a",
                "<b>",
                "c",
                "</b>",
                "<!-- x -->",
                "<!x>",
                "<?y?>",
                "</ z>",
                "< d",
                "<e"
            ]
        );
        assert!(matches!(tokens("<e").next(), Some(Token::Unclosed("<e"))));
        assert!(matches!(
            tokens("<!--x").next(),
            Some(Token::Comment { closed: false, .. })
        ));
        assert!(matches!(
            tokens("<!-->").next(),
            Some(Token::Comment { closed: true, .. })
        ));
        assert!(matches!(
            tokens("<!--x--!>y").next(),
            Some(Token::Comment {
                src: "<!--x--!>",
                ..
            })
        ));
        let src: Vec<Token<'_>> = tokens("a < b <3 </").collect();
        assert_eq!(
            src,
            [
                Token::Text("a "),
                Token::Text("< b "),
                Token::Text("<3 "),
                Token::Unclosed("</")
            ]
        );

        let t = tag(r#"<IMG src="a>b" alt='x' data-x=1 checked/>"#);
        assert!(t.is("img") && !t.closing && t.self_closing);
        let attrs: Vec<(&str, Option<&str>)> = t.attrs.iter().map(|a| (a.name, a.value)).collect();
        assert_eq!(
            attrs,
            [
                ("src", Some("a>b")),
                ("alt", Some("x")),
                ("data-x", Some("1")),
                ("checked", None)
            ]
        );
        assert_eq!(&t.src[t.attrs[0].span.clone()], r#" src="a>b""#);
        // Browser rules: `/` separates attributes, a quoted value ends an attribute.
        let t = tag("<img/src=x/onerror=y>");
        assert_eq!(t.attrs.iter().map(|a| a.name).collect::<Vec<_>>(), ["src"]);
        assert_eq!(t.attrs[0].value, Some("x/onerror=y"));
        let t = tag(r#"<a b="1"c='2' =d e = f>"#);
        assert_eq!(
            t.attrs
                .iter()
                .map(|a| (a.name, a.value))
                .collect::<Vec<_>>(),
            [
                ("b", Some("1")),
                ("c", Some("2")),
                ("=d", None),
                ("e", Some("f"))
            ]
        );
        let t = tag("<a href=x/>");
        assert!(!t.self_closing);
        assert!(tag("</div >").closing);
        assert!(matches!(
            tokens(r#"<a href="x>"#).next(),
            Some(Token::Unclosed(_))
        ));
        assert_eq!(
            tag(r#"<a href="&#106;s">"#).attrs[0]
                .value
                .map(decode_entities)
                .as_deref(),
            Some("js")
        );
    }

    #[test]
    fn entities() {
        assert_eq!(decode_entities("a&amp;b"), "a&b");
        assert_eq!(decode_entities("&#106;&#x61;&#X76;&#97"), "java");
        assert_eq!(
            decode_entities("javascript&colon;x&Tab;&NewLine;"),
            "javascript:x\t\n"
        );
        assert_eq!(
            decode_entities("&unknown; & &#; &#x;"),
            "&unknown; & &#; &#x;"
        );
        assert_eq!(decode_entities("&#0;&#x110000;"), "\u{fffd}\u{fffd}");
        assert_eq!(decode_entities("&#99999999999999999999;"), "\u{fffd}");
    }

    #[test]
    fn sanitizer_keeps_clean_html() {
        for html in [
            "<b>bold</b>",
            r#"<span style="color: #FF0000;">red</span>"#,
            r#"<a href="https://example.com" target="_blank">x</a>"#,
            r#"<img src="workitemimg:1-a.png" style="width: 100px;height: 74px;">"#,
            "<div style=\"text-align: center;\">\n<table><tr><td>x</td></tr></table>\n</div>",
            "<!-- comment --> <!DOCTYPE html> <?pi?> a < b",
            "<onion>x</onion>",
            r#"<p title="onclick">x</p>"#,
        ] {
            let clean = sanitize_raw_html(html);
            assert!(matches!(clean, Cow::Borrowed(_)), "{html}");
            assert_eq!(clean, html);
        }
    }

    #[test]
    fn sanitizer_fixes_dangerous_html() {
        let cases = [
            (r#"<img src="x" onerror="alert(1)">"#, r#"<img src="x">"#),
            ("<img src=x onerror=alert(1)//>", "<img src=x>"),
            ("<IMG SRC=x ONLOAD=y>", "<IMG SRC=x>"),
            (
                "<img/src=x/onerror=alert(1)>",
                "<img/src=x/onerror=alert(1)>",
            ),
            ("<svg/onload=alert(1)>", "<svg>"),
            (r#"<a href="javascript:alert(1)">x</a>"#, "<a>x</a>"),
            (
                r#"<a href=" JavaScript:alert(1)" title="t">x</a>"#,
                r#"<a title="t">x</a>"#,
            ),
            (r#"<a href="&#106;avascript:alert(1)">x</a>"#, "<a>x</a>"),
            (r#"<a href="javascript&colon;alert(1)">x</a>"#, "<a>x</a>"),
            (r#"<a href="java&#x09;script:alert(1)">x</a>"#, "<a>x</a>"),
            (r#"<a href="vbscript:x">x</a>"#, "<a>x</a>"),
            (r#"<img src="data:text/html;base64,PHNjcmlwdD4=">"#, "<img>"),
            (
                r#"<form action="javascript:x"><button formaction="javascript:y">b</button></form>"#,
                "<form><button>b</button></form>",
            ),
            (
                r#"<svg><a xlink:href="javascript:x">y</a></svg>"#,
                "<svg><a>y</a></svg>",
            ),
            (
                r#"<x-object data="javascript:x"></x-object>"#,
                "<x-object></x-object>",
            ),
            (
                r#"<iframe srcdoc="<script>x</script>"></iframe>"#,
                r#"&lt;iframe srcdoc="&lt;script>x&lt;/script>">&lt;/iframe>"#,
            ),
            // A filtered tag is text as a whole, tags in its quoted values included.
            (
                r#"<script a="<img src=x onerror=alert(1)>">"#,
                r#"&lt;script a="&lt;img src=x onerror=alert(1)>">"#,
            ),
            (
                r#"<img src="x <script>alert(1)</script>"#,
                r#"&lt;img src="x &lt;script>alert(1)&lt;/script>"#,
            ),
            (
                "<!-- <script>x</script>",
                "&lt;!-- &lt;script>x&lt;/script>",
            ),
            ("a </", "a &lt;/"),
            (r#"<div srcdoc="x">y</div>"#, "<div>y</div>"),
            (
                r#"<p style="width: expression(alert(1));">x</p>"#,
                "<p>x</p>",
            ),
            (
                r#"<p style="background: url(JAVASCRIPT:alert(1))">x</p>"#,
                "<p>x</p>",
            ),
            (
                r#"<p style="background: url(j\61vascript:alert(1))">x</p>"#,
                "<p>x</p>",
            ),
            (r#"<p style="x: ex/**/pression(1)">x</p>"#, "<p>x</p>"),
            (
                r#"<svg><animate attributeName="href" values="javascript:alert(1)"/></svg>"#,
                r#"<svg><animate attributeName="href"/></svg>"#,
            ),
            (
                r#"<svg><set attributeName="onmouseover" to="alert(1)"/></svg>"#,
                r#"<svg><set to="alert(1)"/></svg>"#,
            ),
            (
                "<script>alert(1)</script>",
                "&lt;script>alert(1)&lt;/script>",
            ),
            ("<STYLE>x</STYLE>", "&lt;STYLE>x&lt;/STYLE>"),
            (
                "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\">",
                "&lt;noscript><p title=\"</noscript><img src=x onerror=alert(1)>\">",
            ),
            (
                "<textarea>x</textarea><title>t</title><xmp>x</xmp>",
                "&lt;textarea>x&lt;/textarea>&lt;title>t&lt;/title>&lt;xmp>x&lt;/xmp>",
            ),
            ("<plaintext>x", "&lt;plaintext>x"),
            (
                "<noembed>x</noembed><noframes>y</noframes>",
                "&lt;noembed>x&lt;/noembed>&lt;noframes>y&lt;/noframes>",
            ),
            (
                r#"<object data="https://x/y.html"></object><embed src="https://x/z">"#,
                r#"&lt;object data="https://x/y.html">&lt;/object>&lt;embed src="https://x/z">"#,
            ),
            (
                r#"<base href="https://x/"><link rel="stylesheet" href="https://x/a.css">"#,
                r#"&lt;base href="https://x/">&lt;link rel="stylesheet" href="https://x/a.css">"#,
            ),
            (
                r#"<meta http-equiv="refresh" content="0; url=https://x/">"#,
                r#"&lt;meta http-equiv="refresh" content="0; url=https://x/">"#,
            ),
            (
                r#"<frameset><frame src="https://x/"></frameset>"#,
                r#"&lt;frameset>&lt;frame src="https://x/">&lt;/frameset>"#,
            ),
            (
                "<img src=x onerror=alert(1) ",
                "&lt;img src=x onerror=alert(1) ",
            ),
            (r#"<a href="x"#, r#"&lt;a href="x"#),
            ("<!-- open", "&lt;!-- open"),
            ("<!x", "&lt;!x"),
            ("ok <b>b</b> <i onclick=x>i</i>", "ok <b>b</b> <i>i</i>"),
            // Removing attributes must not join what is left into something a browser reads
            // differently: the tag name and the next attribute, an unquoted value and the next
            // attribute or `/`, an attribute without a value and ` =x` (its value then).
            (
                r#"<a href=java onclick='x'script:alert(1)>"#,
                r#"<a href=java script:alert(1)>"#,
            ),
            (r#"<a onclick="x"href=y>"#, r#"<a href=y>"#),
            (
                r#"<input =x=a/b / lowsrc="data:text/html,x"/title=onclick=x>"#,
                r#"<input =x=a/b /title=onclick=x>"#,
            ),
            (r#"<a href=x onclick="y"/>"#, r#"<a href=x />"#),
            (r#"<a b onclick="y"/>"#, r#"<a b/>"#),
            ("<svg values//srcdoc= src =javascript:x/>", "<svg values/>"),
            (
                "<form lowsrc srcdoc=x = 'javascript&colon;x'/>",
                "<form lowsrc 'javascript&colon;x'/>",
            ),
            (r#"<a b onclick=x /=c>"#, r#"<a b>"#),
            (r#"<a onclick=x =c>"#, r#"<a =c>"#),
            ("<a\ton = \"a\"b\"srcdoc=on>", "<a b\"srcdoc=on>"),
            // Attributes next to each other in the source stay as they were.
            (
                r#"<img a="1"b="2" onerror=x c='3'd>"#,
                r#"<img a="1"b="2" c='3'd>"#,
            ),
        ];
        for (html, expected) in cases {
            assert_eq!(sanitize_raw_html(html), expected, "{html}");
            // Sanitizing is idempotent.
            assert_eq!(sanitize_raw_html(expected), expected, "{expected}");
        }
    }

    /// Bytes the scan of raw HTML may look at again: none. Unclosed tags, unclosed quoted values
    /// and filtered tags are text to the end of what they cover, so their content is not scanned
    /// again.
    #[test]
    fn sanitizing_takes_linear_time() {
        let n = 20_000;
        for (html, unit) in [
            ("<a".repeat(n), "&lt;a"),
            ("<!x".repeat(n), "&lt;!x"),
            ("</ ".repeat(n), "&lt;/ "),
            ("<!--".repeat(n), "&lt;!--"),
            ("<b ".repeat(n) + "x=\">", "&lt;b "),
            ("<script x=\"".repeat(n) + "\">", "&lt;script x=\""),
        ] {
            let started = std::time::Instant::now();
            let clean = sanitize_raw_html(&html);
            let took = started.elapsed();
            assert!(clean.starts_with(&unit.repeat(100)), "{}", &clean[..100]);
            assert!(!clean.contains('<'), "{}", &clean[..100]);
            // Quadratic, this took seconds even in release builds.
            assert!(took < std::time::Duration::from_secs(1), "{unit}: {took:?}");
        }
    }

    #[test]
    fn comments() {
        assert_eq!(strip_comments("a<!-- b -->c"), "ac");
        assert_eq!(strip_comments("a<!-- b"), "a");
        assert_eq!(strip_comments("<!---->x<!-->y<!--->z"), "xyz");
        assert_eq!(
            strip_comments(r#"<p title="<!-- x -->">y</p>"#),
            r#"<p title="<!-- x -->">y</p>"#
        );
        assert!(matches!(strip_comments("no comment"), Cow::Borrowed(_)));
        // What a removed comment joins is sanitized afterwards.
        let joined = strip_comments("<<!-- -->script>x");
        assert_eq!(sanitize_raw_html(&joined), "&lt;script>x");
    }

    #[test]
    fn raw_block_kinds_by_tags() {
        use BlockKind::*;
        let cases = [
            ("<div style=\"x\">", (Box, Open)),
            ("</div>", (Close, Box)),
            ("<span style=\"color: red;\">", (Text, Open)),
            ("</span>", (Close, Text)),
            ("<div>x</div>", (Box, Box)),
            ("<table><tr><td>x</td></tr></table>", (Box, Box)),
            ("<div>x</div>\ntext", (Box, Text)),
            ("<span>x</span>", (Text, Text)),
            ("<img src=\"x\">", (Text, Text)),
            ("<img src=\"x\"><br>", (Text, Box)),
            ("<hr>", (Box, Box)),
            ("<hr/>", (Box, Box)),
            ("<br>", (Text, Box)),
            ("<pre>\nx\n</pre>", (Box, Box)),
            ("&lt;script>x", (Text, Text)),
            ("<!DOCTYPE html>", (Text, Text)),
            ("<p>a</p>  \n ", (Box, Box)),
            // A closing tag first: it ends the element the blocks before are in; an opening tag
            // last: the blocks after are its content. Inline ones only on their own: next to
            // other content they are text, or two paragraphs would run together.
            ("</div>\ntext", (Close, Text)),
            ("</div>\n<div style=\"y\">", (Close, Open)),
            ("text <div style=\"y\">", (Text, Open)),
            ("</div>\n</div>", (Close, Box)),
            ("</span> more", (Text, Text)),
            ("</span>\n<div style=\"y\">", (Text, Open)),
            ("<div>x</div>\n<span style=\"y\">", (Box, Text)),
            ("</div>\n<span style=\"y\">", (Close, Text)),
            // HTML ignores `/>` on elements with content, except in SVG and MathML.
            ("<div/>", (Box, Open)),
            ("<a/>", (Text, Open)),
            ("<svg/>", (Text, Text)),
            ("<svg><circle r=\"1\"/>", (Text, Text)),
            ("<svg><circle r=\"1\"/></svg><div/>", (Text, Open)),
            ("<math/>", (Text, Text)),
            ("<br/>", (Text, Box)),
            ("</br>", (Text, Box)),
            ("<img src=\"x\"/>", (Text, Text)),
            // Filtered tags are text once sanitized.
            ("<script>", (Text, Text)),
            ("</style>\n<div>", (Text, Open)),
            // Comments and bogus markup do not count; what a removed comment joins does.
            ("<!-- c --></div><!-- d -->", (Close, Box)),
            ("<?pi?><div style=\"x\"><!x>", (Box, Open)),
            ("</div><<!-- c -->text<hr>", (Close, Text)),
            ("<div><<!-- c -->x>", (Box, Text)),
            ("<<!-- c -->div>", (Box, Open)),
        ];
        for (html, kinds) in cases {
            assert_eq!(raw_block_kinds(html), kinds, "{html}");
        }
    }

    /// The kinds do not change when sanitizing and removing comments.
    #[test]
    fn raw_block_kinds_survive_sanitizing() {
        const PIECES: [&str; 22] = [
            "<div>",
            "</div>",
            "<span style=\"x\">",
            "</span>",
            "<div/>",
            "<br>",
            "<hr>",
            "<script>",
            "</script>",
            "<svg>",
            "</svg>",
            "<svg/>",
            "<b/>",
            "<!-- c -->",
            "<!--",
            "<a onclick=x>",
            "<img src=x onerror=y",
            "text",
            " ",
            "\n",
            "<",
            "<!x>",
        ];
        let mut rng = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..20_000 {
            let mut html = String::new();
            for _ in 0..1 + rng % 6 {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                html.push_str(PIECES[(rng % PIECES.len() as u64) as usize]);
            }
            let clean = sanitize_raw_html(&strip_comments(&html)).into_owned();
            assert_eq!(raw_block_kinds(&clean), raw_block_kinds(&html), "{html:?}");
        }
    }

    #[test]
    fn links_open_and_close() {
        assert!(link_open_after("<a href=\"u\">", false));
        assert!(link_open_after("<A HREF=u>x", false));
        assert!(link_open_after("<a/>", false));
        assert!(!link_open_after("</a>", true));
        assert!(!link_open_after("<a href=\"u\">x</a>", false));
        assert!(link_open_after("</a><a href=\"v\">", true));
        assert!(link_open_after("<b>x</b>", true));
        assert!(!link_open_after("<b>x</b>", false));
        // A second `<a>` closes the first: one `</a>` closes both.
        assert!(!link_open_after("<a href=1><a href=2>x</a>", false));
        // Inside attribute values and comments there are no tags.
        assert!(!link_open_after(
            "<span title=\"<a href=u>\"><!-- <a> -->",
            false
        ));
        assert!(!link_open_after("&lt;a href=u>", false));
        assert!(!link_open_after("<abbr>", false));
    }

    #[test]
    fn xml_safety() {
        for c in [
            '\t', '\n', '\r', ' ', 'a', '\u{a0}', '\u{d7ff}', '\u{e000}', '\u{fffd}',
        ] {
            assert!(xml_allows(c), "{c:?}");
        }
        for c in ['\u{10000}', '😀', '\u{10ffff}'] {
            assert!(xml_allows(c), "{c:?}");
        }
        for c in [
            '\0', '\u{1}', '\u{8}', '\u{b}', '\u{c}', '\u{e}', '\u{1b}', '\u{1f}',
        ] {
            assert!(!xml_allows(c), "{c:?}");
        }
        assert!(!xml_allows('\u{fffe}') && !xml_allows('\u{ffff}'));
        assert_eq!(
            xml_safe("a\0b\u{1b}[0m\u{b}\u{fffe}\u{ffff}\t\r\n"),
            "a\u{fffd}b\u{fffd}[0m\u{fffd}\u{fffd}\u{fffd}\t\r\n"
        );
        assert!(matches!(xml_safe("äöü 😀\t\n"), Cow::Borrowed(_)));
        assert_eq!(escape_text("\u{1}<\u{ffff}"), "\u{fffd}&lt;\u{fffd}");
        assert_eq!(escape_attr("\u{1b}\""), "\u{fffd}&quot;");
        assert_eq!(text_to_html("a\u{7}b"), "a\u{fffd}b");
        // The form feed separates attributes like a space: it becomes one.
        assert_eq!(xml_safe("<a\u{c}b=c>\u{c}"), "<a b=c> ");
        assert_eq!(escape_text("a\u{c}b"), "a b");
        // What the replacements leave is split into the same tags and attributes.
        let html = "<img\u{c}src=x\u{1}y\u{c}alt=\"\u{0}\"\u{b}>t\u{1b}<b\u{c}>";
        // Each tag as `name attr=value …`, with the replacements made.
        let shape = |html: &str| -> Vec<String> {
            tokens(html)
                .filter_map(|t| match t {
                    Token::Tag(tag) => Some(
                        std::iter::once(tag.name.to_owned())
                            .chain(tag.attrs.iter().map(|a| match a.value {
                                Some(value) => format!("{}=[{value}]", a.name),
                                None => a.name.to_owned(),
                            }))
                            .map(|part| xml_safe(&part).into_owned())
                            .collect::<Vec<_>>()
                            .join(" "),
                    ),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(shape(&xml_safe(html)), shape(html));
        assert_eq!(
            shape(html),
            ["img src=[x\u{fffd}y] alt=[\u{fffd}] \u{fffd}", "b"]
        );
    }

    /// Appending converted Markdown: `helpers` in tests/forward.rs.
    #[test]
    fn appending() {
        assert_eq!(
            append_html("<b>existing</b>", "<i>new</i>"),
            "<b>existing</b><br/><br/><i>new</i>"
        );
        assert_eq!(append_html("", "<b>content</b>"), "<b>content</b>");
        assert_eq!(
            append_html("  <br/>&nbsp;\n", "<b>content</b>"),
            "<b>content</b>"
        );
        assert_eq!(
            append_html("a<br><BR />&nbsp; <br/>\n", "b"),
            "a<br/><br/>b"
        );
        assert_eq!(append_html("a\u{a0}", "b"), "a<br/><br/>b");
        assert_eq!(append_html("<div>a</div>", "b"), "<div>a</div><br/>b");
        assert_eq!(
            append_html("<table><tr><td>a</td></tr></table>\n", "b"),
            "<table><tr><td>a</td></tr></table><br/>b"
        );
        assert_eq!(
            append_html("<UL><li>a</li></UL>", "b"),
            "<UL><li>a</li></UL><br/>b"
        );
        assert_eq!(append_html("a<hr/>", "b"), "a<hr/><br/>b");
        assert_eq!(
            append_html("<span>a</span>", "b"),
            "<span>a</span><br/><br/>b"
        );
        assert_eq!(
            append_html("a&#160;&#xA0;<br class=\"x\"></br>", "b"),
            "a<br/><br/>b"
        );
        assert_eq!(append_html("&#160;", "b"), "b");
        // A blank addition changes nothing.
        assert_eq!(append_html("a", ""), "a");
        assert_eq!(append_html("a<br/>", " \n"), "a<br/>");
        assert_eq!(append_html(" <br/>", ""), " <br/>");
    }

    #[test]
    fn plain_text() {
        assert_eq!(text_to_html(""), "");
        assert_eq!(text_to_html("a < b & c"), "a &lt; b &amp; c");
        assert_eq!(
            text_to_html("one\ntwo\r\nthree\rfour\n"),
            "one<br/>two<br/>three<br/>four<br/>"
        );
        assert_eq!(text_to_html("PROJ-1 <b>"), "PROJ-1 &lt;b&gt;");
    }
}
