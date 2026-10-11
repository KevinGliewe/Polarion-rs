//! Text and formatting through the conversion to Markdown and back:
//! - plain text never turns into formatting, links or any other element, whatever characters it
//!   holds, and the escapes this crate adds on top of comrak's are exact;
//! - every printable ASCII character and pair, common non-ASCII text, and work item id shapes at
//!   every boundary (under each id setting) come back as text;
//! - emphasis comrak cannot delimit, superscript in link text and pipes in table cells keep their
//!   formatting;
//! - deep nesting converts in bounded time, in both directions;
//! - the raw HTML sanitizer applies on the way back: dangerous HTML is dropped or neutralized,
//!   clean raw HTML comes back byte for byte.

use std::time::Duration;

use polarion_markdown::{
    DiagramSources, RawHtml, ToHtmlOptions, ToMarkdownOptions, WorkItemIds, to_html, to_markdown,
    to_markdown_report,
};

fn keep() -> ToMarkdownOptions {
    ToMarkdownOptions::default()
}

fn drop_html() -> ToMarkdownOptions {
    ToMarkdownOptions {
        raw_html: RawHtml::Drop,
        ..ToMarkdownOptions::default()
    }
}

fn with_ids(work_items: WorkItemIds) -> ToMarkdownOptions {
    ToMarkdownOptions {
        work_items,
        ..ToMarkdownOptions::default()
    }
}

/// The options of the conversion back to HTML that go with `options` (no highlighting).
fn forward_options(options: &ToMarkdownOptions) -> ToHtmlOptions {
    ToHtmlOptions {
        work_items: options.work_items.clone(),
        highlight: false,
    }
}

/// Converting back with every id-shaped token linked: nothing in text may become a link.
fn any_ids() -> ToHtmlOptions {
    ToHtmlOptions {
        work_items: WorkItemIds::Any,
        highlight: false,
    }
}

/// The HTML of plain text: `& < >` as character references, line breaks as `<br/>`.
fn text_html(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            line.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        })
        .collect::<Vec<_>>()
        .join("<br/>")
}

/// ASCII whitespace: what browsers collapse (not U+00A0).
fn is_html_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

/// Text as a browser shows it: runs of ASCII whitespace as one space, every line trimmed.
fn collapse(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            line.split(is_html_space)
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What a browser shows of some HTML: its text ([`collapse`]d, `<br>` as a line break) and the
/// names of the other elements in it.
fn shown(html: &str) -> (String, Vec<String>) {
    let document = scraper::Html::parse_fragment(html);
    let mut text = String::new();
    let mut elements = Vec::new();
    for node in document.root_element().descendants().skip(1) {
        match node.value() {
            scraper::Node::Text(t) => {
                text.extend(t.chars().map(|c| if is_html_space(c) { ' ' } else { c }))
            }
            scraper::Node::Element(e) if e.name() == "br" => text.push('\n'),
            scraper::Node::Element(e) => elements.push(e.name().to_owned()),
            _ => {}
        }
    }
    (collapse(&text), elements)
}

/// The Markdown of `html`, checked: every built tree valid, no fallback, and a fixpoint.
fn checked(html: &str, options: &ToMarkdownOptions) -> String {
    let report = to_markdown_report(html, options, &DiagramSources::new());
    assert_eq!(
        report.invalid, 0,
        "invalid trees for {html:?}:\n{}",
        report.markdown
    );
    assert_eq!(
        report.fallbacks, 0,
        "fallbacks for {html:?}:\n{}",
        report.markdown
    );
    let back = to_html(&report.markdown, &forward_options(options));
    assert_eq!(
        to_markdown(&back, options),
        report.markdown,
        "not a fixpoint: {html:?}, HTML {back:?}"
    );
    report.markdown
}

/// The Markdown of `html`, checked: every built tree valid and a fixpoint (fallbacks allowed).
fn lenient(html: &str, options: &ToMarkdownOptions) -> String {
    let report = to_markdown_report(html, options, &DiagramSources::new());
    assert_eq!(
        report.invalid, 0,
        "invalid trees for {html:?}:\n{}",
        report.markdown
    );
    let back = to_html(&report.markdown, &forward_options(options));
    assert_eq!(
        to_markdown(&back, options),
        report.markdown,
        "not a fixpoint: {html:?}, HTML {back:?}"
    );
    report.markdown
}

/// The Markdown of plain text, checked as by [`checked`] (no fallback: the escapes alone keep
/// text text), and converted back, with the same setting and with every id linked, it shows
/// exactly the text and no element.
fn text_markdown(text: &str, options: &ToMarkdownOptions) -> String {
    let markdown = checked(&text_html(text), options);
    for forward in [forward_options(options), any_ids()] {
        let back = to_html(&markdown, &forward);
        let (text_shown, elements) = shown(&back);
        assert_eq!(
            text_shown,
            collapse(text),
            "{text:?} written {markdown:?} shows {back:?}"
        );
        assert!(
            elements.is_empty(),
            "{text:?} written {markdown:?} makes {elements:?}: {back:?}"
        );
    }
    markdown
}

/// [`text_markdown`] for many texts at once: each a paragraph of one document, 64 to a document.
/// When a document fails, each of its texts is checked alone first, for a clear message.
fn texts_survive(texts: &[String], options: &ToMarkdownOptions) {
    for chunk in texts.chunks(64) {
        let html = chunk
            .iter()
            .map(|text| text_html(text))
            .collect::<Vec<_>>()
            .join("<br/><br/>");
        let expected = chunk
            .iter()
            .map(|text| collapse(text))
            .collect::<Vec<_>>()
            .join("\n\n");
        let report = to_markdown_report(&html, options, &DiagramSources::new());
        let back = to_html(&report.markdown, &any_ids());
        let (text_shown, elements) = shown(&back);
        let again = to_markdown(
            &to_html(&report.markdown, &forward_options(options)),
            options,
        );
        if report.invalid != 0
            || report.fallbacks != 0
            || text_shown != expected
            || !elements.is_empty()
            || again != report.markdown
        {
            for text in chunk {
                text_markdown(text, options);
            }
            panic!(
                "these texts pass one by one but not together ({options:?}):\n{chunk:?}\n{}\n{back}",
                report.markdown
            );
        }
    }
}

mod plain_text {
    use super::*;

    /// What comrak's writer does not escape but would parse as syntax: `$`, `^`, runs of `=` and
    /// `+`, `&#`, a `|` starting a line, `www.`, `://`, id-shaped tokens. Single `=`, `+` and `|`
    /// inside a line need nothing.
    #[test]
    fn syntax_comrak_leaves_alone_is_escaped_exactly() {
        for (text, markdown) in [
            ("costs $5", "costs \\$5"),
            ("$x$", "\\$x\\$"),
            ("a $ b $$ c", "a \\$ b \\$\\$ c"),
            ("a^b", "a\\^b"),
            ("x^2^", "x\\^2\\^"),
            ("a == b", "a \\=\\= b"),
            ("a===b", "a\\=\\=\\=b"),
            ("==x==", "\\=\\=x\\=\\="),
            ("x ++ y", "x \\+\\+ y"),
            ("C++", "C\\+\\+"),
            ("++x++", "\\+\\+x\\+\\+"),
            ("&#65;", "\\&\\#65;"),
            ("&#x41; and &#", "\\&\\#x41; and \\&\\#"),
            ("| a | b |", "\\| a | b |"),
            ("|", "\\|"),
            ("www.example.com", "www\\.example.com"),
            ("see www.example.com.", "see www\\.example.com."),
            ("http://example.com", "http\\://example.com"),
            ("https://www.example.com/x", "https\\://www\\.example.com/x"),
            ("ftp://host/f", "ftp\\://host/f"),
            ("x://y", "x\\://y"),
            ("See PROJ-123", "See PROJ\\-123"),
            ("UTF-8", "UTF\\-8"),
            ("a = b", "a = b"),
            ("a + b", "a + b"),
            ("1 | 2", "1 | 2"),
            ("50% & more", "50% & more"),
            ("x: y", "x: y"),
        ] {
            assert_eq!(text_markdown(text, &keep()), markdown, "{text:?}");
            assert_eq!(text_markdown(text, &drop_html()), markdown, "{text:?}");
        }
    }

    /// Text full of Markdown syntax, at the start of a line and inside one, also on lines after a
    /// break, where a block could start (lists, headings, quotes, tables, setext underlines,
    /// fences, directives, link definitions).
    #[test]
    fn markdown_syntax_in_text_comes_back_as_text() {
        for text in [
            "a*b*c",
            "*not emphasis*",
            "**not bold**",
            "_x_",
            "__x__",
            "snake_case_name",
            "[x]",
            "[x](y)",
            "![x](y)",
            "[x]: /url",
            "[^1]",
            "#tag",
            "# not a heading",
            "a#b",
            "a<b>c",
            "<div>",
            "<!-- c -->",
            "a\\b",
            "\\",
            "`code`",
            "``",
            "~~x~~",
            "~x~",
            "~",
            "Hi!",
            "a@b.cd",
            "<https://x.y>",
            "- item",
            "-",
            "--",
            "---",
            "+ item",
            "+",
            "= x",
            "=",
            "===",
            "1. one",
            "1.",
            "1) one",
            "2024. Year",
            "10) x",
            ": def",
            ":",
            ":::note",
            "> quote",
            ">",
            "***",
            "___",
            "```",
            "~~~",
            "    four spaces",
            "&amp;",
            "&lt;tag&gt;",
            "AT&T",
            "&copy;",
            "{curly} (round) <angle>",
            "\"quotes\" 'single'",
            "a\u{a0}b",
            "$`x`$",
            "\\$",
            "> [!NOTE]",
            "☐ not a task",
            "Term\n: details",
            "a\n- b",
            "a\n# b",
            "a\n1. b",
            "a\n> b",
            "a\n===",
            "a\n---",
            "a\n| b |\n|---|",
            "a|b\n|-|-|",
            "x\n:::note",
            "x\n```",
            "x\n    y",
            "[a]\n[a]: /u",
            "line one\nline two",
        ] {
            text_markdown(text, &keep());
            text_markdown(text, &drop_html());
        }
    }

    /// `www.` and `://` are escaped outside link text only (no autolink is made inside one), the
    /// other escapes everywhere.
    #[test]
    fn link_text_keeps_urls_unescaped() {
        for markdown in [
            "[see https://x.y/a](https://x.y/a)",
            "[www.example.com](http://www.example.com)",
        ] {
            let html = to_html(markdown, &any_ids());
            assert_eq!(checked(&html, &keep()), markdown);
        }
        let html = r#"<a href="u" target="_top" class="descriptionLink">a$b</a>"#;
        let markdown = checked(html, &keep());
        assert_eq!(markdown, "[a\\$b](u)");
        assert_eq!(to_html(&markdown, &any_ids()), html);
    }
}

mod generated {
    use super::*;

    /// Printable ASCII characters but the space.
    fn printable() -> impl Iterator<Item = char> {
        '!'..='~'
    }

    #[test]
    fn every_printable_ascii_character_comes_back_as_text() {
        let mut texts = Vec::new();
        for c in printable() {
            for text in [
                format!("{c}"),
                format!("a{c}"),
                format!("{c}a"),
                format!("a{c}b"),
                format!("a {c} b"),
                format!("{c}{c}{c}"),
                format!("{c} {c}"),
                format!("x\n{c} y"),
                format!("x\n{c}{c}"),
            ] {
                texts.push(text);
            }
        }
        texts_survive(&texts, &keep());
    }

    /// 94² pairs, each alone, between letters and between spaces.
    #[test]
    fn every_pair_of_printable_ascii_characters_comes_back_as_text() {
        let mut texts = Vec::new();
        for a in printable() {
            for b in printable() {
                texts.push(format!("{a}{b}"));
                texts.push(format!("x{a}{b}y"));
                texts.push(format!("x {a}{b} y"));
            }
        }
        texts_survive(&texts, &keep());
    }

    #[test]
    fn common_non_ascii_text_comes_back_as_text() {
        let samples = [
            "ä",
            "Ö",
            "ß",
            "é",
            "è",
            "ñ",
            "ø",
            "€",
            "£",
            "¥",
            "©",
            "®",
            "™",
            "°",
            "±",
            "×",
            "÷",
            "§",
            "¶",
            "«",
            "»",
            "“",
            "”",
            "‘",
            "’",
            "„",
            "‚",
            "‹",
            "›",
            "–",
            "—",
            "…",
            "•",
            "→",
            "←",
            "⇒",
            "✓",
            "✗",
            "☐",
            "☑",
            "☒",
            "日本語",
            "中文",
            "한국어",
            "Ελληνικά",
            "Русский",
            "עברית",
            "العربية",
            "😀",
            "👍🏽",
            "e\u{301}",
            "Größe: PROJ-1 – fertig",
        ];
        let mut texts = Vec::new();
        for s in samples {
            for text in [
                s.to_owned(),
                format!("a{s}b"),
                format!("a {s} b"),
                format!("{s}a"),
            ] {
                texts.push(text);
            }
        }
        // Spaces browsers do not collapse, and invisible characters, between letters.
        for s in [
            "\u{a0}", "\u{2009}", "\u{200b}", "\u{3000}", "\u{feff}", "\u{2028}",
        ] {
            texts.push(format!("a{s}b"));
            texts.push(format!("a {s} b"));
        }
        texts_survive(&texts, &keep());
        texts_survive(&texts, &drop_html());
    }

    const ID_SHAPES: [&str; 13] = [
        "PROJ-1",
        "MY_PROJ-12",
        "UTF-8",
        "ISO-26262",
        "A1-007",
        "X_-2",
        "AB_C9-007",
        "A-1",
        "PROJ-1.5",
        "Proj-1",
        "PROJ-",
        "PROJ-12-34",
        "PROJ_1",
    ];

    /// What can stand before an id: boundaries and characters that are none.
    const LEFT: [&str; 30] = [
        "", " ", "(", "[", "{", "\"", "'", "“", "‘", "«", ",", ";", ":", "!", "?", "a", "x.", "-",
        "_", "/", "#", "$", "=", "é", "1", "„", "*", "`", "\u{a0}", "\\",
    ];

    /// What can stand after an id: boundaries and characters that are none.
    const RIGHT: [&str; 34] = [
        "", " ", ")", "]", "}", "\"", "'", "”", "’", "»", ",", ";", ":", "!", "?", ".", ". ", ".)",
        "..", "a", "A", "_", "-", "-2", ".5", ".a", "/", "#", "(", "*", "%", "é", "“", "\u{a0}",
    ];

    #[test]
    fn id_shaped_tokens_in_text_are_never_links() {
        let mut texts = Vec::new();
        for id in ID_SHAPES {
            for left in LEFT {
                for right in RIGHT {
                    texts.push(format!("{left}{id}{right}"));
                    texts.push(format!("x{left}{id}{right}y"));
                }
            }
        }
        texts_survive(&texts, &keep());
        // The setting does not change how text is written.
        let some: Vec<String> = texts.iter().step_by(7).cloned().collect();
        for work_items in [
            WorkItemIds::Prefixes(vec!["PROJ".into(), "UTF".into()]),
            WorkItemIds::Off,
        ] {
            texts_survive(&some, &with_ids(work_items));
        }
    }

    #[test]
    fn id_shapes_are_escaped_exactly_whatever_the_setting() {
        for work_items in [
            WorkItemIds::Any,
            WorkItemIds::Prefixes(vec!["PROJ".into()]),
            WorkItemIds::Off,
        ] {
            let options = with_ids(work_items.clone());
            for (text, markdown) in [
                ("PROJ-1", "PROJ\\-1"),
                ("MY_PROJ-12", "MY\\_PROJ\\-12"),
                ("PROJ-1.", "PROJ\\-1."),
                ("PROJ-1.5", "PROJ-1.5"),
                ("A-1", "A-1"),
                ("UTF-8", "UTF\\-8"),
                ("ISO-26262", "ISO\\-26262"),
                ("(PROJ-1)", "(PROJ\\-1)"),
                ("\"PROJ-1\"", "\"PROJ\\-1\""),
                ("'PROJ-1'", "'PROJ\\-1'"),
                ("“PROJ-1”", "“PROJ\\-1”"),
                ("«PROJ-1»", "«PROJ\\-1»"),
                ("PROJ-1,PROJ-2", "PROJ\\-1,PROJ\\-2"),
                ("See MP-42 and PROJ-7.", "See MP\\-42 and PROJ\\-7."),
                ("aPROJ-1", "aPROJ-1"),
                ("PROJ-1a", "PROJ-1a"),
                ("PROJ-12-34", "PROJ-12-34"),
                ("x.PROJ-1", "x.PROJ-1"),
                ("\\PROJ-1", "\\\\PROJ-1"),
            ] {
                assert_eq!(
                    text_markdown(text, &options),
                    markdown,
                    "{text:?} ({work_items:?})"
                );
            }
        }
    }
}

mod formatting {
    use super::*;

    /// The formatting a browser shows for each run of text: bold, italic, strike, underline,
    /// highlight, superscript, subscript, monospace and link, from tags and inline styles.
    /// Neighboring runs with the same formatting are one.
    fn runs(html: &str) -> Vec<(String, Vec<&'static str>)> {
        fn add(styles: &mut Vec<&'static str>, style: &'static str) {
            if !styles.contains(&style) {
                styles.push(style);
            }
        }

        fn apply(element: &scraper::node::Element, styles: &mut Vec<&'static str>) {
            match element.name() {
                "b" | "strong" => add(styles, "bold"),
                "i" | "em" => add(styles, "italic"),
                "s" | "del" | "strike" => add(styles, "strike"),
                "u" | "ins" => add(styles, "underline"),
                "mark" => add(styles, "highlight"),
                "sup" => add(styles, "sup"),
                "sub" => add(styles, "sub"),
                "code" => add(styles, "mono"),
                "a" if element.attr("href").is_some() => add(styles, "link"),
                _ => {}
            }
            let style = element
                .attr("style")
                .unwrap_or_default()
                .to_ascii_lowercase();
            for declaration in style.split(';') {
                let Some((property, value)) = declaration.split_once(':') else {
                    continue;
                };
                let value = value.trim();
                match property.trim() {
                    "font-weight" if value == "bold" => add(styles, "bold"),
                    "font-weight" if value == "normal" => styles.retain(|s| *s != "bold"),
                    "font-style" if value == "italic" => add(styles, "italic"),
                    "text-decoration" => {
                        if value.contains("line-through") {
                            add(styles, "strike");
                        }
                        if value.contains("underline") {
                            add(styles, "underline");
                        }
                    }
                    "background-color"
                        if ["#ffff00", "#ff0", "yellow", "rgb(255, 255, 0)"].contains(&value) =>
                    {
                        add(styles, "highlight")
                    }
                    "font-family" if value.contains("courier") || value.contains("monospace") => {
                        add(styles, "mono")
                    }
                    _ => {}
                }
            }
            styles.sort_unstable();
        }

        fn walk(
            element: scraper::ElementRef<'_>,
            styles: &[&'static str],
            out: &mut Vec<(String, Vec<&'static str>)>,
        ) {
            for child in element.children() {
                if let Some(text) = child.value().as_text() {
                    match out.last_mut() {
                        Some((last, last_styles)) if last_styles.as_slice() == styles => {
                            last.push_str(text)
                        }
                        _ => out.push((String::from(&**text), styles.to_vec())),
                    }
                } else if let Some(child) = scraper::ElementRef::wrap(child) {
                    let mut inner = styles.to_vec();
                    apply(child.value(), &mut inner);
                    walk(child, &inner, out);
                }
            }
        }

        let document = scraper::Html::parse_fragment(html);
        let mut out = Vec::new();
        walk(document.root_element(), &[], &mut out);
        out
    }

    /// Formatting written as Markdown would not parse back: delimiters next to punctuation and
    /// letters (left- and right-flanking rules), runs of delimiters of two kinds side by side.
    const FLANKING: [&str; 15] = [
        r#"<span style="font-weight: bold;">foo.</span>bar"#,
        r#"<span style="text-decoration: line-through;">a<sub>b</sub></span>"#,
        r#"<span style="font-style: italic;">a.</span>b"#,
        r#"a<span style="font-weight: bold;">.b</span>"#,
        r#"x<span style="font-weight: bold;">(a)</span>"#,
        r#"<span style="font-weight: bold;">"q"</span>x"#,
        r#"<span style="background-color: #FFFF00;">a.</span>b"#,
        r#"<span style="text-decoration: underline;">a.</span>b"#,
        r#"<span style="font-weight: bold;">a</span><span style="font-style: italic;">b</span>"#,
        r#"<span style="font-style: italic;">a</span><span style="font-weight: bold;">b</span>"#,
        r#"<span style="text-decoration: line-through;">a</span><sub>b</sub>"#,
        r#"<sup>a</sup>b<sup>c</sup>"#,
        // `~~a\~b~~\~`: escaped tildes join no delimiter run.
        r#"<span style="text-decoration: line-through;">a~b</span>~"#,
        // comrak looks through `+` and `=` next to a delimiter run.
        r#"<span style="font-weight: bold;">+ a</span>"#,
        r#"<span style="font-style: italic;">a =</span>"#,
    ];

    #[test]
    fn emphasis_comrak_cannot_delimit_keeps_its_formatting() {
        for html in FLANKING {
            for options in [keep(), drop_html()] {
                let markdown = lenient(html, &options);
                let back = to_html(&markdown, &forward_options(&options));
                assert_eq!(
                    runs(&back),
                    runs(html),
                    "{html} ({options:?}) written {markdown:?}: {back}"
                );
            }
        }
    }

    #[test]
    fn superscript_and_subscript_in_link_text_keep_their_formatting() {
        for markdown in [
            "[x<sup>2</sup>](u)",
            "[x~2~](u)",
            "[a ~b~ <sup>c</sup>](u \"T\")",
        ] {
            let html = to_html(markdown, &any_ids());
            let back = to_html(&lenient(&html, &keep()), &any_ids());
            assert_eq!(back, html, "{markdown}");
            let back = to_html(&lenient(&html, &drop_html()), &any_ids());
            assert_eq!(runs(&back), runs(&html), "{markdown} in drop mode: {back}");
        }
        // A link inside a superscript.
        let html = to_html("^[x](u)^", &any_ids());
        for options in [keep(), drop_html()] {
            let back = to_html(&lenient(&html, &options), &any_ids());
            assert_eq!(runs(&back), runs(&html), "{options:?}: {back}");
        }
    }

    /// A cell holding pipes and a line break whose next line looks like a delimiter row.
    #[test]
    fn a_cell_with_pipes_and_a_break_stays_one_cell() {
        let markdown = "| h |\n| --- |\n| a\\|b<br/>\\|-\\|-\\| |";
        let html = to_html(markdown, &any_ids());
        let back = checked(&html, &keep());
        assert_eq!(back, markdown);
        assert_eq!(to_html(&back, &any_ids()), html);
    }
}

mod nesting {
    use super::*;

    const DEPTH: usize = 10_000;
    /// Generous for a debug build on a busy machine; quadratic work at this depth takes minutes.
    const LIMIT: Duration = Duration::from_secs(20);

    /// Runs `f` on a thread of its own and waits at most [`LIMIT`] for it: a conversion that
    /// hangs fails the test instead of blocking the run.
    fn within<T: Send + 'static>(what: String, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = sender.send(f());
        });
        match receiver.recv_timeout(LIMIT) {
            Ok(value) => value,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("{what}: no result within {LIMIT:?}")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => match worker.join() {
                Err(panic) => std::panic::resume_unwind(panic),
                Ok(()) => panic!("{what}: no result"),
            },
        }
    }

    #[test]
    fn deeply_nested_html_converts_in_bounded_time() {
        for (open, close) in [
            ("<div>", "</div>"),
            ("<span>", "</span>"),
            ("<ul><li>", "</li></ul>"),
            ("<blockquote>", "</blockquote>"),
            ("<div style=\"margin-left: 40px;\">", "</div>"),
            ("<span style=\"font-weight: bold;\">", "</span>"),
            ("<span style=\"color: red;\">", "</span>"),
        ] {
            let html = format!("{}deep text{}", open.repeat(DEPTH), close.repeat(DEPTH));
            for options in [keep(), drop_html()] {
                let html = html.clone();
                let what = format!("{DEPTH} times {open} ({options:?})");
                let (markdown, back) = within(what.clone(), move || {
                    let markdown = lenient(&html, &options);
                    let back = to_html(&markdown, &forward_options(&options));
                    (markdown, back)
                });
                assert!(markdown.contains("deep text"), "{what}: {markdown}");
                assert!(back.contains("deep text"), "{what}: {back}");
            }
        }
    }

    /// Nesting four times deeper than [`DEPTH`]: work quadratic in the depth (the HTML parser's
    /// own included) would take far longer than [`LIMIT`].
    #[test]
    fn much_deeper_nesting_takes_linear_time() {
        let depth = 4 * DEPTH;
        for (open, close) in [
            ("<ul><li>", ""),
            ("<div>", "</div>"),
            ("<div style=\"margin-left: 40px;\">", ""),
            (
                "<table><tr><td colspan=\"2\"><div>",
                "</div></td></tr></table>",
            ),
        ] {
            let html = format!("{}deep text{}", open.repeat(depth), close.repeat(depth));
            for options in [keep(), drop_html()] {
                let html = html.clone();
                let what = format!("{depth} times {open} ({options:?})");
                let markdown = within(what.clone(), move || lenient(&html, &options));
                assert!(markdown.contains("deep text"), "{what}: {markdown}");
            }
        }
    }

    /// Alternating `=` and `+` would make comrak's delimiter processing quadratic in the
    /// self-check: they are escaped.
    #[test]
    fn long_runs_of_alternating_equals_and_plus_take_linear_time() {
        for text in ["=+".repeat(50_000), "+=".repeat(50_000)] {
            let what = format!("{}…", &text[..8]);
            let markdown = within(what.clone(), move || lenient(&text, &keep()));
            assert!(markdown.contains("=\\+="), "{what}: {}", &markdown[..20]);
        }
    }

    #[test]
    fn deeply_nested_markdown_converts_in_bounded_time() {
        let lists: String = (0..2_000)
            .map(|i| format!("{}- x\n", "  ".repeat(i)))
            .collect();
        for markdown in [
            ">".repeat(DEPTH) + " deep text PROJ-1",
            "> - ".repeat(2_000) + "deep text",
            lists + "\ndeep text",
            "*".repeat(5_000) + "deep text" + &"*".repeat(5_000),
            "[".repeat(5_000) + "deep text" + &"](u)".repeat(5_000),
        ] {
            let what = format!("{}…", &markdown[..20]);
            let back = within(what.clone(), move || {
                let html = to_html(&markdown, &ToHtmlOptions::default());
                let back = lenient(&html, &keep());
                let _ = to_html(&back, &ToHtmlOptions::default());
                back
            });
            assert!(back.contains("deep text"), "{what}: {back}");
        }
    }
}

mod sanitizer {
    use super::*;

    /// What a browser would run or load from some HTML: the elements the sanitizer filters, event
    /// handlers, `srcdoc`, script URLs, script in styles.
    fn dangers(html: &str) -> Vec<String> {
        let squeeze = |value: &str| -> String {
            value
                .chars()
                .filter(|c| !c.is_whitespace() && !c.is_control())
                .collect::<String>()
                .to_ascii_lowercase()
        };
        let mut found = Vec::new();
        for node in scraper::Html::parse_fragment(html).tree.nodes() {
            let Some(element) = node.value().as_element() else {
                continue;
            };
            let name = element.name();
            if [
                "script",
                "style",
                "iframe",
                "textarea",
                "title",
                "xmp",
                "noembed",
                "noframes",
                "noscript",
                "plaintext",
                "object",
                "embed",
                "frame",
                "frameset",
                "base",
                "link",
                "meta",
            ]
            .contains(&name)
            {
                found.push(format!("<{name}>"));
            }
            for (attr, value) in element.attrs() {
                let attr = attr.to_ascii_lowercase();
                let value = squeeze(value);
                let script = value.starts_with("javascript:") || value.starts_with("vbscript:");
                if attr.starts_with("on")
                    || attr == "srcdoc"
                    || (["href", "src", "xlink:href", "action", "formaction", "data"]
                        .contains(&attr.as_str())
                        && (script || value.starts_with("data:text")))
                    || (attr == "style"
                        && (value.contains("javascript:") || value.contains("expression(")))
                {
                    found.push(format!("<{name} {attr}={value:?}>"));
                }
            }
        }
        found
    }

    #[test]
    fn the_browser_check_finds_what_it_should() {
        assert_eq!(dangers("<a href=\"javascript:x\">x</a>").len(), 1);
        assert_eq!(dangers("<img src=x onerror=y>").len(), 1);
        assert_eq!(dangers("<script>x</script>").len(), 1);
        assert_eq!(dangers("<p style=\"x: expression(1)\">").len(), 1);
        assert!(dangers("<a href=\"https://ok\" title=\"javascript:x\">x</a>").is_empty());
    }

    #[test]
    fn dangerous_html_is_dropped_or_neutralized() {
        for (html, text) in [
            (r#"<img src="x.png" onerror="alert(1)">"#, ""),
            ("<img src=x onerror=alert(1)>", ""),
            (r#"<a href="javascript:alert(1)">x</a>"#, "x"),
            (r#"<a href="JaVaScRiPt:alert(1)">x</a>"#, "x"),
            (r#"<a href=" javascript:alert(1)">x</a>"#, "x"),
            (r#"<a href="&#106;avascript:alert(1)">x</a>"#, "x"),
            (r#"<a href="java&#x09;script:alert(1)">x</a>"#, "x"),
            (r#"<a href="vbscript:x">x</a>"#, "x"),
            (r#"<a href="data:text/html;base64,PHNjcmlwdD4=">x</a>"#, "x"),
            (r#"<img src="javascript:alert(1)">"#, ""),
            (r#"<span onclick="alert(1)">x</span>"#, "x"),
            (
                r#"<span style="color: red;" onmouseover="alert(1)">x</span>"#,
                "x",
            ),
            (
                r#"<div onmouseover="alert(1)" style="text-align: center;">x</div>"#,
                "x",
            ),
            (
                r#"<span style="background: url(javascript:alert(1))">x</span>"#,
                "x",
            ),
            (r#"<p style="width: expression(alert(1))">x</p>"#, "x"),
            ("a<script>alert(1)</script>b", "ab"),
            ("<style>p { color: red; }</style>x", "x"),
            (r#"<iframe src="https://example.com/evil"></iframe>x"#, "x"),
            ("<svg onload=alert(1)><text>x</text></svg>", ""),
            (r#"<object data="x.swf"></object>x"#, "x"),
            (r#"<embed src="x.swf">x"#, "x"),
            (
                r#"<form action="javascript:x"><button formaction="javascript:y">b</button></form>"#,
                "",
            ),
            (
                r#"<table><tr><td onclick="alert(1)">x</td></tr></table>"#,
                "x",
            ),
            (r#"<pre onmouseover="alert(1)">x</pre>"#, "x"),
        ] {
            for options in [keep(), drop_html()] {
                let markdown = lenient(html, &options);
                let lower = markdown.to_ascii_lowercase();
                for bad in [
                    "onerror",
                    "onclick",
                    "onload",
                    "onmouseover",
                    "javascript:",
                    "vbscript:",
                    "data:text",
                    "<script",
                    "<style",
                    "<iframe",
                    "<object",
                    "<embed",
                    "expression(",
                    "alert(1)",
                ] {
                    assert!(
                        !lower.contains(bad),
                        "{html} ({options:?}): {bad} in {markdown:?}"
                    );
                }
                let back = to_html(&markdown, &forward_options(&options));
                assert!(
                    dangers(&back).is_empty(),
                    "{html} ({options:?}): {:?} in {back}",
                    dangers(&back)
                );
                let (text_shown, _) = shown(&back);
                assert!(
                    text_shown.contains(text),
                    "{html} ({options:?}): {text:?} lost: {back}"
                );
            }
        }
    }

    /// Raw HTML kept in keep mode is written so that the forward direction passes it on as it
    /// was: converting to Markdown and back gives the same bytes.
    #[test]
    fn clean_raw_html_comes_back_byte_for_byte() {
        let colspan = concat!(
            r#"<table id="polarion_wiki macro name=table" class="polarion-Document-table" style="width: 100%;">"#,
            r#"<tbody><tr><th style="font-weight: bold;">a</th><th style="font-weight: bold;">b</th></tr>"#,
            r#"<tr><td colspan="2">x</td></tr></tbody></table>"#
        );
        for html in [
            r#"a <span style="color: #FF0000;">red</span> b"#,
            r#"<span style="background-color: #FF0000;">marked</span>"#,
            r#"<span style="font-family: Georgia, serif;">serif</span>"#,
            r#"Font Size <span style="font-size: 12pt;">12</span>"#,
            r#"<span style="font-size: 11pt;">Body text</span>"#,
            r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-option-id="short"></span>"#,
            r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-option-id="custom" data-custom-label="the spec"></span>"#,
            r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-scope="OTHER" data-item-id="PROJ-1" data-option-id="long"></span>"#,
            r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-revision="1234" data-option-id="long"></span>"#,
            r#"<span data-source="a&#10;b" class="polarion-rte-formula"></span>"#,
            r#"<img src="workitemimg:2-c.jpg" style="width: 100px;height: 74px;"/>"#,
            r#"<div style="text-align: center;">Align Center</div>"#,
            r#"<p class="polarion-rte-caption-paragraph">Figure <span data-sequence="Figure" class="polarion-rte-caption">#</span> caption text</p>"#,
            colspan,
        ] {
            let markdown = checked(html, &keep());
            assert_eq!(to_html(&markdown, &any_ids()), html, "written {markdown:?}");
        }
    }
}
