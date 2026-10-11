//! Syntax highlighting of fenced code (feature `highlight`): syntect's default syntaxes with the
//! `InspiredGitHub` theme, as inline-styled spans. Without the feature code stays plain.
//!
//! What highlighting costs depends on the grammar more than on the code: about a millisecond per
//! kilobyte for most, tens for some (PHP, Lisp, Makefile), up to a few hundred milliseconds for a
//! single line of a few hundred bytes, and up to a second the first time a grammar is used (its
//! patterns are compiled then). So each document has a [`Budget`] of 64 KiB of code and one
//! second of highlighting, checked before each line; code that does not fit, and code with a
//! line longer than 512 bytes, stays plain. Only documents that run into the time limit can
//! come out differently on a faster or slower machine.

use std::time::Duration;

/// What one document may still spend on highlighting: a code block that would exceed the bytes
/// stays plain, and once the time is used up the rest of the document does.
#[derive(Debug, Clone)]
// Without the feature nothing is highlighted, so nothing spends it.
#[cfg_attr(not(feature = "highlight"), allow(dead_code))]
pub(crate) struct Budget {
    pub(crate) bytes: usize,
    pub(crate) time: Duration,
}

impl Budget {
    /// Bytes of code highlighted per document at most.
    pub(crate) const BYTES: usize = 64 * 1024;
    /// Time spent highlighting per document at most.
    pub(crate) const TIME: Duration = Duration::from_secs(1);

    /// The budget of a document.
    pub(crate) fn per_document() -> Budget {
        Budget {
            bytes: Self::BYTES,
            time: Self::TIME,
        }
    }
}

/// The content of a `<pre>` for `code` in language `lang`: syntect's spans, or `None` when the
/// language is unknown or plain text, the code is empty, has a line longer than 512 bytes or
/// does not fit the `budget`, or highlighting fails (the caller writes escaped text). Spends from
/// the budget what highlighting took.
#[cfg(feature = "highlight")]
pub(crate) fn highlight(lang: &str, code: &str, budget: &mut Budget) -> Option<String> {
    imp::highlight(lang, code, budget)
}

/// Without the `highlight` feature code is never highlighted.
#[cfg(not(feature = "highlight"))]
pub(crate) fn highlight(lang: &str, code: &str, budget: &mut Budget) -> Option<String> {
    let _ = (lang, code, budget);
    None
}

#[cfg(feature = "highlight")]
mod imp {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::LazyLock;
    use std::time::{Duration, Instant};

    use syntect::easy::HighlightLines;
    use syntect::highlighting::{Theme, ThemeSet};
    use syntect::html::{IncludeBackground, append_highlighted_html_for_styled_line};
    use syntect::parsing::{SyntaxReference, SyntaxSet};
    use syntect::util::LinesWithEndings;

    use super::Budget;

    /// Code with a longer line stays plain: one line is highlighted at once, and a line this long
    /// can already take more than 100 ms.
    pub(super) const MAX_LINE: usize = 512;

    static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
    static THEME: LazyLock<Theme> = LazyLock::new(|| {
        let mut themes = ThemeSet::load_defaults();
        themes.themes.remove("InspiredGitHub").unwrap_or_default()
    });

    /// Common fence language names syntect does not resolve by extension or grammar name.
    const ALIASES: [(&str, &str); 11] = [
        ("c#", "cs"),
        ("csharp", "cs"),
        ("c++", "cpp"),
        ("cplusplus", "cpp"),
        ("shell", "sh"),
        ("bash", "sh"),
        ("zsh", "sh"),
        ("console", "sh"),
        ("yml", "yaml"),
        ("node", "js"),
        ("golang", "go"),
    ];

    fn syntax(lang: &str) -> Option<&'static SyntaxReference> {
        let lang = lang.to_ascii_lowercase();
        let token = ALIASES
            .iter()
            .find(|(alias, _)| *alias == lang)
            .map_or(lang.as_str(), |(_, to)| to);
        SYNTAXES
            .find_syntax_by_token(token)
            .filter(|s| s.name != "Plain Text")
    }

    pub(super) fn highlight(lang: &str, code: &str, budget: &mut Budget) -> Option<String> {
        if code.is_empty()
            || code.len() > budget.bytes
            || budget.time.is_zero()
            || code.split('\n').any(|line| line.len() > MAX_LINE)
        {
            return None;
        }
        let syntax = syntax(lang)?;
        let started = Instant::now();
        let deadline = started + budget.time;
        // A bug in syntect or a grammar must not take the conversion down: the code stays plain.
        let spans = catch_unwind(AssertUnwindSafe(|| spans(syntax, code, deadline)))
            .ok()
            .flatten();
        budget.time = budget.time.saturating_sub(started.elapsed());
        if Instant::now() >= deadline {
            budget.time = Duration::ZERO;
        }
        if spans.is_some() {
            budget.bytes -= code.len();
        }
        spans
    }

    /// The spans, or `None` once `deadline` has passed (checked before each line).
    fn spans(syntax: &SyntaxReference, code: &str, deadline: Instant) -> Option<String> {
        let mut lines = HighlightLines::new(syntax, &THEME);
        let mut out = String::with_capacity(code.len() * 4);
        for line in LinesWithEndings::from(code) {
            if Instant::now() >= deadline {
                return None;
            }
            if line.ends_with('\n') {
                let regions = lines.highlight_line(line, &SYNTAXES).ok()?;
                append_highlighted_html_for_styled_line(&regions, IncludeBackground::No, &mut out)
                    .ok()?;
            } else {
                // The grammars expect line ends: highlight the last line with one, then drop it.
                let terminated = format!("{line}\n");
                let mut regions = lines.highlight_line(&terminated, &SYNTAXES).ok()?;
                if let Some((_, text)) = regions.last_mut() {
                    *text = text.strip_suffix('\n').unwrap_or(text);
                }
                regions.retain(|(_, text)| !text.is_empty());
                append_highlighted_html_for_styled_line(&regions, IncludeBackground::No, &mut out)
                    .ok()?;
            }
        }
        Some(out)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// With a document's whole budget.
        fn highlight(lang: &str, code: &str) -> Option<String> {
            super::highlight(lang, code, &mut Budget::per_document())
        }

        #[test]
        fn known_languages_and_aliases() {
            for lang in [
                "rust", "rs", "csharp", "C#", "cs", "c++", "cpp", "bash", "shell", "yml", "yaml",
                "js", "py", "json", "golang", "Python",
            ] {
                assert!(syntax(lang).is_some(), "{lang}");
            }
            for lang in ["no-such-language", "text", "txt", ""] {
                assert!(syntax(lang).is_none(), "{lang}");
            }
        }

        #[test]
        fn spans_keep_the_text() {
            let code = "fn main() {\n    let x = \"<&>\";\n}";
            let html = highlight("rust", code).unwrap();
            assert!(html.starts_with("<span style=\""), "{html}");
            assert!(html.contains("color:#"), "{html}");
            assert!(!html.contains("background-color"), "{html}");
            assert!(html.contains("&lt;&amp;&gt;"), "{html}");
            // Stripping the tags gives the code back (syntect also escapes quotes).
            let mut text = String::new();
            let mut in_tag = false;
            for c in html.chars() {
                match c {
                    '<' => in_tag = true,
                    '>' => in_tag = false,
                    c if !in_tag => text.push(c),
                    _ => {}
                }
            }
            let text = text
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&amp;", "&");
            assert_eq!(text, code);
        }

        #[test]
        fn no_trailing_newline_is_added() {
            let html = highlight("rust", "let x = 1;").unwrap();
            assert!(!html.contains('\n'), "{html}");
            assert_eq!(highlight("rust", "a\n").unwrap().matches('\n').count(), 1);
            assert!(highlight("rust", "").is_none());
            assert!(highlight("unknown", "x").is_none());
        }

        #[test]
        fn code_with_a_long_line_stays_plain() {
            assert_eq!(MAX_LINE, 512);
            let line = "x".repeat(MAX_LINE);
            assert!(highlight("rust", &format!("let a = 1;\n{line}")).is_some());
            assert!(highlight("rust", &format!("let a = 1;\n{line}x\nlet b = 2;")).is_none());
        }

        #[test]
        fn highlighting_spends_the_bytes_of_the_budget() {
            let mut budget = Budget {
                bytes: 21,
                time: Duration::from_secs(60),
            };
            assert!(super::highlight("rust", "let x = 1;", &mut budget).is_some());
            assert_eq!(budget.bytes, 11);
            // Unknown languages, empty code and code that stays plain spend nothing.
            assert!(super::highlight("unknown", "let x = 1;", &mut budget).is_none());
            assert!(super::highlight("rust", "", &mut budget).is_none());
            assert_eq!(budget.bytes, 11);
            // Code that does not fit stays plain; smaller code still fits.
            assert!(super::highlight("rust", "let yy = 22;", &mut budget).is_none());
            assert_eq!(budget.bytes, 11);
            assert!(super::highlight("rust", "let y = 2;", &mut budget).is_some());
            assert_eq!(budget.bytes, 1);
            assert!(budget.time > Duration::ZERO && budget.time < Duration::from_secs(60));
        }

        #[test]
        fn highlighting_stops_when_the_time_is_up() {
            let code = "let x = 1;\n".repeat(100);
            let mut budget = Budget {
                bytes: Budget::BYTES,
                time: Duration::from_micros(1),
            };
            // The time is checked before each line: at the second one it is used up.
            assert!(super::highlight("rust", &code, &mut budget).is_none());
            assert_eq!(budget.time, Duration::ZERO);
            assert_eq!(budget.bytes, Budget::BYTES);
            // Then nothing more is highlighted.
            assert!(super::highlight("rust", "let x = 1;", &mut budget).is_none());
        }

        /// The time limit bounds grammars that are slow on lines within the length limit:
        /// without it this block takes several seconds.
        #[test]
        fn slow_code_stays_within_the_time_limit() {
            let code = format!("{}\n", "a".repeat(MAX_LINE)).repeat(100);
            let mut budget = Budget::per_document();
            let started = Instant::now();
            let spans = super::highlight("cs", &code, &mut budget);
            let took = started.elapsed();
            assert!(took < Budget::TIME * 3, "{took:?}");
            if spans.is_none() {
                assert_eq!(budget.time, Duration::ZERO);
            }
        }
    }
}
