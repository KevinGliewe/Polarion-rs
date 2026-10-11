//! Inline CSS (`style` attributes): the declarations, the formatting Markdown expresses (bold,
//! italic, strikethrough, underline, highlight, superscript, subscript, monospace) and what stays
//! raw (colors, other fonts and sizes, alignment, ...).
//!
//! Styles inherit as in CSS: an element starts from its parent's style and applies its own
//! declarations; `font-weight: normal` and `font-style: normal` reset, text decorations only add.

use crate::html;

/// One declaration of a `style` attribute.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Decl {
    /// The property, lowercase.
    pub(super) prop: String,
    /// The value as written (trimmed, `!important` removed).
    pub(super) value: String,
}

/// The declarations of a `style` attribute in order. Comments are removed; `;` and `:` inside
/// quotes and parentheses do not split; declarations without a property or value are skipped.
pub(super) fn declarations(style: &str) -> Vec<Decl> {
    let style = without_comments(style);
    let mut out = Vec::new();
    for part in split_top(&style, ';') {
        let Some(colon) = find_top(part, ':') else {
            continue;
        };
        let prop = part[..colon].trim().to_ascii_lowercase();
        let mut value = part[colon + 1..].trim();
        if let Some(bang) = value.rfind('!')
            && value[bang + 1..].trim().eq_ignore_ascii_case("important")
        {
            value = value[..bang].trim_end();
        }
        let valid_prop = !prop.is_empty()
            && prop
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        // An unclosed quote or parenthesis would take in what follows once written again.
        if valid_prop && !value.is_empty() && balanced(value) {
            out.push(Decl {
                prop,
                value: value.to_owned(),
            });
        }
    }
    out
}

/// Whether every quote and parenthesis of a value is closed.
pub(super) fn balanced(value: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut depth = 0usize;
    for c in value.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') if depth == 0 => return false,
            (None, ')') => depth -= 1,
            _ => {}
        }
    }
    quote.is_none() && depth == 0
}

fn without_comments(style: &str) -> String {
    let mut out = String::with_capacity(style.len());
    let mut rest = style;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .find("*/")
            .map_or("", |end| &rest[start + 2 + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// `s` split at `sep` outside quotes and parentheses.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, c) if c == sep && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// The first `c` outside quotes and parentheses.
fn find_top(s: &str, c: char) -> Option<usize> {
    let first = split_top(s, c)[0];
    (first.len() < s.len()).then_some(first.len())
}

/// A value normalized for comparisons: lowercase, whitespace collapsed (none next to `,`, `(` and
/// `)`), numbers without trailing zeros, zero lengths without their unit, `#abc` as `#aabbcc`.
pub(super) fn norm(value: &str) -> String {
    let lower = value.trim().to_ascii_lowercase();
    let mut squeezed = String::with_capacity(lower.len());
    let mut space = false;
    for c in lower.chars() {
        if c.is_ascii_whitespace() {
            space = true;
            continue;
        }
        if space && !squeezed.is_empty() && !matches!(c, ',' | '(' | ')') {
            let last = squeezed.chars().next_back();
            if !matches!(last, Some(',' | '(')) {
                squeezed.push(' ');
            }
        }
        space = false;
        squeezed.push(c);
    }
    squeezed
        .split(' ')
        .map(norm_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn norm_token(token: &str) -> String {
    if let Some(hex) = token.strip_prefix('#')
        && hex.len() == 3
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return hex.chars().fold(String::from("#"), |mut s, c| {
            s.push(c);
            s.push(c);
            s
        });
    }
    let number_end = token
        .bytes()
        .position(|b| !(b.is_ascii_digit() || b == b'.' || b == b'-'))
        .unwrap_or(token.len());
    let (number, unit) = token.split_at(number_end);
    if number.is_empty() || !number.bytes().any(|b| b.is_ascii_digit()) {
        return token.to_owned();
    }
    let Ok(n) = number.parse::<f64>() else {
        return token.to_owned();
    };
    if n == 0.0 && unit.bytes().all(|b| b.is_ascii_alphabetic() || b == b'%') {
        return "0".to_owned();
    }
    let number = if number.contains('.') {
        number.trim_end_matches('0').trim_end_matches('.')
    } else {
        number
    };
    format!("{number}{unit}")
}

/// A length in a unit (`14pt`, `14.5pt`), if the value is one.
fn length(value: &str, unit: &str) -> Option<f32> {
    let value = value.trim();
    let number = value
        .len()
        .checked_sub(unit.len())
        .filter(|&end| value.is_char_boundary(end))
        .filter(|&end| value[end..].eq_ignore_ascii_case(unit))
        .map(|end| value[..end].trim_end())?;
    number
        .parse::<f32>()
        .ok()
        .filter(|n| n.is_finite() && *n >= 0.0)
}

/// A size in pt.
pub(super) fn pt(value: &str) -> Option<f32> {
    length(value, "pt")
}

/// A length in px.
pub(super) fn px(value: &str) -> Option<f32> {
    length(value, "px")
}

fn weight(value: &str) -> Option<bool> {
    match norm(value).as_str() {
        "bold" | "bolder" => Some(true),
        "normal" | "lighter" => Some(false),
        v => v.parse::<u32>().ok().map(|w| w >= 600),
    }
}

/// Whether a `font-family` is monospace.
pub(super) fn is_mono(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("monospace") || lower.contains("courier")
}

/// Whether a color is the highlight yellow (`#FFFF00`, `#FF0`, `yellow`, `rgb(255, 255, 0)`).
pub(super) fn is_yellow(value: &str) -> bool {
    matches!(
        norm(value).as_str(),
        "#ffff00" | "yellow" | "rgb(255,255,0)" | "rgba(255,255,0,1)"
    )
}

/// Formatting of a run of text: what Markdown expresses, the font size, the link it is in, and
/// the raw wrappers around it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Style {
    pub(super) bold: bool,
    pub(super) italic: bool,
    pub(super) strike: bool,
    pub(super) underline: bool,
    pub(super) highlight: bool,
    pub(super) sup: bool,
    pub(super) sub: bool,
    pub(super) mono: bool,
    /// The font size in pt, when set in pt (headings are recognized by it).
    pub(super) size: Option<f32>,
    /// The size the nearest raw wrapper that writes one sets: in pt, `Some(None)` for another
    /// unit.
    pub(super) written_size: Option<Option<f32>>,
    /// The link the text is in (an index into the walk's links).
    pub(super) link: Option<usize>,
    /// The raw wrappers around the text, outermost first (indexes into the walk's wrappers).
    pub(super) wraps: Vec<usize>,
    /// The formatting set on the text in the order the elements nest it, outermost first: of
    /// equally long runs the outer one is nested outside again.
    pub(super) order: Vec<Layer>,
}

/// A piece of formatting that nests (see [`Style::order`]); what one element sets is nested in
/// this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Layer {
    Link,
    Wrap(usize),
    Italic,
    Bold,
    Strike,
    Underline,
    Highlight,
    Sup,
    Sub,
}

impl Style {
    /// Turns formatting on or off, keeping [`Style::order`].
    pub(super) fn set(&mut self, layer: Layer, on: bool) {
        match layer {
            Layer::Link | Layer::Wrap(_) => {}
            Layer::Italic => self.italic = on,
            Layer::Bold => self.bold = on,
            Layer::Strike => self.strike = on,
            Layer::Underline => self.underline = on,
            Layer::Highlight => self.highlight = on,
            Layer::Sup => self.sup = on,
            Layer::Sub => self.sub = on,
        }
        let at = self.order.iter().position(|l| *l == layer);
        match (on, at) {
            (true, None) => self.order.push(layer),
            (false, Some(at)) => {
                self.order.remove(at);
            }
            _ => {}
        }
    }

    /// Puts the text in a link (an index into the walk's links).
    pub(super) fn set_link(&mut self, link: usize) {
        self.link = Some(link);
        self.set(Layer::Link, true);
    }

    /// Where a layer is in [`Style::order`] (`usize::MAX` when it is not there).
    pub(super) fn rank(&self, layer: Layer) -> usize {
        self.order
            .iter()
            .position(|l| *l == layer)
            .unwrap_or(usize::MAX)
    }
}

/// The formatting of one element that Markdown cannot express: written as `<span style="…">`
/// around its content in keep mode.
#[derive(Debug, Clone, Default)]
pub(super) struct Wrapper {
    pub(super) decls: Vec<Decl>,
    /// A `font-size` in pt and the `line-height` going with it: consumed by a heading, raw
    /// elsewhere.
    pub(super) size: Vec<Decl>,
}

impl Wrapper {
    /// Whether [`Wrapper::open_tag`] has something to write.
    pub(super) fn writes(&self, heading: bool) -> bool {
        self.written(heading).next().is_some()
    }

    /// The declarations written: in a heading not the sizes (the heading sets its own size), else
    /// all of them.
    fn written(&self, heading: bool) -> impl Iterator<Item = &Decl> {
        let size: &[Decl] = if heading { &[] } else { &self.size };
        self.decls
            .iter()
            .filter(move |d| !(heading && matches!(d.prop.as_str(), "font-size" | "line-height")))
            .chain(size)
    }

    /// `<span style="…">` with the declarations (the size ones too unless `heading`), or `None`
    /// when there is nothing to write.
    pub(super) fn open_tag(&self, heading: bool) -> Option<String> {
        let mut style = String::new();
        for decl in self.written(heading) {
            if !style.is_empty() {
                style.push(' ');
            }
            style.push_str(&decl.prop);
            style.push_str(": ");
            style.push_str(&decl.value);
            style.push(';');
        }
        if style.is_empty() {
            return None;
        }
        let mut tag = String::from("<span style=\"");
        tag.push_str(&html::escape_attr(&style.replace(['\n', '\r', '\t'], " ")));
        tag.push_str("\">");
        Some(tag)
    }
}

/// Applies an element's declarations to the style it inherits. What Markdown cannot express
/// becomes a wrapper (added to `wrappers`, its index to `style.wraps`).
pub(super) fn apply(style: &mut Style, decls: &[Decl], wrappers: &mut Vec<Wrapper>) {
    if decls.is_empty() {
        return;
    }
    let mut wrapper = Wrapper::default();
    let first_new = style.order.len();
    let size_pt = decls
        .iter()
        .rev()
        .find(|d| d.prop == "font-size")
        .map(|d| pt(&d.value));
    // The default size, set where no other size is written: nothing to keep.
    let default_size = size_pt == Some(Some(10.0))
        && (style.written_size.is_none() || style.written_size == Some(Some(10.0)));
    for decl in decls {
        let value = decl.value.as_str();
        let raw = match decl.prop.as_str() {
            "font-weight" => match weight(value) {
                Some(bold) => {
                    style.set(Layer::Bold, bold);
                    false
                }
                None => true,
            },
            "font-style" => match norm(value).as_str() {
                "italic" => {
                    style.set(Layer::Italic, true);
                    false
                }
                v if v.starts_with("oblique") => {
                    style.set(Layer::Italic, true);
                    false
                }
                "normal" => {
                    style.set(Layer::Italic, false);
                    false
                }
                _ => true,
            },
            "text-decoration" | "text-decoration-line" => {
                let tokens: Vec<String> = norm(value).split(' ').map(str::to_owned).collect();
                if tokens
                    .iter()
                    .all(|t| matches!(t.as_str(), "underline" | "line-through" | "none"))
                {
                    if tokens.iter().any(|t| t == "underline") {
                        style.set(Layer::Underline, true);
                    }
                    if tokens.iter().any(|t| t == "line-through") {
                        style.set(Layer::Strike, true);
                    }
                    false
                } else {
                    true
                }
            }
            "font-family" => {
                style.mono = is_mono(value);
                !style.mono
            }
            "font-size" => {
                match pt(value) {
                    Some(size) => {
                        style.size = Some(size);
                        if !default_size {
                            style.written_size = Some(Some(size));
                            wrapper.size.push(decl.clone());
                        }
                    }
                    None => {
                        style.size = None;
                        style.written_size = Some(None);
                        wrapper.decls.push(decl.clone());
                    }
                }
                false
            }
            "line-height" => {
                match size_pt {
                    Some(Some(_)) if !default_size => wrapper.size.push(decl.clone()),
                    Some(None) => wrapper.decls.push(decl.clone()),
                    // Without a size of its own it is dropped.
                    _ => {}
                }
                false
            }
            "background-color" => {
                let v = norm(value);
                if is_yellow(value) {
                    style.set(Layer::Highlight, true);
                    false
                } else {
                    !matches!(v.as_str(), "transparent" | "initial" | "inherit" | "none")
                }
            }
            "vertical-align" => match norm(value).as_str() {
                "super" => {
                    style.set(Layer::Sup, true);
                    false
                }
                "sub" => {
                    style.set(Layer::Sub, true);
                    false
                }
                "baseline" => false,
                _ => true,
            },
            _ => true,
        };
        // A declaration that could run script would make the forward sanitizer drop the whole
        // `style` of the wrapper: it is not kept.
        if raw && !html::dangerous_style(&format!("{}: {}", decl.prop, decl.value)) {
            wrapper.decls.push(decl.clone());
        }
    }
    if !wrapper.decls.is_empty() || !wrapper.size.is_empty() {
        style.wraps.push(wrappers.len());
        style.set(Layer::Wrap(wrappers.len()), true);
        wrappers.push(wrapper);
    }
    if let Some(new) = style.order.get_mut(first_new..) {
        new.sort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(style: &str) -> Vec<(String, String)> {
        declarations(style)
            .into_iter()
            .map(|d| (d.prop, d.value))
            .collect()
    }

    #[test]
    fn unclosed_values_are_skipped() {
        assert!(props(r#"e: """; color: red"#).is_empty());
        assert_eq!(
            props("color: red; x: a(b; y: 'c"),
            [("color".to_owned(), "red".to_owned())]
        );
        assert!(props("x: f(a) g)").is_empty());
    }

    #[test]
    fn declarations_in_order() {
        assert_eq!(
            props("font-weight: bold;COLOR:#FF0000 ; ;x;: y;"),
            [
                ("font-weight".to_owned(), "bold".to_owned()),
                ("color".to_owned(), "#FF0000".to_owned())
            ]
        );
        assert_eq!(
            props("font-family: 'a;b', \"c:d\"; background: url(x;y)"),
            [
                ("font-family".to_owned(), "'a;b', \"c:d\"".to_owned()),
                ("background".to_owned(), "url(x;y)".to_owned())
            ]
        );
        assert_eq!(
            props("color: red !important; /* c; */ margin: 0"),
            [
                ("color".to_owned(), "red".to_owned()),
                ("margin".to_owned(), "0".to_owned())
            ]
        );
        assert!(props("").is_empty());
        assert!(props("a b: c").is_empty());
    }

    #[test]
    fn normalized_values() {
        assert_eq!(norm(" 1px  SOLID #CCC "), "1px solid #cccccc");
        assert_eq!(norm("1280.0px"), "1280px");
        assert_eq!(norm("0px"), "0");
        assert_eq!(norm("rgb(255, 255, 0)"), "rgb(255,255,0)");
        assert_eq!(norm("1.50"), "1.5");
        assert_eq!(norm("auto"), "auto");
    }

    #[test]
    fn lengths() {
        assert_eq!(pt("14pt"), Some(14.0));
        assert_eq!(pt("14.5PT"), Some(14.5));
        assert_eq!(pt("10.5 pt"), Some(10.5));
        assert_eq!(pt("14px"), None);
        assert_eq!(pt("pt"), None);
        assert_eq!(px("40px"), Some(40.0));
        assert_eq!(px("-1px"), None);
    }

    #[test]
    fn yellow_and_mono() {
        for v in ["#FFFF00", "#ff0", "yellow", "rgb(255, 255, 0)", "YELLOW"] {
            assert!(is_yellow(v), "{v}");
        }
        assert!(!is_yellow("#FF0000"));
        assert!(is_mono("'Courier New', Courier, monospace"));
        assert!(is_mono("&#39;Courier New&#39;"));
        assert!(!is_mono("Arial, Helvetica, sans-serif"));
    }

    fn styled(decls: &str) -> (Style, Vec<Wrapper>) {
        let mut style = Style::default();
        let mut wrappers = Vec::new();
        apply(&mut style, &declarations(decls), &mut wrappers);
        (style, wrappers)
    }

    #[test]
    fn representable_styles() {
        let (s, w) = styled(
            "font-weight: bold; font-style: italic; text-decoration: underline line-through; \
             background-color: #FF0; vertical-align: super; font-family: Courier",
        );
        assert!(s.bold && s.italic && s.underline && s.strike && s.highlight && s.sup && s.mono);
        assert!(w.is_empty() && s.wraps.is_empty());
        let (s, _) = styled("font-weight: 700");
        assert!(s.bold);
        let (mut s, mut w) = styled("font-weight: bold; font-style: italic");
        apply(
            &mut s,
            &declarations("font-weight: normal; font-style: normal"),
            &mut w,
        );
        assert!(!s.bold && !s.italic);
    }

    #[test]
    fn raw_styles_become_a_wrapper() {
        let (s, w) = styled("color: #FF0000; font-weight: bold; font-family: Arial");
        assert!(s.bold && !s.mono);
        assert_eq!(s.wraps, [0]);
        assert_eq!(
            w[0].open_tag(false).unwrap(),
            "<span style=\"color: #FF0000; font-family: Arial;\">"
        );
        // A size in pt is kept apart: a heading consumes it.
        let (s, w) = styled("font-size: 12pt;line-height: 1.5;");
        assert_eq!(s.size, Some(12.0));
        assert_eq!(
            w[0].open_tag(false).unwrap(),
            "<span style=\"font-size: 12pt; line-height: 1.5;\">"
        );
        assert_eq!(w[0].open_tag(true), None);
        // 10pt is the default size; a line height alone is dropped.
        let (s, w) = styled("font-size: 10pt; line-height: 1.5");
        assert!(s.size == Some(10.0) && w.is_empty());
        let (_, w) = styled("line-height: 2");
        assert!(w.is_empty());
        // Other units stay raw and are no heading size. On a heading line a size of the text
        // inside overrides them: the heading writes its own.
        let (s, w) = styled("font-size: 16px");
        assert_eq!(s.size, None);
        assert_eq!(
            w[0].open_tag(false).unwrap(),
            "<span style=\"font-size: 16px;\">"
        );
        assert_eq!(w[0].open_tag(true), None);
        let (_, w) = styled("font-size: smaller; color: red");
        assert_eq!(w[0].open_tag(true).unwrap(), "<span style=\"color: red;\">");
        // Values with quotes are escaped for the attribute.
        let (_, w) = styled("font-family: \"Segoe UI\"");
        assert_eq!(
            w[0].open_tag(false).unwrap(),
            "<span style=\"font-family: &quot;Segoe UI&quot;;\">"
        );
    }
}
