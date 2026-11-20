//! Exact output of the conversion to Polarion HTML: every mapping, separator and id rule, the raw
//! HTML sanitizer (also checked against a browser's parser), inputs that must not break it or
//! take long, whole documents, and the helpers.

use polarion_markdown::{
    DiagramAttachments, ToHtmlOptions, WorkItemIds, append_html, attachment_refs, diagram_refs,
    mermaid, text_to_html, to_html, to_html_with_diagrams,
};

fn html(markdown: &str) -> String {
    to_html(markdown, &ToHtmlOptions::default())
}

/// Without syntax highlighting.
fn plain(markdown: &str) -> String {
    to_html(
        markdown,
        &ToHtmlOptions {
            highlight: false,
            ..ToHtmlOptions::default()
        },
    )
}

fn with_ids(markdown: &str, work_items: WorkItemIds) -> String {
    to_html(
        markdown,
        &ToHtmlOptions {
            work_items,
            ..ToHtmlOptions::default()
        },
    )
}

const BOLD: &str = r#"<span style="font-weight: bold;">"#;
const ITALIC: &str = r#"<span style="font-style: italic;">"#;
const STRIKE: &str = r#"<span style="text-decoration: line-through;">"#;
const UNDERLINE: &str = r#"<span style="text-decoration: underline;">"#;
const HIGHLIGHT: &str = r#"<span style="background-color: #FFFF00;">"#;
const MONO: &str = r#"<span style="font-family: 'Courier New', Courier, monospace;">"#;
const END: &str = "</span>";
const PRE: &str =
    r#"<pre style="margin-left: 40px; font-family: 'Courier New', Courier, monospace;">"#;
const QUOTE: &str = r#"<div style="margin-left: 40px;">"#;
const TABLE: &str = concat!(
    r#"<table id="polarion_wiki macro name=table" class="polarion-Document-table" "#,
    r#"style="width: 100%; max-width: 1280px; margin-left: 0px; margin-right: auto; "#,
    r#"border: 1px solid #CCCCCC; empty-cells: show; border-collapse: collapse;"><tbody>"#
);
const TABLE_END: &str = "</tbody></table>";

fn heading(size: &str, line_height: &str) -> String {
    format!(r#"<span style="font-weight: bold; font-size: {size}pt; line-height: {line_height};">"#)
}

fn th(align: &str) -> String {
    format!(
        r#"<th style="font-weight: bold; background-color: #F0F0F0; text-align: {align}; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;">"#
    )
}

fn td(align: &str) -> String {
    format!(
        r#"<td style="text-align: {align}; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;">"#
    )
}

fn alert_open(color: &str, title: &str) -> String {
    format!(
        r#"<div style="margin-left: 40px; border-left: 3px solid {color}; padding-left: 10px;"><span style="font-weight: bold; color: {color};">{title}</span><br/>"#
    )
}

fn wi(id: &str) -> String {
    format!(
        r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="{id}" data-option-id="long"></span>"#
    )
}

fn formula(source: &str, display: bool) -> String {
    let inline = if display {
        r#" data-inline="false""#
    } else {
        ""
    };
    format!(r#"<span data-source="{source}"{inline} class="polarion-rte-formula"></span>"#)
}

fn link(href: &str, label: &str) -> String {
    format!(r#"<a href="{href}" target="_top" class="descriptionLink">{label}</a>"#)
}

mod mapping {
    use super::*;

    #[test]
    fn headings() {
        let sizes = [
            ("22", "1.2"),
            ("18", "1.2"),
            ("14", "1.2"),
            ("12", "1.5"),
            ("11", "1.5"),
            ("10.5", "1.5"),
        ];
        for (level, (size, line_height)) in (1..=6).zip(sizes) {
            let markdown = format!("{} Title", "#".repeat(level));
            assert_eq!(
                html(&markdown),
                format!("{}Title{END}", heading(size, line_height)),
                "{markdown}"
            );
        }
        assert_eq!(
            html("Title\n====="),
            format!("{}Title{END}", heading("22", "1.2"))
        );
        assert_eq!(
            html("Title\n-----"),
            format!("{}Title{END}", heading("18", "1.2"))
        );
        // Inline formatting inside; line breaks of a setext heading become spaces.
        assert_eq!(
            html("# A *b* `c`"),
            format!("{}A {ITALIC}b{END} {MONO}c{END}{END}", heading("22", "1.2"))
        );
        assert_eq!(
            html("one\ntwo  \nthree\n==="),
            format!("{}one two three{END}", heading("22", "1.2"))
        );
        assert_eq!(html("#"), format!("{}{END}", heading("22", "1.2")));
        assert_eq!(html("####### seven"), "####### seven");
    }

    #[test]
    fn emphasis() {
        assert_eq!(html("**b**"), format!("{BOLD}b{END}"));
        assert_eq!(html("__b__"), format!("{BOLD}b{END}"));
        assert_eq!(html("*i*"), format!("{ITALIC}i{END}"));
        assert_eq!(html("_i_"), format!("{ITALIC}i{END}"));
        assert_eq!(html("~~s~~"), format!("{STRIKE}s{END}"));
        assert_eq!(html("++u++"), format!("{UNDERLINE}u{END}"));
        assert_eq!(html("==h=="), format!("{HIGHLIGHT}h{END}"));
        assert_eq!(html("x^2^"), "x<sup>2</sup>");
        assert_eq!(html("H~2~O"), "H<sub>2</sub>O");
        assert_eq!(html("***x***"), format!("{ITALIC}{BOLD}x{END}{END}"));
        assert_eq!(
            html("**a *b* c**"),
            format!("{BOLD}a {ITALIC}b{END} c{END}")
        );
        assert_eq!(html("~~a **b**~~"), format!("{STRIKE}a {BOLD}b{END}{END}"));
    }

    #[test]
    fn code_spans() {
        assert_eq!(html("`c`"), format!("{MONO}c{END}"));
        assert_eq!(
            html("`<a href=\"x\">&amp;`"),
            format!("{MONO}&lt;a href=\"x\"&gt;&amp;amp;{END}")
        );
        assert_eq!(html("`` a`b ``"), format!("{MONO}a`b{END}"));
        assert_eq!(html("`**not bold**`"), format!("{MONO}**not bold**{END}"));
        // Line breaks inside are spaces, whatever the line ending.
        for markdown in ["`a\nb`", "`a\r\nb`", "`a\rb`"] {
            assert_eq!(html(markdown), format!("{MONO}a b{END}"), "{markdown:?}");
        }
    }

    #[test]
    fn text_and_breaks() {
        assert_eq!(
            html("a & b < c > d \"e\" 'f'"),
            "a &amp; b &lt; c &gt; d \"e\" 'f'"
        );
        assert_eq!(html("a\u{a0}b"), "a&nbsp;b");
        assert_eq!(html("&copy; &#169; &amp;"), "© © &amp;");
        assert_eq!(html("\\*not emphasis\\*"), "*not emphasis*");
        assert_eq!(html("soft\nbreak"), "soft<br/>break");
        assert_eq!(html("hard  \nbreak"), "hard<br/>break");
        assert_eq!(html("hard\\\nbreak"), "hard<br/>break");
        assert_eq!(html("trailing  "), "trailing");
    }

    #[test]
    fn links() {
        assert_eq!(
            html("[label](http://example.com)"),
            link("http://example.com", "label")
        );
        assert_eq!(
            html("[label](http://example.com \"The title\")"),
            r#"<a href="http://example.com" title="The title" target="_top" class="descriptionLink">label</a>"#
        );
        // Attribute-escaped only, never percent-encoded.
        assert_eq!(
            html("[a](<x y?a=1&b=\"2\"> 't \"q\" &')"),
            r#"<a href="x y?a=1&amp;b=&quot;2&quot;" title="t &quot;q&quot; &amp;" target="_top" class="descriptionLink">a</a>"#
        );
        assert_eq!(html("[a](%20%zz)"), link("%20%zz", "a"));
        assert_eq!(html("[a]()"), link("", "a"));
        assert_eq!(
            html("[*a* `b`](u)"),
            link("u", &format!("{ITALIC}a{END} {MONO}b{END}"))
        );
        assert_eq!(
            html("[x]\n\n[x]: /ref \"T\""),
            r#"<a href="/ref" title="T" target="_top" class="descriptionLink">x</a>"#
        );
        for markdown in ["[a](u \"t\nx\")", "[a](u \"t\r\nx\")", "[a](u \"t\rx\")"] {
            assert_eq!(
                html(markdown),
                "<a href=\"u\" title=\"t\nx\" target=\"_top\" class=\"descriptionLink\">a</a>",
                "{markdown:?}"
            );
        }
        // Autolinks.
        assert_eq!(
            html("<https://a.b/c>"),
            link("https://a.b/c", "https://a.b/c")
        );
        assert_eq!(
            html("https://a.b/c?d=1"),
            link("https://a.b/c?d=1", "https://a.b/c?d=1")
        );
        assert_eq!(
            html("www.example.com"),
            link("http://www.example.com", "www.example.com")
        );
        assert_eq!(html("a@b.cd"), link("mailto:a@b.cd", "a@b.cd"));
        assert_eq!(html("workitemimg:1-a.png"), "workitemimg:1-a.png");
    }

    #[test]
    fn dangerous_links_keep_only_their_label() {
        for url in [
            "javascript:alert(1)",
            "JAVASCRIPT:alert(1)",
            "<  javascript:alert(1)>",
            "javascript&#58;alert(1)",
            "vbscript:x",
            "file:///etc/passwd",
            "data:text/html;base64,PHNjcmlwdD4=",
        ] {
            let markdown = format!("[*x* PROJ-1]({url})");
            assert_eq!(
                html(&markdown),
                format!("{ITALIC}x{END} PROJ-1"),
                "{markdown}"
            );
        }
        assert_eq!(
            html("[x](data:image/png;base64,AAAA)"),
            link("data:image/png;base64,AAAA", "x")
        );
    }

    #[test]
    fn images() {
        assert_eq!(
            html("![](a.png)"),
            r#"<img src="a.png" style="max-width: 650px;"/>"#
        );
        assert_eq!(
            html("![alt *x* `c` $m$](a.png \"T\")"),
            r#"<img src="a.png" title="T" alt="alt x c m" style="max-width: 650px;"/>"#
        );
        assert_eq!(
            html("![a \"q\" & b](<a b.png> \"t&\")"),
            r#"<img src="a b.png" title="t&amp;" alt="a &quot;q&quot; &amp; b" style="max-width: 650px;"/>"#
        );
        assert_eq!(html("![alt <b>x</b>](javascript:alert(1))"), "alt x");
        assert_eq!(
            html("![PROJ-1](x.png)"),
            r#"<img src="x.png" alt="PROJ-1" style="max-width: 650px;"/>"#
        );
        assert_eq!(
            html("[![i](i.png)](u)"),
            link(
                "u",
                r#"<img src="i.png" alt="i" style="max-width: 650px;"/>"#
            )
        );
        assert_eq!(
            html("![p](data:image/png;base64,AAAA)"),
            r#"<img src="data:image/png;base64,AAAA" alt="p" style="max-width: 650px;"/>"#
        );
    }

    #[test]
    fn formulas() {
        assert_eq!(html("$x$"), formula("x", false));
        assert_eq!(
            html(r"$\sqrt{x}$ and $$\int_0^1$$"),
            format!(
                "{} and {}",
                formula(r"\sqrt{x}", false),
                formula(r"\int_0^1", true)
            )
        );
        assert_eq!(html("$`x < y`$"), formula("x &lt; y", false));
        assert_eq!(html("$a\"&<b$"), formula("a&quot;&amp;&lt;b", false));
        assert_eq!(html("$$a\nb$$"), formula("a\nb", true));
        assert_eq!(
            html("```math\na^2 \\\\\nb\n```"),
            formula("a^2 \\\\\nb", true)
        );
        assert_eq!(html("~~~MATH\nx\n~~~"), formula("x", true));
        assert_eq!(html("```math\n```"), formula("", true));
        // Line breaks: spaces in inline formulas, kept in display formulas.
        for eol in ["\n", "\r\n", "\r"] {
            assert_eq!(
                html(&format!("$a{eol}b$")),
                formula("a b", false),
                "{eol:?}"
            );
            assert_eq!(
                html(&format!("$$a{eol}b$$")),
                formula("a\nb", true),
                "{eol:?}"
            );
            assert_eq!(
                html(&format!("```math{eol}a{eol}b{eol}```")),
                formula("a\nb", true),
                "{eol:?}"
            );
        }
        // Not formulas.
        assert_eq!(html("$ x $ \\$y\\$ $5"), "$ x $ $y$ $5");
    }

    #[test]
    fn code_blocks() {
        assert_eq!(
            html("```\nline1\nline2\n```"),
            format!("{PRE}line1\nline2</pre>")
        );
        assert_eq!(
            html("    indented\n    code"),
            format!("{PRE}indented\ncode</pre>")
        );
        assert_eq!(
            html("~~~\n<a> & b\n~~~"),
            format!("{PRE}&lt;a&gt; &amp; b</pre>")
        );
        assert_eq!(html("```\n```"), format!("{PRE}</pre>"));
        assert_eq!(html("```\n\n```"), format!("{PRE}</pre>"));
        // A leading line break gets one more: HTML parsers drop the first.
        assert_eq!(html("```\n\nx\n```"), format!("{PRE}\n\nx</pre>"));
        assert_eq!(html("```\nx\n\n```"), format!("{PRE}x\n</pre>"));
        // Lines end in `\n`, whatever the Markdown's line endings.
        for markdown in [
            "```\r\na\r\nb\r\n```",
            "```\ra\rb\r```",
            "    a\r\n    b\r\n",
            "~~~\r\na\rb\n~~~",
        ] {
            assert_eq!(html(markdown), format!("{PRE}a\nb</pre>"), "{markdown:?}");
        }
        // The language is the first word of the info string, as written.
        let pre = |lang: &str| PRE.replace("<pre ", &format!("<pre class=\"language-{lang}\" "));
        assert_eq!(
            plain("```rust title=\"x\"\nfn x() {}\n```"),
            format!("{}fn x() {{}}</pre>", pre("rust"))
        );
        assert_eq!(plain("```Rust\nx\n```"), format!("{}x</pre>", pre("Rust")));
        assert_eq!(
            plain("~~~ c\"x\n<y>\n~~~"),
            format!("{}&lt;y&gt;</pre>", pre("c&quot;x"))
        );
        // Unknown languages and plain text are never highlighted.
        assert_eq!(
            html("```no-such-language\na<b\n```"),
            format!("{}a&lt;b</pre>", pre("no-such-language"))
        );
        assert_eq!(
            html("```text\nPROJ-1\n```"),
            format!("{}PROJ-1</pre>", pre("text"))
        );
    }

    #[test]
    fn highlighted_code_blocks() {
        let html = html("```rust\nfn main() {\n    let s = \"<&>\";\n}\n```");
        let open = PRE.replace("<pre ", "<pre class=\"language-rust\" ");
        assert!(
            html.starts_with(&open) && html.ends_with("</pre>"),
            "{html}"
        );
        let content = &html[open.len()..html.len() - "</pre>".len()];
        if cfg!(feature = "highlight") {
            assert!(
                content.starts_with("<span style=\"") && content.contains("color:#"),
                "{html}"
            );
            assert!(!content.contains("background-color"), "{html}");
            assert!(
                !content.ends_with('\n') && !content.ends_with("\n</span>"),
                "{html}"
            );
        } else {
            assert!(!content.contains('<'), "{html}");
        }
        // The text is the code either way.
        let text = strip_tags(content);
        assert_eq!(text, "fn main() {\n    let s = \"<&>\";\n}");
        // A leading blank line survives highlighting too.
        let html = super::html("```rust\n\nlet x = 1;\n```");
        assert!(html.starts_with(&format!("{open}\n")), "{html}");
        assert_eq!(
            strip_tags(&html[open.len() + 1..html.len() - 6]),
            "\nlet x = 1;"
        );
        // Code with a line longer than 512 bytes stays plain: a line is highlighted at once, and
        // on some grammars a line this long already takes more than 100 ms.
        let long = "x".repeat(512 + 1);
        assert_eq!(
            super::html(&format!("```rust\nlet a = 1;\n{long}\n```")),
            format!("{open}let a = 1;\n{long}</pre>")
        );
        let html = super::html(&format!("```rust\nlet a = 1;\n{}\n```", &long[1..]));
        if cfg!(feature = "highlight") {
            assert!(html.contains("<span style=\""), "{html}");
        }
    }

    fn strip_tags(html: &str) -> String {
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
        text.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&amp;", "&")
    }

    #[test]
    fn mermaid_blocks() {
        let markdown = "```mermaid\ngraph TD\n    A-->B\n```";
        let diagram = mermaid::diagram("graph TD\n    A-->B");
        let file = &diagram.file_name;
        // Without an attachment: a code block, never highlighted.
        let pre = PRE.replace("<pre ", "<pre class=\"language-mermaid\" ");
        assert_eq!(html(markdown), format!("{pre}graph TD\n    A--&gt;B</pre>"));
        // With one: its image.
        let mut diagrams = DiagramAttachments::new();
        diagrams.insert(file.clone(), format!("3-{file}"));
        let image = format!(
            r#"<img src="workitemimg:3-{file}" title="{file}" style="max-width: 650px;"/>"#
        );
        assert_eq!(
            to_html_with_diagrams(markdown, &ToHtmlOptions::default(), &diagrams),
            image
        );
        // The same diagram written differently (CRLF, trailing spaces, info words) is the same image.
        let variant = "~~~ Mermaid theme\r\n\r\ngraph TD  \r\n    A-->B\r\n\r\n~~~";
        assert_eq!(
            to_html_with_diagrams(variant, &ToHtmlOptions::default(), &diagrams),
            image
        );
        // Other diagrams stay code blocks; an attachment id is percent-encoded.
        let other = "```mermaid\ngraph LR\n    X-->Y\n```";
        assert!(
            to_html_with_diagrams(other, &ToHtmlOptions::default(), &diagrams)
                .starts_with("<pre class=\"language-mermaid\"")
        );
        let other_file = mermaid::diagram("graph LR\n    X-->Y").file_name;
        diagrams.insert(other_file.clone(), "4-a b\"&.svg".to_owned());
        assert_eq!(
            to_html_with_diagrams(other, &ToHtmlOptions::default(), &diagrams),
            format!(
                r#"<img src="workitemimg:4-a%20b%22%26.svg" title="{other_file}" style="max-width: 650px;"/>"#
            )
        );
        // The image's reference reads back as the attachment id.
        let both = to_html_with_diagrams(
            &format!("{markdown}\n\n{other}"),
            &ToHtmlOptions::default(),
            &diagrams,
        );
        assert_eq!(
            attachment_refs(&both).into_iter().collect::<Vec<_>>(),
            [format!("3-{file}"), "4-a b\"&.svg".to_owned()]
        );
        assert_eq!(
            diagram_refs(&both).into_iter().collect::<Vec<_>>(),
            [format!("3-{file}")]
        );
    }

    /// The file name of a diagram is pinned: changing the hash or the profile names every
    /// existing diagram anew, so each is rendered and uploaded again.
    #[test]
    fn diagram_file_names_are_pinned() {
        let file = "mermaid-744d43fa10d8c687.svg";
        let image = format!(
            r#"<img src="workitemimg:1-{file}" title="{file}" style="max-width: 650px;"/>"#
        );
        let diagrams: DiagramAttachments = [(file.to_owned(), format!("1-{file}"))]
            .into_iter()
            .collect();
        for markdown in [
            "```mermaid\ngraph TD\n    A-->B\n```",
            "```mermaid\r\ngraph TD  \r\n    A-->B\t\r\n\r\n```",
        ] {
            assert_eq!(
                mermaid::diagrams(markdown)
                    .into_iter()
                    .map(|d| d.file_name)
                    .collect::<Vec<_>>(),
                [file],
                "{markdown:?}"
            );
            assert_eq!(
                to_html_with_diagrams(markdown, &ToHtmlOptions::default(), &diagrams),
                image,
                "{markdown:?}"
            );
        }
    }

    /// The words of an info string end at any whitespace, a no-break space too, wherever they
    /// are read (the language, a diagram, a directive's name): ` ```mermaid ` is a diagram,
    /// so its class must name mermaid alone.
    #[test]
    fn info_string_words() {
        let pre = |lang: &str| PRE.replace("<pre ", &format!("<pre class=\"language-{lang}\" "));
        assert_eq!(
            html("```mermaid\u{a0}x\ngraph TD\n```"),
            format!("{}graph TD</pre>", pre("mermaid"))
        );
        let file = mermaid::diagram("graph TD").file_name;
        let diagrams: DiagramAttachments =
            [(file.clone(), format!("1-{file}"))].into_iter().collect();
        assert_eq!(
            to_html_with_diagrams(
                "```mermaid\u{a0}x\ngraph TD\n```",
                &ToHtmlOptions::default(),
                &diagrams
            ),
            format!(
                r#"<img src="workitemimg:1-{file}" title="{file}" style="max-width: 650px;"/>"#
            )
        );
        assert_eq!(
            plain("```rust\u{2003}title\nx\n```"),
            format!("{}x</pre>", pre("rust"))
        );
        assert_eq!(html("```math\u{a0}x\na\n```"), formula("a", true));
        assert_eq!(
            html(":::note\u{a0}Title\nx\n:::"),
            format!("{}x</div>", alert_open("#0969DA", "Title"))
        );
        assert_eq!(
            html(":::tip\u{3000}\nx\n:::"),
            format!("{}x</div>", alert_open("#1A7F37", "Tip"))
        );
    }

    #[test]
    fn tables() {
        let markdown = "| a | b | c | d |\n|---|:--|:-:|--:|\n| 1 | 2 | 3 | 4 |";
        assert_eq!(
            html(markdown),
            format!(
                "{TABLE}<tr>{}a</th>{}b</th>{}c</th>{}d</th></tr><tr>{}1</td>{}2</td>{}3</td>{}4</td></tr>{TABLE_END}",
                th("left"),
                th("left"),
                th("center"),
                th("right"),
                td("left"),
                td("left"),
                td("center"),
                td("right")
            )
        );
        // Short rows are padded, long ones cut; cells hold inlines.
        let markdown = "| a | b |\n|---|--:|\n| **x** PROJ-1 |\n| `p\\|q` | 2 | 3 |";
        assert_eq!(
            html(markdown),
            format!(
                "{TABLE}<tr>{}a</th>{}b</th></tr><tr>{}{BOLD}x{END} {}</td>{}</td></tr><tr>{}{MONO}p|q{END}</td>{}2</td></tr>{TABLE_END}",
                th("left"),
                th("right"),
                td("left"),
                wi("PROJ-1"),
                td("right"),
                td("left"),
                td("right")
            )
        );
        // Only a header row.
        assert_eq!(
            html("| h |\n|---|"),
            format!("{TABLE}<tr>{}h</th></tr>{TABLE_END}", th("left"))
        );
    }

    #[test]
    fn lists() {
        assert_eq!(html("- a\n- b"), "<ul><li>a</li><li>b</li></ul>");
        assert_eq!(
            html("* a\n+ b"),
            "<ul><li>a</li></ul><br/><ul><li>b</li></ul>"
        );
        assert_eq!(html("1. a\n2. b"), "<ol><li>a</li><li>b</li></ol>");
        assert_eq!(
            html("3. a\n4. b"),
            "<ol start=\"3\"><li>a</li><li>b</li></ol>"
        );
        assert_eq!(html("0) a"), "<ol start=\"0\"><li>a</li></ol>");
        assert_eq!(
            html("- a\n  - b\n    1. c"),
            "<ul><li>a<ul><li>b<ol><li>c</li></ol></li></ul></li></ul>"
        );
        // Loose and tight lists look the same.
        assert_eq!(html("- a\n\n- b"), "<ul><li>a</li><li>b</li></ul>");
        assert_eq!(html("- a\n\n  b"), "<ul><li>a<br/><br/>b</li></ul>");
        assert_eq!(html("-"), "<ul><li></li></ul>");
    }

    #[test]
    fn task_lists() {
        assert_eq!(
            html("- [ ] open\n- [x] done\n- [X] done"),
            "<ul><li>☐ open</li><li>☑ done</li><li>☑ done</li></ul>"
        );
        assert_eq!(
            html("1. [ ] a\n2. [x] **b**"),
            format!("<ol><li>☐ a</li><li>☑ {BOLD}b{END}</li></ol>")
        );
        assert_eq!(html("- [ ]"), "<ul><li>☐ </li></ul>");
        assert_eq!(html("- [ ] a\n\n  b"), "<ul><li>☐ a<br/><br/>b</li></ul>");
    }

    #[test]
    fn quotes() {
        assert_eq!(html("> a"), format!("{QUOTE}a</div>"));
        assert_eq!(html("> a\n> b"), format!("{QUOTE}a<br/>b</div>"));
        assert_eq!(html("> a\n>\n> b"), format!("{QUOTE}a<br/><br/>b</div>"));
        assert_eq!(
            html("> a\n> > b"),
            format!("{QUOTE}a<br/><br/>{QUOTE}b</div></div>")
        );
        assert_eq!(html(">> b"), format!("{QUOTE}{QUOTE}b</div></div>"));
        assert_eq!(html("> - a"), format!("{QUOTE}<ul><li>a</li></ul></div>"));
        assert_eq!(html(">"), format!("{QUOTE}</div>"));
    }

    #[test]
    fn alerts() {
        let alerts = [
            ("NOTE", "Note", "#0969DA"),
            ("TIP", "Tip", "#1A7F37"),
            ("IMPORTANT", "Important", "#8250DF"),
            ("WARNING", "Warning", "#9A6700"),
            ("CAUTION", "Caution", "#CF222E"),
        ];
        for (name, title, color) in alerts {
            let expected = format!("{}body</div>", alert_open(color, title));
            assert_eq!(html(&format!("> [!{name}]\n> body")), expected, "{name}");
            assert_eq!(
                html(&format!(":::{}\nbody\n:::", name.to_lowercase())),
                expected,
                "{name}"
            );
            assert_eq!(html(&format!(":::{name}\nbody\n:::")), expected, "{name}");
        }
        assert_eq!(
            html("> [!WARNING] Heads <up>\n> a\n>\n> b"),
            format!(
                "{}a<br/><br/>b</div>",
                alert_open("#9A6700", "Heads &lt;up&gt;")
            )
        );
        assert_eq!(
            html(":::tip Do this\nx\n:::"),
            format!("{}x</div>", alert_open("#1A7F37", "Do this"))
        );
        assert_eq!(
            html("> [!NOTE]"),
            format!("{}</div>", alert_open("#0969DA", "Note"))
        );
        assert_eq!(
            html("> [!NOTE]\n> - a"),
            format!("{}<ul><li>a</li></ul></div>", alert_open("#0969DA", "Note"))
        );
        // Other directives are plain divs; `> [!OTHER]` is a quote.
        assert_eq!(html(":::custom thing\nx\n:::"), "<div>x</div>");
        assert_eq!(
            html("> [!OTHER]\n> x"),
            format!("{QUOTE}[!OTHER]<br/>x</div>")
        );
    }

    #[test]
    fn thematic_breaks() {
        assert_eq!(html("---"), "<hr/>");
        assert_eq!(html("***\n___"), "<hr/><br/><hr/>");
    }

    #[test]
    fn raw_html() {
        assert_eq!(
            html("a <span style=\"color: red;\">red</span> b"),
            "a <span style=\"color: red;\">red</span> b"
        );
        assert_eq!(html("a <!-- comment --> b"), "a  b");
        assert_eq!(html("a <br> b<br/>c"), "a <br> b<br/>c");
        assert_eq!(
            html("<div style=\"x\">\n*not md*\n</div>"),
            "<div style=\"x\">\n*not md*\n</div>"
        );
        assert_eq!(
            html("  <table><tr><td>x</td></tr></table>  \n\n"),
            "<table><tr><td>x</td></tr></table>"
        );
        assert_eq!(html("<!-- only a comment -->"), "");
        assert_eq!(html("<!--\nmulti\nline\n-->\n\nafter"), "after");
        assert_eq!(html("<div>\n<!-- c -->\nx\n</div>"), "<div>\n\nx\n</div>");
        assert_eq!(
            html("<pre>\n  keep *this*\n</pre>"),
            "<pre>\n  keep *this*\n</pre>"
        );
        assert_eq!(html("<?php echo 1; ?>"), "<?php echo 1; ?>");
        // comrak's `<!-- end list -->` between two lists is dropped.
        assert_eq!(
            html("- a\n\n<!-- end list -->\n\n- b"),
            "<ul><li>a</li></ul><br/><ul><li>b</li></ul>"
        );
    }

    #[test]
    fn footnotes() {
        assert_eq!(
            html("Text[^1].\n\n[^1]: The note."),
            "Text<sup>[1]</sup>.<br/><br/><hr/><br/><sup>[1]</sup> The note.".to_string()
        );
        // Numbered by first reference; repeated references share the number; unused ones vanish.
        assert_eq!(
            html("a[^x] b[^y] c[^x]\n\n[^y]: Y *y*\n\n[^x]: X\n\n[^z]: unused"),
            format!(
                "a<sup>[1]</sup> b<sup>[2]</sup> c<sup>[1]</sup><br/><br/><hr/><br/><sup>[1]</sup> X<br/><br/><sup>[2]</sup> Y {ITALIC}y{END}"
            )
        );
        assert_eq!(
            html("a[^1]\n\n[^1]: one\n\n    two"),
            "a<sup>[1]</sup><br/><br/><hr/><br/><sup>[1]</sup> one<br/><br/>two"
        );
        assert_eq!(
            html("a[^1]\n\n[^1]: - item"),
            "a<sup>[1]</sup><br/><br/><hr/><br/><sup>[1]</sup> <ul><li>item</li></ul>"
        );
        assert_eq!(html("[^missing]"), "[^missing]");
    }

    #[test]
    fn description_lists() {
        assert_eq!(
            html("Term\n\n: Details"),
            format!("{BOLD}Term{END}<br/><br/>{QUOTE}Details</div>")
        );
        assert_eq!(
            html("T1\n: d1\n: d2\n\nT2\n\n: d3\n\n  more"),
            format!(
                "{BOLD}T1{END}<br/><br/>{QUOTE}d1</div><br/>{QUOTE}d2</div><br/>{BOLD}T2{END}<br/><br/>{QUOTE}d3<br/><br/>more</div>"
            )
        );
        assert_eq!(
            html("*T* PROJ-1\n: d"),
            format!(
                "{BOLD}{ITALIC}T{END} {}{END}<br/><br/>{QUOTE}d</div>",
                wi("PROJ-1")
            )
        );
    }
}

mod separators {
    use super::*;

    #[test]
    fn between_text_blocks() {
        assert_eq!(html("a\n\nb"), "a<br/><br/>b");
        assert_eq!(html("a\n\n\n\n\nb"), "a<br/><br/>b");
        assert_eq!(
            html("a\n\n# H\n\nb"),
            format!("a<br/><br/>{}H{END}<br/><br/>b", heading("22", "1.2"))
        );
        assert_eq!(
            html("$$x$$\n\n```math\ny\n```\n\nz"),
            format!(
                "{}<br/><br/>{}<br/><br/>z",
                formula("x", true),
                formula("y", true)
            )
        );
    }

    #[test]
    fn around_boxes() {
        assert_eq!(html("a\n\n---\n\nb"), "a<br/><br/><hr/><br/>b");
        assert_eq!(html("---\n\n---"), "<hr/><br/><hr/>");
        assert_eq!(
            html("a\n\n> q\n\nb"),
            format!("a<br/><br/>{QUOTE}q</div><br/>b")
        );
        assert_eq!(
            html("- l\n\n```\nc\n```\n\n# H"),
            format!(
                "<ul><li>l</li></ul><br/>{PRE}c</pre><br/>{}H{END}",
                heading("22", "1.2")
            )
        );
        assert_eq!(
            html("a\n\n| h |\n|---|\n| c |\n\nb"),
            format!(
                "a<br/><br/>{TABLE}<tr>{}h</th></tr><tr>{}c</td></tr>{TABLE_END}<br/>b",
                th("left"),
                td("left")
            )
        );
        assert_eq!(
            html("> [!NOTE]\n> x\n\ny"),
            format!("{}x</div><br/>y", alert_open("#0969DA", "Note"))
        );
        assert_eq!(html("a\n- b"), "a<br/><br/><ul><li>b</li></ul>");
    }

    #[test]
    fn nothing_before_the_first_block_or_after_the_last() {
        assert_eq!(html("\n\n\na\n\n\n"), "a");
        assert_eq!(html("---\n\n\n"), "<hr/>");
        assert_eq!(html("> a\n\n"), format!("{QUOTE}a</div>"));
    }

    #[test]
    fn diagram_images_are_text_blocks() {
        let markdown = "a\n\n```mermaid\ngraph TD\n    A-->B\n```\n\nb";
        let file = mermaid::diagram("graph TD\n    A-->B").file_name;
        let diagrams: DiagramAttachments =
            [(file.clone(), format!("1-{file}"))].into_iter().collect();
        assert_eq!(
            to_html_with_diagrams(markdown, &ToHtmlOptions::default(), &diagrams),
            format!(
                r#"a<br/><br/><img src="workitemimg:1-{file}" title="{file}" style="max-width: 650px;"/><br/><br/>b"#
            )
        );
        // Unrendered it is a box.
        let pre = PRE.replace("<pre ", "<pre class=\"language-mermaid\" ");
        assert_eq!(
            html(markdown),
            format!("a<br/><br/>{pre}graph TD\n    A--&gt;B</pre><br/>b")
        );
    }

    #[test]
    fn raw_opening_and_closing_tags() {
        // A raw tag on its own wraps the following blocks.
        assert_eq!(
            html("<div style=\"text-align: center;\">\n\npara\n\n</div>\n\nafter"),
            "<div style=\"text-align: center;\">para</div><br/>after"
        );
        assert_eq!(
            html("<div style=\"text-align: center;\">\n\n| h |\n|---|\n\nb\n\n</div>"),
            format!(
                "<div style=\"text-align: center;\">{TABLE}<tr>{}h</th></tr>{TABLE_END}<br/>b</div>",
                th("left")
            )
        );
        assert_eq!(
            html("<span style=\"color: red;\">\n\ntext\n\n</span>\n\nafter"),
            "<span style=\"color: red;\">text</span><br/><br/>after"
        );
        assert_eq!(html("<div>\n\n</div>"), "<div></div>");
        assert_eq!(html("a\n\n<div>\n\nb\n\n</div>"), "a<br/><br/><div>b</div>");
        assert_eq!(
            html("<div>\n\n<div>\n\nb\n\n</div>\n\n</div>"),
            "<div><div>b</div></div>"
        );
    }

    #[test]
    fn raw_blocks_by_how_they_start_and_end() {
        assert_eq!(html("<div>x</div>\n\nafter"), "<div>x</div><br/>after");
        assert_eq!(
            html("<div>x</div>\ntext\n\nafter"),
            "<div>x</div>\ntext<br/><br/>after"
        );
        assert_eq!(
            html("<div>x</div><br>\n\nafter"),
            "<div>x</div><br><br/>after"
        );
        assert_eq!(html("<hr>\n\nafter"), "<hr><br/>after");
        assert_eq!(
            html("<custom-tag>\nx\n</custom-tag>\n\nafter"),
            "<custom-tag>\nx\n</custom-tag><br/><br/>after"
        );
        assert_eq!(
            html("- a\n\n  <div>x</div>\n\n  b"),
            "<ul><li>a<div>x</div>b</li></ul>"
        );
    }

    /// Raw HTML that starts with a closing tag ends the wrapper the blocks before it are in, and
    /// raw HTML that ends with an opening tag starts a wrapper for the blocks after it, whatever
    /// else it holds: no break is written inside the wrappers.
    #[test]
    fn raw_blocks_closing_and_opening_wrappers() {
        // One wrapper closed and the next opened without a blank line between them.
        assert_eq!(
            html("<div style=\"x\">\n\na\n\n</div>\n<div style=\"y\">\n\nb\n\n</div>"),
            "<div style=\"x\">a</div>\n<div style=\"y\">b</div>"
        );
        assert_eq!(
            html("<div style=\"x\">\n\na\n\n</div>\ntext"),
            "<div style=\"x\">a</div>\ntext"
        );
        assert_eq!(
            html("<div style=\"x\">\n\na\n\n</div>\ntext\n\nafter"),
            "<div style=\"x\">a</div>\ntext<br/><br/>after"
        );
        assert_eq!(
            html("before\n\n<div>x</div>\n<div style=\"y\">\n\nb\n\n</div>"),
            "before<br/><br/><div>x</div>\n<div style=\"y\">b</div>"
        );
        // Inline tags next to other content are text: the paragraphs stay apart.
        assert_eq!(
            html("<span style=\"color: red;\">\n\ntext\n\n</span>\nmore\n\nafter"),
            "<span style=\"color: red;\">text<br/><br/></span>\nmore<br/><br/>after"
        );
        assert_eq!(
            html("<span style=\"color: red;\">\n\ntext\n\n</span> more"),
            "<span style=\"color: red;\">text<br/><br/></span> more"
        );
        assert_eq!(
            html("<p>a</p>\nb <span style=\"color: red;\">\n\ntext"),
            "<p>a</p>\nb <span style=\"color: red;\"><br/><br/>text"
        );
        // Inside list items too.
        assert_eq!(
            html("- <div style=\"x\">\n\n  a\n\n  </div>\n  <div style=\"y\">\n\n  b\n\n  </div>"),
            "<ul><li><div style=\"x\">a</div>\n<div style=\"y\">b</div></li></ul>"
        );
        // HTML ignores `/>` on elements with content: `<div/>` opens a div.
        assert_eq!(html("<div/>\n\npara\n\nmore"), "<div/>para<br/><br/>more");
        assert_eq!(
            html("<div/>\n\npara\n\n</div>\n\nafter"),
            "<div/>para</div><br/>after"
        );
        // Void and self-closed SVG elements open nothing.
        assert_eq!(html("<hr/>\n\npara"), "<hr/><br/>para");
        assert_eq!(
            html("<p>x</p>\n<svg width=\"1\"/>\n\npara"),
            "<p>x</p>\n<svg width=\"1\"/><br/><br/>para"
        );
    }

    #[test]
    fn inside_list_items() {
        assert_eq!(
            html("- a\n\n  b\n\n  c"),
            "<ul><li>a<br/><br/>b<br/><br/>c</li></ul>"
        );
        assert_eq!(
            html("- a\n  - b\n\n  c"),
            "<ul><li>a<ul><li>b</li></ul>c</li></ul>"
        );
        assert_eq!(
            html("- a\n\n  ```\n  c\n  ```\n\n  b"),
            format!("<ul><li>a{PRE}c</pre>b</li></ul>")
        );
        assert_eq!(
            html("- # H\n\n  p"),
            format!(
                "<ul><li>{}H{END}<br/><br/>p</li></ul>",
                heading("22", "1.2")
            )
        );
        assert_eq!(
            html("- > q\n\n  p"),
            format!("<ul><li>{QUOTE}q</div>p</li></ul>")
        );
        assert_eq!(html("- ***"), "<ul><li><hr/></li></ul>");
    }

    #[test]
    fn inside_quotes_and_alerts() {
        assert_eq!(
            html("> a\n>\n> ---\n>\n> b"),
            format!("{QUOTE}a<br/><br/><hr/><br/>b</div>")
        );
        assert_eq!(
            html(":::note\na\n\nb\n:::"),
            format!("{}a<br/><br/>b</div>", alert_open("#0969DA", "Note"))
        );
    }
}

mod work_item_ids {
    use super::*;

    #[test]
    fn left_boundaries() {
        for (markdown, before) in [
            ("PROJ-1", ""),
            ("x PROJ-1", "x "),
            ("(PROJ-1", "("),
            ("[PROJ-1", "["),
            ("{PROJ-1", "{"),
            ("\"PROJ-1", "\""),
            ("'PROJ-1", "'"),
            ("“PROJ-1", "“"),
            ("‘PROJ-1", "‘"),
            ("«PROJ-1", "«"),
            ("x,PROJ-1", "x,"),
            ("x;PROJ-1", "x;"),
            ("x:PROJ-1", "x:"),
            ("x!PROJ-1", "x!"),
            ("x?PROJ-1", "x?"),
            ("x\u{a0}PROJ-1", "x&nbsp;"),
        ] {
            assert_eq!(
                html(markdown),
                format!("{before}{}", wi("PROJ-1")),
                "{markdown}"
            );
        }
        for markdown in [
            "aPROJ-1", "x.PROJ-1", "x-PROJ-1", "x_PROJ-1", "x/PROJ-1", "x#PROJ-1", "x$PROJ-1",
            "1PROJ-1", "éPROJ-1",
        ] {
            assert_eq!(html(markdown), markdown, "{markdown}");
        }
    }

    #[test]
    fn right_boundaries() {
        for (markdown, after) in [
            ("PROJ-1 x", " x"),
            ("PROJ-1)", ")"),
            ("PROJ-1]", "]"),
            ("PROJ-1}", "}"),
            ("PROJ-1\"", "\""),
            ("PROJ-1'", "'"),
            ("PROJ-1”", "”"),
            ("PROJ-1’", "’"),
            ("PROJ-1»", "»"),
            ("PROJ-1,", ","),
            ("PROJ-1;", ";"),
            ("PROJ-1:", ":"),
            ("PROJ-1!", "!"),
            ("PROJ-1?", "?"),
            ("PROJ-1.", "."),
            ("PROJ-1. Next", ". Next"),
            ("PROJ-1..", ".."),
        ] {
            assert_eq!(
                html(markdown),
                format!("{}{after}", wi("PROJ-1")),
                "{markdown}"
            );
        }
        for markdown in [
            "PROJ-1a", "PROJ-1A", "PROJ-1-2", "PROJ-1.5", "PROJ-1.a", "PROJ-1/x", "PROJ-1#",
            "PROJ-1é", "PROJ-1%",
        ] {
            assert_eq!(html(markdown), markdown, "{markdown}");
        }
        assert_eq!(html("PROJ-1_"), "PROJ-1_");
    }

    #[test]
    fn in_sentences() {
        assert_eq!(
            html("See PROJ-9066 for details"),
            format!("See {} for details", wi("PROJ-9066"))
        );
        assert_eq!(
            html("See MP-100 and PROJ-200 for details"),
            format!("See {} and {} for details", wi("MP-100"), wi("PROJ-200"))
        );
        assert_eq!(
            html("Fixed bug (MP-42)"),
            format!("Fixed bug ({})", wi("MP-42"))
        );
        assert_eq!(html("No work items here"), "No work items here");
    }

    #[test]
    fn shapes() {
        assert_eq!(html("MY_PROJ-12"), wi("MY_PROJ-12"));
        assert_eq!(html("A1-007"), wi("A1-007"));
        assert_eq!(html("UTF-8"), wi("UTF-8"));
        for markdown in [
            "A-1", "Proj-1", "PROJ-", "PROJ-x", "proj-1", "PROJ_1", "PRÖJ-1",
        ] {
            assert_eq!(html(markdown), markdown, "{markdown}");
        }
        assert_eq!(
            html("PROJ-1 MP-42,UTF-8"),
            format!("{} {},{}", wi("PROJ-1"), wi("MP-42"), wi("UTF-8"))
        );
    }

    #[test]
    fn escapes() {
        assert_eq!(html("PROJ\\-12"), "PROJ-12");
        assert_eq!(html("MY\\_PROJ-12"), wi("MY_PROJ-12"));
        assert_eq!(html("MY\\_PROJ\\-12"), "MY_PROJ-12");
        assert_eq!(html("\\(PROJ-12\\)"), format!("({})", wi("PROJ-12")));
        assert_eq!(html("\\PROJ-12"), "\\PROJ-12");
        assert_eq!(html("PROJ-1\\2"), "PROJ-1\\2");
    }

    #[test]
    fn settings() {
        let markdown = "MP-1 PROJ-2 UTF-8";
        assert_eq!(
            with_ids(markdown, WorkItemIds::Any),
            format!("{} {} {}", wi("MP-1"), wi("PROJ-2"), wi("UTF-8"))
        );
        assert_eq!(
            with_ids(markdown, WorkItemIds::Prefixes(vec!["MP".into()])),
            format!("{} PROJ-2 UTF-8", wi("MP-1"))
        );
        assert_eq!(
            with_ids(
                markdown,
                WorkItemIds::Prefixes(vec!["PROJ".into(), "MP".into()])
            ),
            format!("{} {} UTF-8", wi("MP-1"), wi("PROJ-2"))
        );
        assert_eq!(with_ids(markdown, WorkItemIds::Prefixes(vec![])), markdown);
        assert_eq!(with_ids(markdown, WorkItemIds::Off), markdown);
        assert_eq!(
            with_ids("MPX-1", WorkItemIds::Prefixes(vec!["MP".into()])),
            "MPX-1"
        );
    }

    #[test]
    fn contexts() {
        let id = wi("PROJ-1");
        // Linked in text of any inline or block container.
        assert_eq!(html("**PROJ-1**"), format!("{BOLD}{id}{END}"));
        assert_eq!(html("*a PROJ-1*"), format!("{ITALIC}a {id}{END}"));
        assert_eq!(
            html("# PROJ-1"),
            format!("{}{id}{END}", heading("22", "1.2"))
        );
        assert_eq!(html("> PROJ-1"), format!("{QUOTE}{id}</div>"));
        assert_eq!(html("- [ ] PROJ-1"), format!("<ul><li>☐ {id}</li></ul>"));
        assert_eq!(html("see\nPROJ-1"), format!("see<br/>{id}"));
        assert_eq!(html("x^PROJ-1^"), format!("x<sup>{id}</sup>"));
        // A run of text starts after any other inline.
        assert_eq!(html("`x`PROJ-1"), format!("{MONO}x{END}{id}"));
        assert_eq!(html("x**PROJ**-1"), format!("x{BOLD}PROJ{END}-1"));
        // Never in code, links, images or raw HTML.
        assert_eq!(html("`PROJ-1`"), format!("{MONO}PROJ-1{END}"));
        assert_eq!(html("```\nPROJ-1\n```"), format!("{PRE}PROJ-1</pre>"));
        assert_eq!(html("[PROJ-1](u)"), link("u", "PROJ-1"));
        assert_eq!(
            html("[**PROJ-1**](u)"),
            link("u", &format!("{BOLD}PROJ-1{END}"))
        );
        assert_eq!(
            html("https://x.y/PROJ-1"),
            link("https://x.y/PROJ-1", "https://x.y/PROJ-1")
        );
        assert_eq!(
            html("![PROJ-1](i.png)"),
            r#"<img src="i.png" alt="PROJ-1" style="max-width: 650px;"/>"#
        );
        assert_eq!(html("<div>\nPROJ-1\n</div>"), "<div>\nPROJ-1\n</div>");
        assert_eq!(
            html("<span title=\"PROJ-1\">x</span>"),
            "<span title=\"PROJ-1\">x</span>"
        );
        assert_eq!(
            html("<a href=\"u\">PROJ-1 **PROJ-2**</a> PROJ-3"),
            format!(
                "<a href=\"u\">PROJ-1 {BOLD}PROJ-2{END}</a> {}",
                wi("PROJ-3")
            )
        );
        // To HTML `<a/>` opens a link like `<a>`.
        assert_eq!(html("<a href=\"u\"/>PROJ-1"), "<a href=\"u\"/>PROJ-1");
        assert_eq!(html("$PROJ-1$"), formula("PROJ-1", false));
        // Highlighted code is code too.
        let code = html("```rust\nlet x = PROJ-456;\n```");
        assert!(
            code.contains("PROJ") && !code.contains("polarion-rte-link"),
            "{code}"
        );
    }

    /// A raw `<a>` stays open as a browser keeps it: across paragraphs and blocks, and out of
    /// the emphasis it was opened in (the browser opens it again there). Until it is closed, ids
    /// are not linked.
    #[test]
    fn inside_raw_links_opened_elsewhere() {
        let id = wi("PROJ-3");
        assert_eq!(
            html("<a href=\"u\">PROJ-1\n\nPROJ-2</a> PROJ-3"),
            format!("<a href=\"u\">PROJ-1<br/><br/>PROJ-2</a> {id}")
        );
        assert_eq!(
            html("<a href=\"u\">\n\nPROJ-1\n\n</a>\n\nPROJ-3"),
            format!("<a href=\"u\">PROJ-1</a><br/><br/>{id}")
        );
        assert_eq!(
            html("*<a href=\"u\">x*PROJ-1</a> PROJ-3"),
            format!("{ITALIC}<a href=\"u\">x{END}PROJ-1</a> {id}")
        );
        assert_eq!(
            html("<div>\n<a href=\"u\">\n</div>\n\n- PROJ-1\n\n> PROJ-2\n\n</a> PROJ-3"),
            format!(
                "<div>\n<a href=\"u\">\n</div><br/><ul><li>PROJ-1</li></ul><br/>{QUOTE}PROJ-2</div><br/></a> {id}"
            )
        );
        // `<a/>` and an `<a>` written in pieces open one too; one `</a>` closes nested ones,
        // which HTML does not nest.
        assert_eq!(
            html("<a/>\n\nPROJ-1</a> PROJ-3"),
            format!("<a/>PROJ-1</a> {id}")
        );
        assert_eq!(
            html("<a href=1><a href=2>x</a> PROJ-3"),
            format!("<a href=1><a href=2>x</a> {id}")
        );
        // A Markdown link closes a raw one, as its `<a>` does in a browser.
        assert_eq!(
            html("<a href=\"u\">PROJ-1 [x](v) PROJ-3"),
            format!("<a href=\"u\">PROJ-1 {} {id}", link("v", "x"))
        );
        // A raw `<a>` in a table cell ends with the cell; one opened before the table holds it.
        let row = |cells: [&str; 2]| {
            format!(
                "<tr>{}{}</td>{}{}</td></tr>",
                td("left"),
                cells[0],
                td("left"),
                cells[1]
            )
        };
        let header = format!("<tr>{}a</th>{}b</th></tr>", th("left"), th("left"));
        assert_eq!(
            html("| a | b |\n|---|---|\n| <a href=\"u\"> PROJ-1 | PROJ-3 |"),
            format!(
                "{TABLE}{header}{}{TABLE_END}",
                row(["<a href=\"u\"> PROJ-1", &id])
            )
        );
        assert_eq!(
            html("<a href=\"u\">\n\n| a | b |\n|---|---|\n| PROJ-1 | PROJ-2 |\n\n</a> PROJ-3"),
            format!(
                "<a href=\"u\">{TABLE}{header}{}{TABLE_END}<br/></a> {id}",
                row(["PROJ-1", "PROJ-2"])
            )
        );
        // Text that looks like a link is none.
        assert_eq!(
            html("<span title=\"<a href=u>\">x</span> `<a>` PROJ-3"),
            format!("<span title=\"<a href=u>\">x</span> {MONO}&lt;a&gt;{END} {id}")
        );
    }
}

mod sanitizer {
    use super::*;

    #[test]
    fn dangerous_raw_html_is_neutralized() {
        let cases = [
            (
                "x <img src=\"a.png\" onerror=\"alert(1)\"> y",
                "x <img src=\"a.png\"> y",
            ),
            ("<img src=x onerror=alert(1)>", "<img src=x>"),
            ("x <a href=\"javascript:alert(1)\">a</a>", "x <a>a</a>"),
            (
                "x <a href=\"&#106;avascript:alert(1)\" title=\"t\">a</a>",
                "x <a title=\"t\">a</a>",
            ),
            (
                "<div onclick=\"x()\" style=\"color: red;\">\nd\n</div>",
                "<div style=\"color: red;\">\nd\n</div>",
            ),
            (
                "<p style=\"background: url('javascript:alert(1)')\">\nx\n</p>",
                "<p>\nx\n</p>",
            ),
            ("<form action=\"javascript:x\">\n</form>", "<form>\n</form>"),
            (
                "<iframe src=\"https://evil\"></iframe>",
                "&lt;iframe src=\"https://evil\">&lt;/iframe>",
            ),
            (
                "x <script>alert(1)</script>",
                "x &lt;script>alert(1)&lt;/script>",
            ),
            (
                "<script>\nalert(1)\n</script>",
                "&lt;script>\nalert(1)\n&lt;/script>",
            ),
            (
                "<style>p { color: red }</style>",
                "&lt;style>p { color: red }&lt;/style>",
            ),
            (
                "x <textarea>t</textarea> <title>t</title>",
                "x &lt;textarea>t&lt;/textarea> &lt;title>t&lt;/title>",
            ),
            // Elements that embed other documents or change how the page loads and resolves
            // URLs are filtered too.
            (
                "x <object data=\"https://x/y\"></object> <embed src=\"https://x/z\">",
                "x &lt;object data=\"https://x/y\">&lt;/object> &lt;embed src=\"https://x/z\">",
            ),
            (
                "<base href=\"https://x/\">\n<meta http-equiv=\"refresh\" content=\"0\">\n<link rel=\"stylesheet\" href=\"a.css\">",
                "&lt;base href=\"https://x/\">\n&lt;meta http-equiv=\"refresh\" content=\"0\">\n&lt;link rel=\"stylesheet\" href=\"a.css\">",
            ),
            (
                "<svg><a xlink:href=\"javascript:x\">y</a></svg>",
                "<svg><a>y</a></svg>",
            ),
            (
                "<div>\n<img src=\"x\" onerror=\"y\"\n",
                "<div>\n&lt;img src=\"x\" onerror=\"y\"",
            ),
            // To a browser `<scr<!-- -->` is a tag named `scr<!--`: no comment, no script.
            (
                "<div>\n<scr<!-- -->ipt>alert(1)</script>\n</div>",
                "<div>\n<scr<!-- -->ipt>alert(1)&lt;/script>\n</div>",
            ),
            (
                "<div>\n<<!-- -->script>alert(1)\n</div>",
                "<div>\n&lt;script>alert(1)\n</div>",
            ),
        ];
        for (markdown, expected) in cases {
            assert_eq!(html(markdown), expected, "{markdown}");
        }
    }

    #[test]
    fn clean_raw_html_stays_byte_identical() {
        for raw in [
            "<span style=\"color: #FF0000;font-size: 12pt\">x</span>",
            "<a href=\"https://example.com/a?b=1&amp;c=2\" target=\"_blank\" data-x='y'>z</a>",
            "<img src=\"workitemimg:1-a.png\" style=\"width: 100px;height: 74px;\" data-lastmodified=\"1\">",
            "<SPAN Style = 'x'>y</SPAN>",
            "<sup>1</sup><sub>2</sub><u>u</u><font color=\"red\">f</font>",
        ] {
            let markdown = format!("a {raw} b");
            assert_eq!(html(&markdown), markdown, "{raw}");
        }
        let block = "<table class=\"polarion-Document-table\" style=\"width: 80%;\">\n<tbody>\n<tr><td colspan=\"2\">x</td></tr>\n</tbody>\n</table>";
        assert_eq!(html(block), block);
    }

    /// A deterministic xorshift generator.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }

        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[self.below(items.len())]
        }
    }

    /// A tag built from what browsers read differently: separators (`/`, form feed, nothing
    /// after a quote), quoting, `=` in odd places, names and values that look dangerous.
    fn random_tag(rng: &mut Rng) -> String {
        const NAMES: [&str; 22] = [
            "a", "A", "img", "svg", "animate", "set", "div", "span", "form", "button", "iframe",
            "script", "object", "x-y", "math", "use", "image", "base", "embed", "input", "video",
            "td",
        ];
        const ATTRS: [&str; 31] = [
            "href",
            "HREF",
            "src",
            "xlink:href",
            "action",
            "formaction",
            "data",
            "srcdoc",
            "style",
            "onclick",
            "onerror",
            "ONLOAD",
            "on",
            "values",
            "to",
            "from",
            "attributeName",
            "title",
            "class",
            "background",
            "poster",
            "codebase",
            "dynsrc",
            "lowsrc",
            "x",
            "=",
            "\"",
            "'",
            "<",
            "=x",
            "a\"b",
        ];
        const VALUES: [&str; 26] = [
            "javascript:alert(1)",
            "JaVaScRiPt:x",
            " javascript:x",
            "java\tscript:x",
            "java&#x09;script:x",
            "&#106;avascript:x",
            "javascript&colon;x",
            "vbscript:x",
            "data:text/html,x",
            "https://ok",
            "ok",
            "",
            "x y",
            "a\"b",
            "a'b",
            "a>b",
            "a/b",
            "onclick=x",
            "expression(x)",
            "url(javascript:x)",
            "j\\61vascript:x",
            "x onclick=y",
            "x\"onclick=y",
            "javascript:x/",
            "a b=javascript:x",
            "&quot;onclick=x",
        ];
        const SEPS: [&str; 13] = [
            " ", "\t", "\n", "/", "", " / ", "\u{c}", "//", "\r", "\u{0}", "\u{b}", "\u{1}",
            "\u{c}/",
        ];
        let mut tag = String::from("<");
        if rng.below(8) == 0 {
            tag.push('/');
        }
        tag.push_str(rng.pick(&NAMES));
        for _ in 0..rng.below(5) {
            tag.push_str(rng.pick(&SEPS));
            tag.push_str(rng.pick(&ATTRS));
            if rng.below(4) != 0 {
                tag.push_str(rng.pick(&["", " ", "\n"]));
                tag.push('=');
                tag.push_str(rng.pick(&["", " ", "\t"]));
                let quote = rng.pick(&["", "\"", "'"]);
                tag.push_str(quote);
                tag.push_str(rng.pick(&VALUES));
                if rng.below(10) != 0 {
                    tag.push_str(quote);
                }
            }
        }
        tag.push_str(rng.pick(&["", " ", "/", "\n"]));
        tag.push_str(rng.pick(&[">", "/>", ">"]));
        tag
    }

    /// What survives that must not, read as a browser reads the HTML (html5ever): elements the
    /// sanitizer filters, event handlers, `srcdoc`, `javascript:` and `vbscript:` URLs (after
    /// the URL clean-up browsers do), SVG animations to such values, script in styles.
    fn dangers(html: &str) -> Vec<String> {
        let mut found = Vec::new();
        let squeeze = |value: &str| -> String {
            value
                .chars()
                .filter(|c| !c.is_whitespace() && !c.is_control())
                .collect::<String>()
                .to_ascii_lowercase()
        };
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
                let url = value
                    .trim_matches(|c: char| c <= ' ')
                    .replace(['\t', '\n', '\r'], "")
                    .to_ascii_lowercase();
                let dangerous = attr.starts_with("on")
                    || attr == "srcdoc"
                    || ([
                        "href",
                        "src",
                        "xlink:href",
                        "action",
                        "formaction",
                        "data",
                        "background",
                        "poster",
                        "codebase",
                        "dynsrc",
                        "lowsrc",
                    ]
                    .contains(&attr.as_str())
                        && (url.starts_with("javascript:") || url.starts_with("vbscript:")))
                    || (["values", "to", "from", "by"].contains(&attr.as_str())
                        && (squeeze(value).contains("javascript:")
                            || squeeze(value).contains("vbscript:")))
                    || (attr == "style"
                        && (squeeze(value).contains("javascript:")
                            || squeeze(value).contains("expression(")));
                if dangerous {
                    found.push(format!("<{name} {attr}={value:?}>"));
                }
            }
        }
        found
    }

    /// The sanitizer against a browser's parser: whatever it leaves of random tags, html5ever
    /// finds nothing dangerous in. Removing an attribute must not let a browser read the rest
    /// differently (the tag's name taking in the next attribute, an attribute without a value
    /// taking ` =javascript:x` as its value, an unquoted value taking in `/title=…`).
    #[test]
    fn browsers_find_nothing_dangerous_in_sanitized_tags() {
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        for case in 0..10_000 {
            let tags: String = (0..1 + rng.below(3))
                .map(|_| random_tag(&mut rng))
                .collect();
            let markdown = match rng.below(3) {
                0 => format!("x {tags} y"),
                1 => format!("<div>\n{tags}\n</div>"),
                _ => tags,
            };
            let out = plain(&markdown);
            let found = dangers(&out);
            assert!(
                found.is_empty(),
                "case {case}: {found:?}\n{markdown:?}\n{out:?}"
            );
        }
    }

    #[test]
    fn the_browser_check_finds_what_it_should() {
        assert!(dangers("<a href=\"javascript:x\">x</a>").len() == 1);
        assert!(dangers("<a href=\"java&#x09;script:x\">x</a>").len() == 1);
        assert!(dangers("<img src=x onerror=y>").len() == 1);
        assert!(dangers("<svg><animate values=\"javascript:x\"/></svg>").len() == 1);
        assert!(dangers("<p style=\"x: expression(1)\">").len() == 1);
        assert!(dangers("<iframe srcdoc=x></iframe>").len() == 2);
        assert!(dangers("<a href=java onclick='x'script:alert(1)>").len() == 1);
        assert!(dangers("<a href=javascript:alert(1)>").len() == 1);
        assert!(dangers("<a href=\"https://ok\" title=\"javascript:x\">x</a>").is_empty());
    }
}

mod robustness {
    use super::*;

    #[test]
    fn empty_and_blank_input() {
        assert_eq!(html(""), "");
        assert_eq!(html(" \n\t\n  \r\n"), "");
        assert_eq!(html("\0"), "\u{fffd}");
        assert_eq!(html("a\0b"), "a\u{fffd}b");
        assert_eq!(html("\u{feff}x"), "x");
    }

    /// Whether XML 1.0 allows every character: the HTML goes to Polarion in a SOAP message.
    fn xml_safe(html: &str) -> bool {
        html.chars()
            .all(|c| matches!(c, '\t' | '\n' | '\r' | ' '..='\u{fffd}' | '\u{10000}'..))
    }

    /// Characters XML does not allow (terminal colors pasted into a code block, ...) become
    /// U+FFFD on every path, raw HTML and highlighted code included; the form feed, which
    /// separates attributes in HTML, becomes a space.
    #[test]
    fn characters_xml_does_not_allow_are_replaced() {
        let r = '\u{fffd}';
        assert_eq!(
            plain("```\n\u{1b}[31mred\u{1b}[0m\n```"),
            format!("{PRE}{r}[31mred{r}[0m</pre>")
        );
        assert_eq!(
            plain("a\u{1}b\u{b}c\u{1b}d\u{fffe}e\u{ffff}"),
            format!("a{r}b{r}c{r}d{r}e{r}")
        );
        assert_eq!(plain("a\u{c}b"), "a b");
        assert_eq!(plain("$a\u{1}$"), formula(&format!("a{r}"), false));
        assert_eq!(plain("`\u{8}`"), format!("{MONO}{r}{END}"));
        assert_eq!(
            plain("<span title=\"\0\u{1}\">x\u{7}</span>"),
            format!("<span title=\"{r}{r}\">x{r}</span>")
        );
        assert_eq!(
            plain("<div\u{c}title=\"\u{1}\">\nx\n</div>"),
            format!("<div title=\"{r}\">\nx\n</div>")
        );
        assert_eq!(
            plain("[a\u{2}](u \"t\u{3}\") ![i\u{4}](i.png)"),
            format!(
                r#"<a href="u" title="t{r}" target="_top" class="descriptionLink">a{r}</a> <img src="i.png" alt="i{r}" style="max-width: 650px;"/>"#
            )
        );
        for markdown in [
            "```rust\nlet a = \"\0\u{1b}\";\n```",
            "```\n\0\n```",
            "# \u{1}",
            "> [!NOTE] \u{2}\n> x",
            "| \u{5} |\n|---|",
        ] {
            let out = html(markdown);
            assert!(xml_safe(&out), "{markdown:?}: {out:?}");
            assert!(out.contains(r), "{markdown:?}: {out:?}");
        }
    }

    /// Raw HTML is scanned in linear time, also when tags or quoted values are not closed or
    /// filtered tags hold more tags.
    #[test]
    fn raw_html_takes_linear_time() {
        let n = 50_000;
        for (unit, tail) in [
            ("<a", ""),
            ("<!x", ""),
            ("</ ", ""),
            ("<b ", "x=\">"),
            ("<script x=\"", "\">"),
        ] {
            for markdown in [
                format!("<div>\n{}{tail}", unit.repeat(n)),
                format!("x {}{tail}", unit.repeat(n)),
            ] {
                let started = std::time::Instant::now();
                let out = html(&markdown);
                let took = started.elapsed();
                assert!(!out.contains("<a") && !out.contains("<b ") && !out.contains("<script"));
                // Quadratic, this took minutes in a debug build; linear, under a second here and
                // about 13 s on a slow CI runner.
                assert!(
                    took < std::time::Duration::from_secs(60),
                    "{unit:?}: {took:?}"
                );
            }
        }
    }

    /// Highlighting a document stops after a second: the rest of the code stays plain. Without
    /// the limit this document takes more than ten seconds.
    #[cfg(feature = "highlight")]
    #[test]
    fn highlighting_a_document_has_a_time_limit() {
        let slow = format!("{}\n", "a".repeat(512)).repeat(100);
        let markdown = format!("```cs\n{slow}```\n\n```rust\nlet x = 1;\n```");
        let started = std::time::Instant::now();
        let out = html(&markdown);
        let took = started.elapsed();
        assert!(took < std::time::Duration::from_secs(4), "{took:?}");
        let pre = |lang: &str| PRE.replace("<pre ", &format!("<pre class=\"language-{lang}\" "));
        assert_eq!(
            out,
            format!(
                "{}{}</pre><br/>{}let x = 1;</pre>",
                pre("cs"),
                slow.strip_suffix('\n').unwrap(),
                pre("rust")
            )
        );
    }

    /// Windows (`\r\n`) and old Mac (`\r`) line endings give the HTML of `\n` (raw HTML aside,
    /// which is passed on as written).
    #[test]
    fn line_endings_do_not_matter() {
        let markdown = "# Title\n\nSome **bold** and `code`\nnext line  \nhard\\\nbreak\n\n\
            Setext\n===\n\n- a\n- b\n  - c\n\n1. x\n\n   y\n\n> quote\n> more\n>\n> - item\n\n\
            > [!NOTE] Title\n> body\n\n:::tip\ntip body\n:::\n\n\
            ```rust\nfn x() {}\n\nlet y = 2;\n```\n\n    indented\n    code\n\n\
            $$\na\n$$ and $b\nc$ and $`d\ne`$\n\n```math\nx\ny\n```\n\n\
            | a | b |\n|:-:|--:|\n| 1 | PROJ-1 |\n\n---\n\nTerm\n: details\n\n\
            Text[^1] ~s~ ^p^ ==h== ++u++ ~~x~~\n\n[^1]: note\n\n\
            [l](u \"t\nx\") ![i](i.png 'a\nb') [r][ref]\n\n[ref]: /r \"r\nt\"\n\n\
            ```mermaid\ngraph TD\n    A-->B\n```\n";
        let unix = html(markdown);
        assert!(!unix.contains('\r'), "{unix}");
        for eol in ["\r\n", "\r"] {
            assert_eq!(html(&markdown.replace('\n', eol)), unix, "{eol:?}");
        }
        let file = mermaid::diagram("graph TD\n    A-->B").file_name;
        let diagrams: DiagramAttachments =
            [(file.clone(), format!("1-{file}"))].into_iter().collect();
        let with_diagram = to_html_with_diagrams(markdown, &ToHtmlOptions::default(), &diagrams);
        assert!(
            with_diagram.contains("workitemimg:1-mermaid-"),
            "{with_diagram}"
        );
        assert_eq!(
            to_html_with_diagrams(
                &markdown.replace('\n', "\r\n"),
                &ToHtmlOptions::default(),
                &diagrams
            ),
            with_diagram
        );
    }

    #[test]
    fn deep_nesting_is_cut_to_text() {
        let html = html(&(">".repeat(10_000) + " deep PROJ-1"));
        assert_eq!(html.matches("<div").count(), 64);
        assert!(
            html.contains("deep PROJ-1") && !html.contains("polarion-rte-link"),
            "{html}"
        );

        let html = super::html(&("*".repeat(10_000) + "a" + &"*".repeat(10_000)));
        assert!(
            html.matches("<span").count() <= 64 && html.contains('a'),
            "{html}"
        );

        let html = super::html(&("^".repeat(10_000) + "a" + &"^".repeat(10_000)));
        assert!(html.matches("<sup>").count() <= 64, "{html}");

        let markdown: String = (0..2000)
            .map(|i| format!("{}- x\n", "  ".repeat(i)))
            .collect();
        let html = super::html(&markdown);
        assert!(
            html.matches("<ul>").count() <= 64 && html.ends_with("</ul>"),
            "{}",
            &html[html.len() - 200..]
        );

        let html = super::html(&"> - ".repeat(5_000));
        assert!(html.matches('<').count() < 300, "{html}");

        let html = super::html(&("[".repeat(10_000) + "a" + &"](u)".repeat(10_000)));
        assert!(html.contains("<a href=\"u\""), "{}", &html[..200]);
    }

    #[test]
    fn long_documents() {
        let markdown = "| a | b |\n|---|---|\n".to_owned() + &"| 1 | PROJ-2 |\n".repeat(20_000);
        let html = html(&markdown);
        assert_eq!(html.matches("<tr>").count(), 20_001);
        assert_eq!(html.matches("data-item-id=\"PROJ-2\"").count(), 20_000);
        let html = super::html(&"PROJ-1 ".repeat(50_000));
        assert_eq!(html.matches("polarion-rte-link").count(), 50_000);
        let html = super::html(&format!("```rust\n{}```", "let x = 1;\n".repeat(5_000)));
        assert!(
            html.ends_with("let x = 1;</pre>") || html.ends_with("</span></pre>"),
            "{}",
            &html[html.len() - 100..]
        );
    }

    #[test]
    fn invalid_looking_html() {
        for markdown in [
            "<",
            ">",
            "</",
            "<>",
            "</>",
            "<<<>>>",
            "<a",
            "<a href=",
            "<a href=\"",
            "<a href='x",
            "<!--",
            "<!-->",
            "<!---->",
            "<!",
            "<?",
            "<![CDATA[",
            "<div <span>>",
            "<div\n\n>",
            "<img src=\"\u{0}\">",
            "<x =y>",
            "<x a=\"1\"b=2/>",
            "</div foo=\"a>b\">",
            "<div>\n\n<",
            "&#",
            "&#x",
            "&#99999999999;",
            "&",
            "<a href=\"x\">\n\n</a>",
        ] {
            let _ = html(markdown);
            let _ = append_html(markdown, markdown);
            let _ = attachment_refs(markdown);
            let _ = text_to_html(markdown);
        }
    }

    /// A deterministic xorshift generator (no new dependency).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[(self.next() % items.len() as u64) as usize]
        }
    }

    const FRAGMENTS: [&str; 66] = [
        "\u{1b}",
        "\u{c}",
        "<",
        ">",
        "</",
        "/>",
        "<!--",
        "-->",
        "<!",
        "<?",
        "\"",
        "'",
        "=",
        " ",
        "\n",
        "\n\n",
        "\t",
        "script",
        "style",
        "iframe",
        "noscript",
        "textarea",
        "title",
        "img",
        "a",
        "div",
        "span",
        "svg",
        "animate",
        "href",
        "src",
        "xlink:href",
        "onerror",
        "onload",
        "on",
        "srcdoc",
        "style=",
        "javascript:",
        "java\tscript:",
        "&#106;",
        "&colon;",
        "&#x3a;",
        "data:text/html,",
        "expression(",
        "*",
        "_",
        "`",
        "[",
        "](",
        ")",
        "- ",
        "1. ",
        "> ",
        "# ",
        "|",
        "---",
        "PROJ-1",
        "\\",
        "ä",
        "😀",
        "\0",
        "~~",
        "==",
        "$",
        ":::note",
        "```",
    ];

    #[test]
    fn random_input_never_yields_script() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for case in 0..3_000 {
            let len = 1 + rng.next() % 40;
            let markdown: String = (0..len).map(|_| rng.pick(&FRAGMENTS)).collect();
            let html = html(&markdown);
            assert!(xml_safe(&html), "case {case}: {markdown:?}: {html:?}");
            let document = scraper::Html::parse_fragment(&html);
            for node in document.tree.nodes() {
                let Some(element) = node.value().as_element() else {
                    continue;
                };
                let name = element.name();
                assert!(
                    ![
                        "script",
                        "style",
                        "iframe",
                        "textarea",
                        "title",
                        "xmp",
                        "noembed",
                        "noframes",
                        "noscript",
                        "plaintext"
                    ]
                    .contains(&name),
                    "case {case}: <{name}> from {markdown:?}: {html}"
                );
                for (attr, value) in element.attrs() {
                    assert!(
                        !attr.to_ascii_lowercase().starts_with("on"),
                        "case {case}: {attr} from {markdown:?}: {html}"
                    );
                    assert_ne!(attr, "srcdoc", "case {case}: from {markdown:?}: {html}");
                    let squeezed: String = value
                        .chars()
                        .filter(|c| !c.is_whitespace() && !c.is_control())
                        .collect::<String>()
                        .to_lowercase();
                    if [
                        "href",
                        "src",
                        "xlink:href",
                        "action",
                        "formaction",
                        "data",
                        "values",
                        "to",
                        "from",
                    ]
                    .contains(&attr)
                    {
                        assert!(
                            !squeezed.starts_with("javascript:")
                                && !squeezed.contains(";javascript:"),
                            "case {case}: {attr}={value:?} from {markdown:?}: {html}"
                        );
                    }
                    if attr == "style" {
                        assert!(
                            !squeezed.contains("javascript:") && !squeezed.contains("expression("),
                            "case {case}: {html}"
                        );
                    }
                }
            }
        }
    }
}

mod documents {
    use super::*;

    /// The HTML with every mermaid block of `markdown` attached as `1-<file name>`.
    fn html_with_diagrams(markdown: &str) -> String {
        let diagrams: DiagramAttachments = mermaid::diagrams(markdown)
            .into_iter()
            .map(|d| (d.file_name.clone(), format!("1-{}", d.file_name)))
            .collect();
        to_html_with_diagrams(markdown, &ToHtmlOptions::default(), &diagrams)
    }

    /// The text of some HTML: tags removed, the basic entities decoded.
    fn text_of(html: &str) -> String {
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
        text.replace("&gt;", ">")
            .replace("&lt;", "<")
            .replace("&amp;", "&")
    }

    const COURIER: &str = "font-family: 'Courier New'";

    /// Polarion's rich text has no paragraphs, headings or semantic emphasis: whole documents
    /// come out as styled spans, breaks and Polarion's own elements.
    #[test]
    fn whole_documents_are_styled_spans_and_breaks() {
        let html = html("Hello world");
        assert_eq!(html, "Hello world");
        let documents = [
            "# Heading\n\nSome **bold** and *italic* text.\n\n[Link](http://example.com)\n\n\
             | H1 | H2 |\n| --- | --- |\n| C1 | C2 |",
            "# Main Title\n\nSome **bold** and *italic* text.\n\n## Section\n\nReference: PROJ-99999\n\n\
             `inline code` here.\n\n```\ncode block line\n```\n\n[Link](http://example.com)\n\nhard\nbreak\n\n\
             | H1 | H2 |\n| --- | --- |\n| C1 | C2 |\n\n### Sub Section\n\nFinal paragraph.",
        ];
        for markdown in documents {
            let html = super::html(markdown);
            for tag in [
                "<p>", "</p>", "<strong>", "<em>", "<h1", "<h2", "<h3", "<br />",
            ] {
                assert!(!html.contains(tag), "{tag}: {html}");
            }
        }
        let html = super::html(documents[1]);
        for expected in [
            "font-size: 22pt",
            "font-size: 18pt",
            "font-size: 14pt",
            "font-weight: bold;\">",
            "font-style: italic",
            COURIER,
            "margin-left: 40px",
            "target=\"_top\"",
            "descriptionLink",
            "polarion-Document-table",
            "data-item-id=\"PROJ-99999\"",
            "hard<br/>break",
            "Final paragraph.",
        ] {
            assert!(html.contains(expected), "{expected}: {html}");
        }
        assert!(!html.ends_with("<br/>"), "{html}");
    }

    /// The diagrams a document lists ([`mermaid::diagrams`]) are what it references once they
    /// are attached; the source travels in the SVG (`mermaid::embed_source`), and without the
    /// attachment it is the code block's text.
    #[test]
    fn diagrams_are_images_of_their_attachments() {
        let markdown = "```mermaid\ngraph TD;\n    A-->B;\n```";
        let file = mermaid::diagram("graph TD;\n    A-->B;").file_name;
        assert_eq!(
            html_with_diagrams(markdown),
            format!(
                r#"<img src="workitemimg:1-{file}" title="{file}" style="max-width: 650px;"/>"#
            )
        );
        let html = html(markdown);
        assert!(
            html.starts_with("<pre class=\"language-mermaid\""),
            "{html}"
        );
        assert!(!html.contains("mermaid.ink"), "{html}");

        let content = "graph LR;\n    A---B;";
        let markdown = format!("```mermaid\n{content}\n```");
        let diagrams = mermaid::diagrams(&markdown);
        assert_eq!(diagrams.len(), 1);
        assert_eq!(diagrams[0].source, content);
        let image = html_with_diagrams(&markdown);
        assert!(
            image.contains(&format!("src=\"workitemimg:1-{}\"", diagrams[0].file_name)),
            "{image}"
        );
        assert!(text_of(&super::html(&markdown)).contains(content));

        // Other code blocks are no diagrams.
        let html = html_with_diagrams("```rust\nlet x = 1;\n```");
        assert!(html.contains(COURIER), "{html}");
        assert!(
            !html.contains("mermaid") && !html.contains("<img"),
            "{html}"
        );
    }

    #[test]
    fn diagrams_and_code_blocks_together() {
        let highlighted = |html: &str| {
            assert!(html.contains("<pre class=\"language-rust\" "), "{html}");
            if cfg!(feature = "highlight") {
                assert!(html.contains("style=\"color:"), "{html}");
            }
        };
        let html = html_with_diagrams(
            "```mermaid\ngraph TD;\n    A-->B;\n```\n\n```rust\nlet x = 1;\n```",
        );
        assert!(
            html.starts_with("<img src=\"workitemimg:1-mermaid-"),
            "{html}"
        );
        assert!(html.contains("style=\"max-width: 650px;\""), "{html}");
        highlighted(&html);
        assert!(html.contains("let"), "{html}");

        let html = html_with_diagrams(
            "```mermaid\ngraph LR;\n    X---Y;\n```\n\n```\nplain code block\n```",
        );
        assert!(
            html.starts_with("<img src=\"workitemimg:1-mermaid-"),
            "{html}"
        );
        assert!(
            html.contains("plain code block") && html.contains(COURIER),
            "{html}"
        );

        let markdown = "# Overview\n\n```mermaid\ngraph TD;\n    A-->B;\n```\n\n## Implementation\n\n\
                        ```rust\npub struct Foo {}\n```\n\n## Config\n\n```\nkey=value\n```";
        let html = html_with_diagrams(markdown);
        assert!(html.contains("<img src=\"workitemimg:1-mermaid-"), "{html}");
        highlighted(&html);
        for word in [
            "pub",
            "struct",
            "Foo",
            "key=value",
            COURIER,
            "Overview",
            "Implementation",
        ] {
            assert!(html.contains(word), "{word}: {html}");
        }
    }
}

mod helpers {
    use super::*;

    #[test]
    fn appending_converted_markdown() {
        assert_eq!(append_html("", &html("**b**")), format!("{BOLD}b{END}"));
        assert_eq!(
            append_html("<b>existing</b>", "<i>new</i>"),
            "<b>existing</b><br/><br/><i>new</i>"
        );
        assert_eq!(
            append_html("<b>existing</b>", &html("**bold**")),
            format!("<b>existing</b><br/><br/>{BOLD}bold{END}")
        );
        // Blank rich text is replaced.
        assert_eq!(
            append_html(" <br/>&nbsp;", "<b>content</b>"),
            "<b>content</b>"
        );
        assert_eq!(
            append_html("<b>old</b>", &html("new")),
            "<b>old</b><br/><br/>new"
        );
        assert_eq!(
            append_html("<b>old</b><br/><br/>", &html("new")),
            "<b>old</b><br/><br/>new"
        );
        assert_eq!(
            append_html(&html("> q"), &html("new")),
            format!("{QUOTE}q</div><br/>new")
        );
        assert_eq!(append_html(&html("---"), &html("new")), "<hr/><br/>new");
        assert_eq!(
            append_html(&html("- a"), &html("b")),
            "<ul><li>a</li></ul><br/>b"
        );
        assert_eq!(append_html(&html("a"), &html("---")), "a<br/><br/><hr/>");
        // Markdown without content changes nothing.
        assert_eq!(append_html("<b>old</b>", &html("")), "<b>old</b>");
        assert_eq!(
            append_html("<b>old</b><br/>", &html("<!-- note -->")),
            "<b>old</b><br/>"
        );
    }

    #[test]
    fn plain_text_descriptions() {
        assert_eq!(
            text_to_html("line 1\nline <2> & PROJ-1\r\nend"),
            "line 1<br/>line &lt;2&gt; &amp; PROJ-1<br/>end"
        );
        assert_eq!(text_to_html("\u{a0}"), "&nbsp;");
    }
}
