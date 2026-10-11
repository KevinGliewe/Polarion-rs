//! Escapes comrak's CommonMark writer lacks, as `Raw` nodes of a backslash and the character (a
//! hand-built `Escaped` node writes nothing).
//!
//! comrak escapes ``* _ [ ] # < > \ ` ~ !``, `@` and, at the start of a line, `- + = : 1.`. On top
//! of that, in text:
//! - `$` and `^` (formulas, superscript);
//! - every `=` or `+` of a run of two or more, and one next to `==`/`++` delimiters (highlight,
//!   insert);
//! - the `&` of `&#` (numeric character references; comrak escapes the `#` as well);
//! - outside link text: the `.` of `www.` and the `:` of `://` (autolinks);
//! - `|` starting a line outside tables (a table's delimiter row);
//! - the hyphen of every id-shaped token (`KEY\-NUM`), whatever the setting, so plain text never
//!   becomes a work item link.
//!
//! The conservative build escapes all ASCII punctuation. Linked work item ids are written as they
//! are (`Raw`), so the forward direction links them again.

use crate::ids;

/// A piece of a run of text: text to escape, or a linked work item id.
#[derive(Debug, Clone, Copy)]
pub(super) enum Piece<'s> {
    Text(&'s str),
    Id(&'s str),
}

/// What a run is written as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Segment {
    Text(String),
    /// Written as it is: an escape (`\$`) or a linked id.
    Raw(String),
}

/// Where a run of text is.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct RunCtx {
    /// The run starts a line.
    pub(super) line_start: bool,
    /// In link text or image alternative text (no autolinks, no links to work items).
    pub(super) in_link: bool,
    /// In a table cell (comrak escapes `|` there itself).
    pub(super) cell: bool,
    /// Escape all ASCII punctuation.
    pub(super) conservative: bool,
    /// The delimiter character written right before the run (`=` of a highlight, `+` of an
    /// insert), if any.
    pub(super) before: Option<char>,
    /// The delimiter character written right after the run.
    pub(super) after: Option<char>,
}

/// A run of adjacent text pieces (the forward direction reads them as one text: work item ids
/// are found in it) as text and raw segments.
pub(super) fn run(pieces: &[Piece<'_>], ctx: &RunCtx) -> Vec<Segment> {
    let mut text = String::new();
    // Per piece: its byte range in `text` and whether it is an id.
    let mut ranges = Vec::with_capacity(pieces.len());
    for piece in pieces {
        let (s, id) = match piece {
            Piece::Text(s) => (*s, false),
            Piece::Id(s) => (*s, true),
        };
        ranges.push((text.len()..text.len() + s.len(), id));
        text.push_str(s);
    }
    // Per byte: inside an id; per id start: its end.
    let mut in_id = vec![false; text.len()];
    let mut id_end = vec![None; text.len()];
    for (range, id) in &ranges {
        if *id && !range.is_empty() {
            in_id[range.clone()].fill(true);
            id_end[range.start] = Some(range.end);
        }
    }
    let mut escaped = vec![false; text.len()];
    // A byte order mark left out.
    let mut bom = vec![false; text.len()];
    if !ctx.in_link {
        for token in ids::tokens(&text, &[]) {
            if id_end[token.start] == Some(token.end) {
                continue;
            }
            if let Some(hyphen) = text[token.clone()].find('-').map(|h| token.start + h)
                && !in_id[hyphen]
            {
                escaped[hyphen] = true;
            }
        }
    }
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        if in_id[i] {
            continue;
        }
        if c == '\u{feff}' && i == 0 && ctx.line_start {
            // A byte order mark starting a line is left out: comrak's parser drops one starting
            // its input (and HTML parsers one starting theirs), and it shows nothing.
            bom[i] = true;
            continue;
        }
        let escape = match c {
            '$' | '^' => true,
            '=' | '+' => {
                let b = c as u8;
                (i > 0 && bytes[i - 1] == b)
                    || bytes.get(i + 1) == Some(&b)
                    // comrak looks through `=` and `+` for the character next to a delimiter
                    // run: escaped, they stop it.
                    || (i == 0 && ctx.before.is_some())
                    || (i + 1 == text.len() && ctx.after.is_some())
                    // Alternating `=` and `+` make comrak's delimiter processing quadratic.
                    || (b == b'+' && ((i > 0 && bytes[i - 1] == b'=') || bytes.get(i + 1) == Some(&b'=')))
            }
            // comrak escapes the `#` too: `\&\#38;` (a numeric character reference otherwise).
            '&' => bytes.get(i + 1) == Some(&b'#'),
            '.' => !ctx.in_link && text[..i].ends_with("www"),
            ':' => !ctx.in_link && text[i + 1..].starts_with("//"),
            '|' => !ctx.cell && i == 0 && ctx.line_start,
            _ => false,
        } || (ctx.conservative && c.is_ascii_punctuation() && !(ctx.cell && c == '|'));
        if escape {
            escaped[i] = true;
        }
    }
    let mut out = Vec::new();
    for (range, id) in ranges {
        if id {
            out.push(Segment::Raw(text[range].to_owned()));
            continue;
        }
        let mut start = range.start;
        for i in range.clone() {
            if escaped[i] || bom[i] {
                if start < i {
                    out.push(Segment::Text(text[start..i].to_owned()));
                }
                if bom[i] {
                    start = i + '\u{feff}'.len_utf8();
                } else {
                    out.push(Segment::Raw(format!("\\{}", &text[i..i + 1])));
                    start = i + 1;
                }
            }
        }
        if start < range.end {
            out.push(Segment::Text(text[start..range.end].to_owned()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(pieces: &[Piece<'_>], ctx: RunCtx) -> String {
        run(pieces, &ctx)
            .into_iter()
            .map(|s| match s {
                Segment::Text(t) => t,
                Segment::Raw(r) => format!("[{r}]"),
            })
            .collect()
    }

    fn text(s: &str) -> String {
        written(&[Piece::Text(s)], RunCtx::default())
    }

    #[test]
    fn escapes_on_top_of_comrak() {
        assert_eq!(text("a $5 ^b"), "a [\\$]5 [\\^]b");
        assert_eq!(text("&#38; & b"), "[\\&]#38; & b");
        assert_eq!(
            text("a = b == c +++ d + e"),
            "a = b [\\=][\\=] c [\\+][\\+][\\+] d + e"
        );
        assert_eq!(
            text("see www.x.com or https://x"),
            "see www[\\.]x.com or https[\\:]//x"
        );
        assert_eq!(text("a|b"), "a|b");
        assert_eq!(
            written(
                &[Piece::Text("|a|")],
                RunCtx {
                    line_start: true,
                    ..RunCtx::default()
                }
            ),
            "[\\|]a|"
        );
        assert_eq!(
            written(
                &[Piece::Text("|a|")],
                RunCtx {
                    line_start: true,
                    cell: true,
                    ..RunCtx::default()
                }
            ),
            "|a|"
        );
        // Inside link text there are no autolinks.
        assert_eq!(
            written(
                &[Piece::Text("www.x.com http://x")],
                RunCtx {
                    in_link: true,
                    ..RunCtx::default()
                }
            ),
            "www.x.com http://x"
        );
    }

    #[test]
    fn delimiters_next_to_the_run() {
        let ctx = RunCtx {
            before: Some('='),
            after: Some('+'),
            ..RunCtx::default()
        };
        assert_eq!(written(&[Piece::Text("=a+")], ctx), "[\\=]a[\\+]");
        // comrak looks through any `=` or `+` next to a delimiter run.
        assert_eq!(written(&[Piece::Text("+a=")], ctx), "[\\+]a[\\=]");
        let star = RunCtx {
            before: Some('*'),
            after: Some('*'),
            ..RunCtx::default()
        };
        assert_eq!(written(&[Piece::Text("+ a =")], star), "[\\+] a [\\=]");
        // Not elsewhere.
        assert_eq!(written(&[Piece::Text("a + b = c")], star), "a + b = c");
    }

    #[test]
    fn alternating_equals_and_plus_are_escaped() {
        assert_eq!(text("=+=+"), "=[\\+]=[\\+]");
        assert_eq!(text("a += 1"), "a [\\+]= 1");
        assert_eq!(text("a + = b"), "a + = b");
    }

    #[test]
    fn a_byte_order_mark_starting_a_line_is_left_out() {
        let line = RunCtx {
            line_start: true,
            ..RunCtx::default()
        };
        assert_eq!(written(&[Piece::Text("\u{feff}a")], line), "a");
        assert_eq!(text("\u{feff}a"), "\u{feff}a");
        assert_eq!(written(&[Piece::Text("a\u{feff}b")], line), "a\u{feff}b");
    }

    #[test]
    fn id_shaped_tokens_never_become_links() {
        assert_eq!(
            text("See PROJ-123, MY_PROJ-12."),
            "See PROJ[\\-]123, MY_PROJ[\\-]12."
        );
        assert_eq!(text("UTF-8 A-1 ABC-1.5"), "UTF[\\-]8 A-1 ABC-1.5");
        // A linked id stays as it is; plain ids next to it are escaped.
        assert_eq!(
            written(
                &[Piece::Id("PROJ-1"), Piece::Text(" PROJ-2")],
                RunCtx::default()
            ),
            "[PROJ-1] PROJ[\\-]2"
        );
    }

    #[test]
    fn conservative_escapes_all_punctuation() {
        let ctx = RunCtx {
            conservative: true,
            ..RunCtx::default()
        };
        assert_eq!(written(&[Piece::Text("a.b-c")], ctx), "a[\\.]b[\\-]c");
        let cell = RunCtx {
            conservative: true,
            cell: true,
            ..RunCtx::default()
        };
        assert_eq!(written(&[Piece::Text("a|b")], cell), "a|b");
    }
}
