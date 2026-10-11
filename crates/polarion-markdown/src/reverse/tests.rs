//! Quick checks of the required behaviors of the conversion to Markdown, and seeded random checks
//! of its whole-document properties.

use base64::Engine as _;

use super::*;
use crate::{
    DiagramAttachments, ToHtmlOptions, WorkItemIds, mermaid, styles, to_html, to_html_with_diagrams,
};

fn keep() -> ToMarkdownOptions {
    ToMarkdownOptions::default()
}

fn dropping() -> ToMarkdownOptions {
    ToMarkdownOptions {
        raw_html: RawHtml::Drop,
        ..ToMarkdownOptions::default()
    }
}

fn forward(options: &ToMarkdownOptions) -> ToHtmlOptions {
    ToHtmlOptions {
        work_items: options.work_items.clone(),
        highlight: true,
    }
}

/// Converting the Markdown back and forth gives it again.
fn assert_fixpoint(markdown: &str, options: &ToMarkdownOptions) {
    let html = to_html(markdown, &forward(options));
    let again = to_markdown(&html, options, &DiagramSources::new());
    assert_eq!(again.invalid, 0, "{markdown:?}");
    assert_eq!(
        again.markdown, markdown,
        "not a fixpoint; the HTML was {html:?}"
    );
}

/// The Markdown of some HTML: every built tree valid, no fallback, a fixpoint.
fn md_with(html: &str, options: &ToMarkdownOptions) -> String {
    let report = to_markdown(html, options, &DiagramSources::new());
    assert_eq!(report.invalid, 0, "{html}");
    assert_eq!(report.fallbacks, 0, "{html} -> {:?}", report.markdown);
    assert_fixpoint(&report.markdown, options);
    report.markdown
}

fn md(html: &str) -> String {
    md_with(html, &keep())
}

fn md_drop(html: &str) -> String {
    md_with(html, &dropping())
}

fn html(markdown: &str) -> String {
    to_html(markdown, &ToHtmlOptions::default())
}

/// Canonical Markdown comes back unchanged, and so does its HTML.
fn assert_canonical(markdown: &str) {
    let first = html(markdown);
    let back = md(&first);
    assert_eq!(back, markdown, "{first}");
    assert_eq!(html(&back), first);
}

const LINK_SPAN: &str = r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-16026" data-option-id="long"></span>"#;

mod headings_back {
    use super::*;

    fn sized(style: &str) -> String {
        format!("<span style=\"{style}\">Text</span>")
    }

    #[test]
    fn bold_sizes_give_heading_levels() {
        for (size, level) in [
            ("22", 1),
            ("18", 2),
            ("14", 3),
            ("12", 4),
            ("11", 5),
            ("10.5", 6),
            ("20", 2),
            ("16", 3),
            ("13", 4),
            ("14.5", 3),
        ] {
            let input = sized(&format!("font-weight: bold; font-size: {size}pt;"));
            assert_eq!(
                md(&input),
                format!("{} Text", "#".repeat(level)),
                "{size}pt"
            );
        }
        assert!(!md(&sized("font-weight: bold; font-size: 22pt;")).starts_with("## "));
    }

    #[test]
    fn large_text_on_a_whole_line_is_a_heading() {
        assert_eq!(md(&sized("font-size: 18pt;")), "## Text");
        assert_eq!(md(&sized("font-size: 14pt;")), "### Text");
    }

    #[test]
    fn body_sizes_stay_text() {
        let line = "Text <span style=\"font-size: 12pt;\">12</span>";
        let kept = md(line);
        assert!(
            kept.contains("<span style=\"font-size: 12pt;\">12</span>"),
            "{kept}"
        );
        assert!(kept.lines().all(|l| !l.starts_with('#')), "{kept}");
        assert_eq!(md_drop(line), "Text 12");
        for size in ["11", "12"] {
            let whole = sized(&format!("font-size: {size}pt;"));
            assert_eq!(
                md(&whole),
                format!("<span style=\"font-size: {size}pt;\">Text</span>")
            );
        }
        assert_eq!(md(&sized("font-size: 10pt;")), "Text");
        assert_eq!(
            md(&sized("font-weight: bold; font-size: 10pt;")),
            "**Text**"
        );
        assert_eq!(md(&sized("font-weight: bold;")), "**Text**");
    }

    #[test]
    fn empty_heading_text_is_an_empty_heading() {
        assert_eq!(
            md("<span style=\"font-weight: bold; font-size: 22pt;\"></span>"),
            "#"
        );
        assert_eq!(
            md("<span style=\"font-weight: bold; font-size: 22pt;\"> </span>"),
            "#"
        );
    }

    #[test]
    fn headings_round_trip() {
        for level in 1..=6 {
            assert_canonical(&format!("{} H{level}", "#".repeat(level)));
        }
        assert_eq!(md(&html("# Title")), "# Title");
        assert_eq!(md(&html("### Sub")), "### Sub");
    }

    #[test]
    fn the_combined_document() {
        let input = [
            r#"<span style="font-weight: bold; font-size: 22pt; line-height: 1.2;">Main Title</span><br/><br/>"#,
            r#"Some <span style="font-weight: bold;">bold</span> and <span style="font-style: italic;">italic</span> text.<br/><br/>"#,
            r#"<span style="font-weight: bold; font-size: 18pt; line-height: 1.2;">Section</span><br/><br/>"#,
            r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-99999" data-option-id="long"></span><br/><br/>"#,
            r#"<span style="font-family: 'Courier New', Courier, monospace;">inline code</span><br/><br/>"#,
            r#"<a href="http://example.com" target="_top" class="descriptionLink">Link</a><br/><br/>"#,
            r#"Font Size <span style="font-size: 12pt;line-height: 1.5;">12</span><br/><br/>"#,
            r#"<span style="font-size: 18pt;">Whole Line Heading</span><br/><br/>"#,
            r#"<span class="polarion-rte-formula" data-source="\sqrt{x}"></span><br/><br/>"#,
            r#"<span data-source="\int_{0}^{1}" data-inline="false" class="polarion-rte-formula"></span><br/><br/>"#,
            r#"<table id="polarion_wiki macro name=table" class="polarion-Document-table" style="width: 100%;border: 1px solid #CCCCCC;border-collapse: collapse;">"#,
            r#"  <tbody>"#,
            r#"    <tr>"#,
            r#"      <th style="font-weight: bold;background-color: #F0F0F0;text-align: left;border: 1px solid #CCCCCC;padding: 5px;">H1</th>"#,
            r#"      <th style="font-weight: bold;background-color: #F0F0F0;text-align: left;border: 1px solid #CCCCCC;padding: 5px;">H2</th>"#,
            r#"    </tr>"#,
            r#"    <tr>"#,
            r#"      <td style="text-align: left;border: 1px solid #CCCCCC;padding: 5px;">C1</td>"#,
            r#"      <td style="text-align: left;border: 1px solid #CCCCCC;padding: 5px;">C2</td>"#,
            r#"    </tr>"#,
            r#"  </tbody>"#,
            r#"</table>"#,
            r#"<span style="font-weight: bold; font-size: 14pt; line-height: 1.2;">Sub</span><br/><br/>"#,
            r#"Final paragraph."#,
        ]
        .join("\n");
        let out = md(&input);
        for expected in [
            "# Main Title",
            "## Section",
            "### Sub",
            "## Whole Line Heading",
            "**bold**",
            "*italic*",
            "`inline code`",
            "[Link](http://example.com)",
            "PROJ-99999",
            "$\\sqrt{x}$",
            "$$\\int_{0}^{1}$$",
            "| H1 | H2 |",
            "| C1 | C2 |",
            "Final paragraph.",
        ] {
            assert!(out.contains(expected), "{expected}: {out}");
        }
        assert!(
            out.lines()
                .filter(|l| l.contains("Font Size"))
                .all(|l| !l.starts_with('#')),
            "{out}"
        );
        assert!(!out.contains("PROJ\\-99999"), "{out}");
    }
}

mod inline_formatting_back {
    use super::*;

    #[test]
    fn styles_markdown_expresses() {
        let cases = [
            (
                r#"<span style="font-weight: bold;">Bold</span>"#,
                "**Bold**",
            ),
            (
                r#"<span style="font-style: italic;">Italic</span>"#,
                "*Italic*",
            ),
            (
                r#"<span style="font-family: 'Courier New', Courier, monospace;">code</span>"#,
                "`code`",
            ),
            (
                r#"<span style="text-decoration: line-through;">x</span>"#,
                "~~x~~",
            ),
            (
                r#"<span style="text-decoration: underline;">x</span>"#,
                "++x++",
            ),
            (
                r#"<span style="background-color: #FFFF00;">x</span>"#,
                "==x==",
            ),
            (r#"<span style="background-color: #FF0;">x</span>"#, "==x=="),
            (
                r#"<span style="background-color: yellow;">x</span>"#,
                "==x==",
            ),
            (
                r#"<span style="background-color: rgb(255, 255, 0);">x</span>"#,
                "==x==",
            ),
            ("X<sup>2</sup>", "X^2^"),
            ("X<sub>2</sub>", "X~2~"),
        ];
        for (input, expected) in cases {
            assert_eq!(md(input), expected, "{input}");
        }
    }

    #[test]
    fn formatting_elements() {
        let cases = [
            ("<b>x</b>", "**x**"),
            ("<strong>x</strong>", "**x**"),
            ("<i>x</i>", "*x*"),
            ("<em>x</em>", "*x*"),
            ("<u>x</u>", "++x++"),
            ("<s>x</s>", "~~x~~"),
            ("<del>x</del>", "~~x~~"),
            ("<mark>x</mark>", "==x=="),
            ("<code>x</code>", "`x`"),
        ];
        for (input, expected) in cases {
            assert_eq!(md(input), expected, "{input}");
        }
    }

    #[test]
    fn normal_weight_is_plain() {
        assert_eq!(
            md(r#"<span style="font-weight: normal;">text</span>"#),
            "text"
        );
    }

    #[test]
    fn colors_and_fonts_stay_raw_in_keep_mode() {
        let red = r#"<span style="color: #FF0000;">text</span>"#;
        assert_eq!(md(red), red);
        assert_eq!(md_drop(red), "text");
        let arial = r#"<span style="font-family: Arial, Helvetica, sans-serif;">text</span>"#;
        assert_eq!(md(arial), arial);
        assert_eq!(md_drop(arial), "text");
        for out in [md(red), md(arial), md_drop(red), md_drop(arial)] {
            assert!(!out.contains("**") && !out.contains('`'), "{out}");
        }
    }

    #[test]
    fn links() {
        assert_eq!(
            md(r#"<a href="http://example.com" target="_top" class="descriptionLink">Link</a>"#),
            "[Link](http://example.com)"
        );
        assert_eq!(
            md(
                r#"<a href="https://polarion.example.com/polarion/#/project/P/workitem?id=PROJ-7">PROJ-7</a>"#
            ),
            "PROJ-7"
        );
        // Another label stays a link.
        assert_eq!(
            md(
                r#"<a href="https://polarion.example.com/polarion/#/project/P/workitem?id=PROJ-7">the item</a>"#
            ),
            "[the item](https://polarion.example.com/polarion/#/project/P/workitem?id=PROJ-7)"
        );
    }

    #[test]
    fn emphasis_next_to_punctuation_is_html() {
        let out = md(r#"<span style="font-weight: bold;">foo.</span>bar"#);
        assert_eq!(out, r#"<span style="font-weight: bold;">foo.</span>bar"#);
        let out = md(r#"<span style="text-decoration: line-through;">a<sub>b</sub></span>"#);
        assert_eq!(
            html(&out),
            r#"<span style="text-decoration: line-through;">a<sub>b</sub></span>"#
        );
    }

    #[test]
    fn superscript_in_link_text_is_html() {
        let out = md(r#"<a href="u">x<sup>2</sup></a>"#);
        assert_eq!(out, "[x<sup>2</sup>](u)");
    }

    #[test]
    fn whitespace_is_hoisted_out_of_formatting() {
        assert_eq!(md("a<b> b </b>c"), "a **b** c");
        assert_eq!(md("<i>a</i> <i>b</i>"), "*a* *b*");
        assert_eq!(md("<b>a<br/>b</b>"), "**a\\\nb**");
    }
}

mod polarion_spans_back {
    use super::*;

    #[test]
    fn long_work_item_links_become_the_bare_id() {
        assert_eq!(md(LINK_SPAN), "PROJ-16026");
        assert_eq!(html(&md(LINK_SPAN)), LINK_SPAN);
        assert_eq!(md(&format!("See {LINK_SPAN}.")), "See PROJ-16026.");
    }

    #[test]
    fn links_the_setting_would_not_link_again() {
        for work_items in [WorkItemIds::Off, WorkItemIds::Prefixes(vec!["MP".into()])] {
            let kept = ToMarkdownOptions {
                work_items: work_items.clone(),
                ..keep()
            };
            assert_eq!(md_with(LINK_SPAN, &kept), LINK_SPAN);
            let dropped = ToMarkdownOptions {
                work_items,
                ..dropping()
            };
            assert_eq!(md_with(LINK_SPAN, &dropped), "PROJ\\-16026");
        }
        let mp = ToMarkdownOptions {
            work_items: WorkItemIds::Prefixes(vec!["PROJ".into()]),
            ..keep()
        };
        assert_eq!(md_with(LINK_SPAN, &mp), "PROJ-16026");
        // Glued to a letter the id would not be linked: the span stays.
        assert_eq!(md(&format!("x{LINK_SPAN}")), format!("x{LINK_SPAN}"));
    }

    #[test]
    fn other_link_variants_stay_raw() {
        let variants = [
            r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-option-id="short"></span>"#,
            r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-option-id="custom" data-custom-label="My label"></span>"#,
            r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-scope="OTHER" data-item-id="PROJ-1" data-option-id="long"></span>"#,
            r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-revision="1234" data-option-id="long"></span>"#,
            r#"<span class="polarion-rte-link" data-type="crossReference" data-item-id="PROJ-1" data-option-id="long"></span>"#,
        ];
        for span in variants {
            let out = md(span);
            assert_eq!(out, span);
            assert_eq!(html(&out), span, "byte-identical after the round trip");
            assert_eq!(md_drop(span), "PROJ\\-1", "{span}");
        }
        // Links to other things: the raw span; in drop mode their name.
        let others = [
            (
                r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-option-id="default"></span>"#,
                "PROJ\\-1",
            ),
            (
                r#"<span class="polarion-rte-link" data-type="richPage" data-item-name="Home" data-space-name="_default" data-option-id="long"></span>"#,
                "Home",
            ),
            (
                r#"<span class="polarion-rte-link" data-type="polarion" data-url="https://x" data-custom-label="Site" data-option-id="custom"></span>"#,
                "Site",
            ),
        ];
        for (span, dropped) in others {
            assert_eq!(md(span), span);
            assert_eq!(md_drop(span), dropped, "{span}");
        }
    }

    #[test]
    fn formula_sources_with_markup_text() {
        // `data-source` holding `<br/>` text keeps it.
        let input = r#"<span class="polarion-rte-formula" data-source="a<br/>b"></span>"#;
        let out = md(input);
        assert_eq!(out, "$a<br/>b$");
        assert_eq!(
            html(&out),
            r#"<span data-source="a&lt;br/&gt;b" class="polarion-rte-formula"></span>"#
        );
        let display = r#"<span data-source="a<br>ä" data-inline="false" class="polarion-rte-formula"></span>"#;
        assert_eq!(md(display), "$$a<br>ä$$");
    }

    #[test]
    fn formulas() {
        assert_eq!(
            md(r#"<span class="polarion-rte-formula" data-source="\sqrt{x}"></span>"#),
            "$\\sqrt{x}$"
        );
        assert_eq!(
            md(
                r#"<span data-source="\sqrt{Block}" data-inline="false" class="polarion-rte-formula"></span>"#
            ),
            "$$\\sqrt{Block}$$"
        );
        assert_eq!(
            md(r#"<span class="polarion-rte-formula" data-source="a &amp; b"></span>"#),
            "$a & b$"
        );
        // Code math where `$…$` cannot hold the formula.
        assert_eq!(
            md(r#"<span class="polarion-rte-formula" data-source="x"></span>2"#),
            "$`x`$2"
        );
        assert_eq!(
            md(r#"<span class="polarion-rte-formula" data-source=" x"></span>"#),
            "$` x`$"
        );
        assert_eq!(
            md(r#"<span class="polarion-rte-formula" data-source="a$b"></span>"#),
            "$`a$b`$"
        );
        // A multi-line display formula alone in its paragraph: a math block.
        assert_eq!(
            md(
                "<span data-source=\"a \\\\\nb\" data-inline=\"false\" class=\"polarion-rte-formula\"></span>"
            ),
            "```math\na \\\\\nb\n```"
        );
    }

    #[test]
    fn formulas_markdown_cannot_hold_stay_raw() {
        // A line break in an inline formula; a multi-line display formula among text.
        let inline = "<span class=\"polarion-rte-formula\" data-source=\"a\nb\"></span>";
        let out = md(inline);
        assert_eq!(
            out,
            "<span class=\"polarion-rte-formula\" data-source=\"a&#10;b\"></span>"
        );
        assert_eq!(md_drop(inline), out);
        let display = "x <span data-source=\"a\nb\" data-inline=\"false\" class=\"polarion-rte-formula\"></span>";
        assert!(md(display).contains("data-source=\"a&#10;b\""));
    }
}

mod code_back {
    use super::*;

    const MONO: &str = "font-family: 'Courier New', Courier, monospace;";

    #[test]
    fn indented_monospace() {
        let one = format!(
            "<div style=\"margin-left: 40px;\"><span style=\"{MONO}\">let x = 1;</span></div>"
        );
        assert_eq!(md(&one), "> `let x = 1;`");
        let lines = format!(
            "<div style=\"margin-left: 40px;\"><span style=\"{MONO}\">line1<br/>line2<br/>line3</span></div>"
        );
        assert_eq!(md(&lines), "```\nline1\nline2\nline3\n```");
    }

    #[test]
    fn pre_blocks() {
        let out = md("<pre><code>let x = 1;</code></pre>");
        assert_eq!(out, "```\nlet x = 1;\n```");
        assert_eq!(
            md("<pre><code class=\"language-rust\">let x = 1;</code></pre>"),
            "```rust\nlet x = 1;\n```"
        );
        let highlighted = html("```rust\nlet x = 1;\n```");
        assert_eq!(md(&highlighted), "```rust\nlet x = 1;\n```");
        let spans = md(
            "<pre><code><span style=\"color:Blue;\">let</span> x = <span style=\"color:DarkCyan;\">1</span>;</code></pre>",
        );
        assert_eq!(spans, "```\nlet x = 1;\n```");
        let wrapped = md(
            "<div style=\"color:#000000;background-color:#FFFFFF;\"><pre><span style=\"color:#0000FF;\">let</span> x = <span>1</span>;</pre></div>",
        );
        assert_eq!(wrapped, "```\nlet x = 1;\n```");
        assert_eq!(md("<pre>a<br>b<br/>c</pre>"), "```\na\nb\nc\n```");
        assert_eq!(
            md("<pre class=\"language-mermaid\">graph TD\n    A--&gt;B</pre>"),
            "```mermaid\ngraph TD\n    A-->B\n```"
        );
    }
}

mod diagrams_and_images_back {
    use super::*;

    fn sources(id: &str, source: &str) -> DiagramSources {
        [(id.to_owned(), source.to_owned())].into_iter().collect()
    }

    #[test]
    fn diagram_attachments_with_their_source_become_fences() {
        let source = "graph TD\n    A-->B";
        let file = mermaid::diagram(source).file_name;
        let id = format!("1-{file}");
        let image =
            format!(r#"<img src="workitemimg:{id}" title="{file}" style="max-width: 650px;"/>"#);
        let report = to_markdown(&image, &keep(), &sources(&id, source));
        assert_eq!(report.markdown, format!("```mermaid\n{source}\n```"));
        assert_eq!((report.fallbacks, report.invalid), (0, 0));
        // A `/wi-attachment/` URL of the attachment too.
        let url = format!(r#"<img src="https://x/polarion/wi-attachment/P/P-1/{id}"/>"#);
        assert_eq!(
            to_markdown(&url, &keep(), &sources(&id, source)).markdown,
            format!("```mermaid\n{source}\n```")
        );
        // Without the source it stays an image.
        assert_eq!(md(&image), format!("![](workitemimg:{id} \"{file}\")"));
    }

    #[test]
    fn mermaid_ink_images_become_fences() {
        let source = "graph TD\n    A-->B";
        let payload = base64::engine::general_purpose::STANDARD.encode(source);
        let image =
            format!(r#"<img src="https://mermaid.ink/img/{payload}" style="max-width: 650px;" />"#);
        assert_eq!(md(&image), format!("```mermaid\n{source}\n```"));
    }

    #[test]
    fn images() {
        let out = md(r#"<img src="https://example.com/image.png" alt="test" />"#);
        assert_eq!(out, "![test](https://example.com/image.png)");
        assert_eq!(
            md(
                r#"<img src="workitemimg:1-a.png" title="a.png" style="max-width: 650px;" data-lastmodified="2025-01-01T08:00:00.000+01:00"/>"#
            ),
            "![](workitemimg:1-a.png \"a.png\")"
        );
        let sized = r#"<img src="workitemimg:2-c.jpg" style="width: 100px;height: 74px;">"#;
        assert_eq!(
            md(sized),
            r#"<img src="workitemimg:2-c.jpg" style="width: 100px;height: 74px;"/>"#
        );
        assert_eq!(md_drop(sized), "![](workitemimg:2-c.jpg)");
    }

    #[test]
    fn an_image_and_its_caption_line() {
        let out = md(r#"<img src="a.png" alt="i"/><br/>Figure 1"#);
        assert_eq!(out, "![i](a.png)\\\nFigure 1");
        let raw = r#"<img src="a.png" style="width: 10px;"><br/>Figure 1"#;
        let out = md(raw);
        assert!(out.ends_with("\\\nFigure 1"), "{out}");
        assert_eq!(
            html(&out),
            r#"<img src="a.png" style="width: 10px;"/><br/>Figure 1"#
        );
    }

    #[test]
    fn diagrams_round_trip() {
        let markdown = "```mermaid\ngraph TD;\n    A-->B;\n```";
        let diagram = &mermaid::diagrams(markdown)[0];
        let id = format!("1-{}", diagram.file_name);
        let attachments: DiagramAttachments = [(diagram.file_name.clone(), id.clone())]
            .into_iter()
            .collect();
        let html = to_html_with_diagrams(markdown, &ToHtmlOptions::default(), &attachments);
        let back = to_markdown(&html, &keep(), &sources(&id, &diagram.source));
        assert_eq!(back.markdown, markdown);
    }
}

mod blocks_back {
    use super::*;

    #[test]
    fn nested_empty_bullets_stay_a_list() {
        let markdown = md("<ul><li><ul><li><ul><li></li></ul></li></ul></li></ul><p>x</p>");
        assert!(!html(&markdown).contains("<hr"), "{markdown:?}");
        assert!(markdown.starts_with("- <!-- -->\n"), "{markdown:?}");
        assert!(md("<ul><li><ul><li><ul><li>a</li></ul></li></ul></li></ul>").starts_with("- "));
    }

    #[test]
    fn raw_html_is_what_its_reparse_gives() {
        // The parser nests a link in a link here; written as it is, the raw HTML would read back
        // differently.
        let input = r#"<table><tr><td><a href="u"><p><sub><a href="javascript:x"><b>t</b></a></sub></p></a></td></tr></table>"#;
        let markdown = md(input);
        assert!(markdown.starts_with("<table>"), "{markdown:?}");
    }

    #[test]
    fn formatting_around_a_table_reaches_its_cells() {
        // A size there is no heading size: it stays raw.
        let input = "<span style=\"font-weight: bold; font-size: 22pt\"><table><tbody><tr><th><u></u>a</th></tr></tbody></table></span>";
        assert_eq!(
            md(input),
            "| <span style=\"font-size: 22pt;\">**a**</span> |
| --- |"
        );
        assert_eq!(
            md_drop("<i><table><tr><th>h</th></tr><tr><td>c</td></tr></table></i>"),
            "| *h* |\n| --- |\n| *c* |"
        );
    }

    #[test]
    fn filtered_elements_in_a_table_leave_it_standard() {
        assert_eq!(
            md("<table><script>x</script><tr><th>a</th><style>p{}</style></tr></table>"),
            "| a |
| --- |"
        );
    }

    #[test]
    fn paragraphs_and_breaks() {
        assert_eq!(md("Para1<br/><br/>Para2"), "Para1\n\nPara2");
        assert_eq!(md("a<br/>b"), "a\\\nb");
        assert_eq!(md("a<br><br><br>b"), "a\n\n<br/>b");
        assert_eq!(md_drop("a<br><br><br>b"), "a\n\nb");
    }

    #[test]
    fn lists() {
        assert_eq!(
            md("<ol><li>Ordered</li><li>List</li></ol>"),
            "1. Ordered\n2. List"
        );
        assert_eq!(md("<ul><li>a</li><li>b</li></ul>"), "- a\n- b");
        assert_eq!(
            md("<ol start=\"3\"><li>a</li><li>b</li></ol>"),
            "3. a\n4. b"
        );
        assert_eq!(
            md("<ul><li>☐ open</li><li>☑ done</li></ul>"),
            "- [ ] open\n- [x] done"
        );
        assert_eq!(md("<ul><li>a<ul><li>b</li></ul></li></ul>"), "- a\n  - b");
        // Several blocks in an item: a loose list.
        assert_eq!(
            md("<ul><li>a<br/><br/>b</li><li>c</li></ul>"),
            "- a\n  \n  b\n\n- c"
        );
        // Task lists as Markdown converters write them.
        assert_eq!(
            md(
                "<ul><li><input type=\"checkbox\" disabled> open</li><li><input type=\"checkbox\" checked disabled> done</li></ul>"
            ),
            "- [ ] open\n- [x] done"
        );
        // A task item starting with another block keeps its marker as text.
        assert_eq!(
            md("<ul><li>☑ <pre>x</pre></li></ul>"),
            "- ☑\n  \n  ```\n  x\n  ```"
        );
    }

    #[test]
    fn quotes() {
        assert_eq!(
            md("<div style=\"margin-left: 40px;\">Indented<br/> </div>"),
            "> Indented"
        );
        assert_eq!(md("<div style=\"margin-left: 80px;\">x</div>"), "> > x");
    }

    #[test]
    fn alerts() {
        for name in ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"] {
            let markdown = format!("> [!{name}]\n> body");
            assert_canonical(&markdown);
        }
        assert_canonical("> [!WARNING] Heads up\n> a");
    }

    #[test]
    fn styled_divs() {
        let center = "<div style=\"text-align: center;\">Align Center<br/> </div>";
        let out = md(center);
        assert_eq!(
            out,
            "<div style=\"text-align: center;\">\n\nAlign Center\n\n</div>"
        );
        assert_eq!(
            html(&out),
            "<div style=\"text-align: center;\">Align Center</div>"
        );
        assert_eq!(md_drop(center), "Align Center");
    }

    #[test]
    fn rules() {
        assert_eq!(md("a<br/><br/><hr/><br/>b"), "a\n\n-----\n\nb");
    }

    const TH: &str = "font-weight: bold;background-color: #F0F0F0;text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;padding: 5px;";
    const TD: &str =
        "text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;padding: 5px;";

    #[test]
    fn standard_tables_are_gfm() {
        let table = format!(
            "<table id=\"polarion_wiki macro name=table\" class=\"polarion-Document-table\" style=\"width: 100%;max-width: 1280px;margin-left: 0px;margin-right: auto;border: 1px solid #CCCCCC;empty-cells: show;border-collapse: collapse;max-width: 1280.0px;\"><tbody><tr><th style=\"{TH}\">a</th><th style=\"{}\">b</th></tr><tr><td style=\"{TD}\">1<br/>2</td><td style=\"{}\">x</td></tr></tbody></table>",
            TH.replace("left", "center"),
            TD.replace("left", "center")
        );
        assert_eq!(md(&table), "| a | b |\n| --- | :-: |\n| 1<br/>2 | x |");
        assert_canonical("| a | b |\n| --- | --: |\n| 1 | **x** |");
    }

    #[test]
    fn other_tables_stay_raw_in_keep_mode() {
        let merged = format!(
            "<table><tbody>\n<tr><th style=\"{TH}\">a</th><th style=\"{TH}\">b</th></tr>\n\n<tr><td colspan=\"2\" style=\"{TD}\">1</td></tr></tbody></table>"
        );
        let out = md(&merged);
        assert!(out.starts_with("<table>") && !out.contains("\n\n"), "{out}");
        assert_eq!(md_drop(&merged), "| a | b |\n| --- | --- |\n| 1 |  |");
        let blocks =
            "<table><tr><th>a</th></tr><tr><td><ul><li>x</li><li>y</li></ul></td></tr></table>";
        assert!(md(blocks).starts_with("<table>"));
        assert_eq!(md_drop(blocks), "| a |\n| --- |\n| x<br/>y |");
        let no_header = "<table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>";
        assert!(md(no_header).starts_with("<table>"));
        assert_eq!(md_drop(no_header), "| a | b |\n| --- | --- |\n| c | d |");
    }

    #[test]
    fn caption_paragraphs_stay_raw() {
        let caption = "<p class=\"polarion-rte-caption-paragraph\">\n  Figure <span data-sequence=\"Figure\" class=\"polarion-rte-caption\">#</span> caption text\n</p>";
        let out = md(caption);
        assert!(
            out.starts_with("<p class=\"polarion-rte-caption-paragraph\">"),
            "{out}"
        );
        assert_eq!(md(&html(&out)), out);
        assert_eq!(md_drop(caption), "Figure \\# caption text");
    }

    #[test]
    fn filtered_elements_and_comments_are_dropped() {
        for wrapped in [
            "<script>alert(1)</script>",
            "<style>p {}</style>",
            "<iframe src=\"x\"></iframe>",
            "<object data=\"x\"></object>",
            "<embed src=\"x\">",
            "<!-- comment -->",
            "<noscript>n</noscript>",
            "<textarea>t</textarea>",
            "<title>t</title>",
        ] {
            let input = format!("a{wrapped}b");
            assert_eq!(md(&input), "ab", "{wrapped}");
            assert_eq!(md_drop(&input), "ab", "{wrapped}");
        }
    }

    #[test]
    fn unknown_elements_are_transparent() {
        assert_eq!(
            md("<abbr title=\"t\">a</abbr> <custom-x>b</custom-x>"),
            "a b"
        );
        // Block elements without margins end lines, those with margins paragraphs.
        assert_eq!(md("<section>a</section><section>b</section>"), "a\\\nb");
        assert_eq!(md("<p>a</p><p>b</p>"), "a\n\nb");
    }
}

mod whole_documents {
    use super::*;

    #[test]
    fn empty_and_blank_html() {
        for input in ["", " ", "\n\t", "<br/>", "<!-- c -->", "<span></span>"] {
            let report = to_markdown(input, &keep(), &DiagramSources::new());
            assert_eq!(report.markdown, "", "{input:?}");
        }
    }

    #[test]
    fn hostile_input_does_not_break_it() {
        let inputs = [
            "<div>".repeat(10_000),
            "<span style=\"font-weight: bold;\">".repeat(10_000) + "x",
            "<ul><li>".repeat(5_000) + "deep",
            "<div style=\"margin-left: 40px;\">".repeat(3_000) + "q",
            "<table><tr><td>".repeat(500),
            "<b><i><u>".repeat(3_000) + "x",
            "a\0b\u{1}c\u{b}d\u{fffe}".to_owned(),
            "&#0; &#xD800; &bogus; &amp &#99999999999;".to_owned(),
            "<a href=\"x\"><a href=\"y\">z".to_owned(),
            "<img src=\"javascript:alert(1)\" alt=\"a\">".to_owned(),
            "<p><table><tr><td>x</p></td></tr></table>".to_owned(),
        ];
        for input in inputs {
            for options in [keep(), dropping()] {
                let report = to_markdown(&input, &options, &DiagramSources::new());
                assert_eq!(report.invalid, 0, "{}", &input[..input.len().min(80)]);
                assert_fixpoint(&report.markdown, &options);
                assert!(report.markdown.chars().all(crate::html::xml_allows));
            }
        }
    }

    #[test]
    fn plain_text_never_turns_into_links_or_formatting() {
        let texts = [
            "See PROJ-123 for details",
            "*not* _emphasis_ **x** ~~y~~ ==z== ++u++ ^s^ ~t~",
            "$x$ and $$y$$ and `code` and <b>tag</b> & &amp; &#65;",
            "www.example.com https://example.com a@b.cd",
            "| a | b |",
            "# not a heading",
            "- not a list",
            "1. not a list",
            "> not a quote",
            "[link](u) ![img](i) [^1] \\escaped\\",
            "MY_PROJ-12 UTF-8 (PROJ-1) PROJ-1. PROJ-1.5 A-1",
            ":::note",
            "``` fence",
            "---",
            "a\u{a0}b ä 😀",
        ];
        for text in texts {
            let input = crate::html::escape_text(text).into_owned();
            let out = md(&input);
            assert_eq!(html(&out), input, "{text} -> {out}");
            let off = ToHtmlOptions {
                work_items: WorkItemIds::Off,
                ..ToHtmlOptions::default()
            };
            assert_eq!(to_html(&out, &off), input, "{text}");
        }
        // Text that looks like `|-|` on the next line does not make a table.
        assert_eq!(html(&md("a|b<br/>|-|-|")), "a|b<br/>|-|-|");
        let linked = md(&html("See PROJ-123 for details"));
        assert!(linked.contains("PROJ-123") && !linked.contains("polarion-rte-link"));
    }

    #[test]
    fn canonical_markdown_round_trips() {
        for markdown in [
            "Some **bold**, *italic*, `code`, ~~strike~~, ++under++, ==mark==, x^2^ and H~2~O.",
            "# Title\n\nA paragraph\\\nwith a break.\n\n## Section\n\n- a\n- b\n  - c\n\n<!-- end list -->\n\n1. one\n2. two",
            "> quoted\n> \n> > nested",
            "- [ ] open\n- [x] done",
            "| a | b |\n| --- | :-: |\n| 1 | 2 |",
            "```rust\nfn main() {}\n```",
            "Formula $\\sqrt{x}$ and $$\\int_0^1$$.",
            "```math\na \\\\\nb\n```",
            "See PROJ-1, PROJ\\-2 and [a link](https://example.com \"t\").",
            "![alt](i.png \"title\")",
            "-----",
            "> [!TIP]\n> body",
            "<span style=\"color: red;\">red</span> text",
        ] {
            assert_canonical(markdown);
        }
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

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'s>(&mut self, items: &[&'s str]) -> &'s str {
        items[self.below(items.len())]
    }
}

/// RTE-like HTML: styled spans, breaks, links, formulas, lists, quotes, tables.
fn random_html(rng: &mut Rng, depth: usize) -> String {
    const TEXT: [&str; 22] = [
        "word",
        "two words",
        " ",
        "PROJ-12",
        "MY_ID-3",
        "a*b",
        "x_y",
        "$5",
        "2^3",
        "==",
        "++",
        "&amp;",
        "&lt;tag&gt;",
        "&nbsp;",
        "www.example.com",
        "http://x.y",
        "#1",
        "| a |",
        "-",
        "1.",
        "ä😀",
        "\n  ",
    ];
    const OPEN: [&str; 14] = [
        "<span style=\"font-weight: bold;\">",
        "<span style=\"font-style: italic;\">",
        "<span style=\"text-decoration: underline;\">",
        "<span style=\"text-decoration: line-through;\">",
        "<span style=\"background-color: #FFFF00;\">",
        "<span style=\"color: #FF0000;\">",
        "<span style=\"font-family: 'Courier New', Courier, monospace;\">",
        "<span style=\"font-size: 12pt;\">",
        "<span style=\"font-weight: bold; font-size: 18pt;\">",
        "<sup>",
        "<sub>",
        "<a href=\"https://example.com/x\">",
        "<span style=\"font-weight: normal;\">",
        "<i>",
    ];
    let mut out = String::new();
    for _ in 0..1 + rng.below(6) {
        match rng.below(14) {
            0..=4 => out.push_str(rng.pick(&TEXT)),
            5 | 6 => out.push_str(rng.pick(&["<br/>", "<br>", "<br/><br/>", "<br/><br/><br/>"])),
            7 if depth < 4 => {
                let open = rng.pick(&OPEN);
                let close = if open.starts_with("<sup") {
                    "</sup>"
                } else if open.starts_with("<sub") {
                    "</sub>"
                } else if open.starts_with("<a") {
                    "</a>"
                } else if open.starts_with("<i") {
                    "</i>"
                } else {
                    "</span>"
                };
                out.push_str(open);
                out.push_str(&random_html(rng, depth + 1));
                out.push_str(close);
            }
            8 => out.push_str(rng.pick(&[
                LINK_SPAN,
                "<span class=\"polarion-rte-link\" data-type=\"workItem\" id=\"fake\" data-item-id=\"MP-1\" data-option-id=\"short\"></span>",
                "<span class=\"polarion-rte-formula\" data-source=\"\\sqrt{x}\"></span>",
                "<span data-source=\"a\nb\" data-inline=\"false\" class=\"polarion-rte-formula\"></span>",
                "<span class=\"polarion-rte-formula\" data-source=\"x$\"></span>",
                "<img src=\"workitemimg:1-a.png\" style=\"max-width: 650px;\"/>",
                "<img src=\"workitemimg:2-b.png\" style=\"width: 10px;\">",
            ])),
            9 if depth < 3 => {
                let tag = rng.pick(&["ul", "ol"]);
                out.push_str(&format!("<{tag}>"));
                for _ in 0..1 + rng.below(3) {
                    out.push_str("<li>");
                    if rng.below(3) == 0 {
                        out.push_str(rng.pick(&["☐ ", "☑ "]));
                    }
                    out.push_str(&random_html(rng, depth + 1));
                    out.push_str("</li>");
                }
                out.push_str(&format!("</{tag}>"));
            }
            10 if depth < 3 => {
                out.push_str(rng.pick(&[
                    "<div style=\"margin-left: 40px;\">",
                    "<div style=\"text-align: center;\">",
                    "<div>",
                    "<p>",
                ]));
                out.push_str(&random_html(rng, depth + 1));
                out.push_str(if out.ends_with("<p>") { "</p>" } else { "</div>" });
            }
            11 if depth < 2 => {
                out.push_str("<table><tbody><tr><th>h</th><th>");
                out.push_str(&random_html(rng, 3));
                out.push_str("</th></tr><tr><td>");
                out.push_str(&random_html(rng, 3));
                out.push_str("</td><td>c</td></tr></tbody></table>");
            }
            12 => out.push_str(rng.pick(&["<hr/>", "<pre>let x = 1;\n\nlet y;</pre>"])),
            _ => out.push_str(rng.pick(&TEXT)),
        }
    }
    out
}

/// Larger pieces shaped like Polarion's rich text editor output and the forward direction's.
fn random_blocks(rng: &mut Rng, depth: usize) -> String {
    const TH: &str = "font-weight: bold;background-color: #F0F0F0;text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;padding: 5px;";
    const TD: &str =
        "text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;padding: 5px;";
    const PIECES: [&str; 24] = [
        "<span style=\"font-weight: bold; font-size: 22pt; line-height: 1.2;\">Title</span>",
        "<span style=\"font-weight: bold; font-size: 14pt;\">Sub *x*</span>",
        "<span style=\"font-size: 18pt;\">Big</span>",
        "<span style=\"font-weight: bold; font-size: 22pt;\"> </span>",
        "Font Size <span style=\"font-size: 12pt;line-height: 1.5;\">12</span>",
        "<span style=\"font-family: &#39;Segoe UI&#39;, Selawik, sans-serif;\">segoe</span>",
        "<font color=\"#00FF00\">green</font>",
        "<a href=\"https://polarion.example.com/polarion/#/project/P/workitem?id=PROJ-7\">PROJ-7</a>",
        "<a href=\"https://example.com/a(b)\" title=\"T &quot;q&quot;\">link</a>&nbsp;",
        "<a href=\"mailto:a@b.cd\">a@b.cd</a>",
        "<a href=\"https://example.com\">https://example.com</a>",
        "<span class=\"polarion-rte-formula\" data-source=\" x \"></span>",
        "<span class=\"polarion-rte-formula\" data-source=\"a`$b\"></span>",
        "<span class=\"polarion-rte-formula\" data-source=\"\"></span>",
        "&nbsp;<span data-source=\"\\sqrt{2}\" data-inline=\"false\" class=\"polarion-rte-formula\"></span>&nbsp;",
        "<span class=\"polarion-rte-formula\" data-source=\"x\"></span>2",
        "&amp;#65; &#39;q&#39; &lt;b&gt; &amp;amp;",
        "<img title=\"a.png\" src=\"workitemimg:1-a.png\" style=\"max-width: 650px;\" data-lastmodified=\"2025-01-01T08:00:00.000+01:00\"/>",
        "<img src=\"workitemimg:3-mermaid-0123456789abcdef.svg\" title=\"mermaid-0123456789abcdef.svg\" style=\"max-width: 650px;\"/>",
        "<img src=\"https://mermaid.ink/img/Z3JhcGggVEQKICAgIEEtLT5C\" style=\"max-width: 650px;\" />",
        "&lt;Image: diagram.svg&gt;",
        "<span style=\"color: #FF0000;\">red <b>bold</b></span>",
        "<span style=\"font-weight: normal;\">normal<br/><br/>para</span>",
        "PROJ-1 MP-42 UTF-8 (ABC-7).",
    ];
    let mut out = String::new();
    for _ in 0..1 + rng.below(5) {
        match rng.below(12) {
            0..=3 => out.push_str(rng.pick(&PIECES)),
            4 => out.push_str(rng.pick(&["<br/><br/>", "<br/>", " \n", "<br/> <br/>"])),
            5 if depth < 2 => {
                let header = format!(
                    "<tr><th style=\"{TH}\">H1</th><th style=\"{}\">{}</th></tr>",
                    TH.replace("left", "center"),
                    rng.pick(&PIECES)
                );
                let row = format!(
                    "<tr><td style=\"{TD}\">{}</td><td style=\"{}\">c</td></tr>",
                    random_html(rng, 3),
                    TD.replace("left", "center")
                );
                out.push_str(&format!(
                    "<table id=\"polarion_wiki macro name=table\" class=\"polarion-Document-table\" style=\"width: 100%;max-width: 1280px;margin-left: 0px;margin-right: auto;border: 1px solid #CCCCCC;empty-cells: show;border-collapse: collapse;max-width: 1280.0px;\">\n  <tbody>\n    {header}\n    {row}\n  </tbody>\n</table>"
                ));
            }
            6 if depth < 2 => out.push_str(&format!(
                "<table><tr><td colspan=\"2\">{}</td></tr><tr><td>a</td><td>b</td></tr></table>",
                random_html(rng, 3)
            )),
            7 if depth < 2 => {
                let alert = styles::ALERTS[rng.below(5)];
                out.push_str(&format!(
                    "<div style=\"{}\"><span style=\"{}\">{}</span><br/>{}</div>",
                    alert.div_style(),
                    alert.title_style(),
                    rng.pick(&[alert.title, "Custom title"]),
                    random_blocks(rng, depth + 1)
                ));
            }
            8 if depth < 2 => {
                out.push_str(rng.pick(&[
                    "<div style=\"margin-left: 40px;\">\n  ",
                    "<div style=\"margin-left: 80px;\">",
                    "<div style=\"text-align: center;\">\n  ",
                    "<div style=\"text-align: right;\">",
                    "<p class=\"polarion-rte-caption-paragraph\">\n  Figure <span data-sequence=\"Figure\" class=\"polarion-rte-caption\">#</span> ",
                ]));
                let closing = if out.ends_with("Figure\" class=\"polarion-rte-caption\">#</span> ") {
                    "\n</p>"
                } else {
                    "<br/> \n</div>"
                };
                out.push_str(&random_blocks(rng, depth + 1));
                out.push_str(closing);
            }
            9 => out.push_str(rng.pick(&[
                "<div style=\"margin-left: 40px;\"><span style=\"font-family: 'Courier New', Courier, monospace;\">line1<br/>&nbsp;&nbsp;line2<br/></span></div>",
                "<div style=\"color:#000000;background-color:#FFFFFF;\"><pre><span style=\"color:#0000FF;\">let</span> x = 1;</pre></div>",
                "<pre class=\"language-rust\" style=\"margin-left: 40px;\">fn main() {}\n</pre>",
                "<pre><code class=\"language-mermaid\">graph TD\n  A--&gt;B</code></pre>",
                "<hr/>",
            ])),
            10 if depth < 2 => {
                out.push_str("<ol start=\"3\">\n  <li>");
                out.push_str(&random_blocks(rng, depth + 1));
                out.push_str("</li>\n  <li>☑ done</li>\n</ol>\n \n");
            }
            _ => out.push_str(&random_html(rng, depth + 2)),
        }
    }
    out
}

/// Markdown made of the constructs both directions map.
fn random_markdown(rng: &mut Rng) -> String {
    const INLINE: [&str; 26] = [
        "word",
        "**bold**",
        "*it*",
        "`code`",
        "~~s~~",
        "++u++",
        "==h==",
        "x^2^",
        "H~2~O",
        "$x$",
        "$$y$$",
        "[l](https://e.x)",
        "![i](i.png)",
        "PROJ-1",
        "PROJ\\-2",
        "<span style=\"color: red;\">c</span>",
        "a\\*b",
        "\\$",
        " ",
        ", ",
        ".",
        "\\\n",
        "MY\\_X-1",
        "&nbsp;",
        "**a *b* c**",
        "<br/>",
    ];
    let mut blocks = Vec::new();
    for _ in 0..1 + rng.below(4) {
        let line: String = (0..1 + rng.below(6)).map(|_| rng.pick(&INLINE)).collect();
        let block = match rng.below(9) {
            0 => format!("# {}", line.replace("\\\n", " ")),
            1 => format!("- {line}\n- b"),
            2 => format!("> {line}"),
            3 => format!(
                "| a | b |\n| --- | --- |\n| {} | x |",
                line.replace("\\\n", " ")
            ),
            4 => "```\ncode\n```".to_owned(),
            5 => "-----".to_owned(),
            6 => format!("1. {line}"),
            _ => line,
        };
        blocks.push(block);
    }
    blocks.join("\n\n")
}

/// A description shaped like Polarion's rich text editor output.
const RTE_SAMPLE: &str = concat!(
    "<span style=\"font-weight: normal;\">Intro text with a trailing space <br/><br/>",
    "<span class=\"polarion-rte-link\" data-type=\"workItem\" id=\"fake\" data-item-id=\"PROJ-1\" data-option-id=\"long\"></span><br/><br/>",
    "<span class=\"polarion-rte-link\" data-type=\"workItem\" id=\"fake\" data-item-id=\"PROJ-2\" data-option-id=\"short\"></span> <br/><br/>",
    " <br/>",
    "Some <span style=\"font-weight: bold;\">bold</span>, <span style=\"font-style: italic;\">italic</span>, ",
    "<span style=\"text-decoration: underline;\">underlined</span>, X<sup>2</sup>, X<sub>2</sub>, ",
    "<span style=\"color: #FF0000;\">red</span> and <span style=\"background-color: #FF0000;\">red background</span>.<br/><br/>",
    "<span style=\"font-family: &#39;Segoe UI&#39;, Selawik, &#39;Open Sans&#39;, sans-serif;\">Segoe</span> ",
    "<span style=\"font-family: &#39;Courier New&#39;, Courier, monospace;\">mono</span> ",
    "<span style=\"font-family: Georgia, serif;\">Georgia</span><br/><br/>",
    "Font Size <span style=\"font-size: 12pt;line-height: 1.5;\">12</span></span><br/><br/> \n",
    "<ol>\n  <li>first</li>\n  <li>second</li>\n</ol>\n \n<ul>\n  <li>bullet</li>\n</ul>\n <br/> \n",
    "<div style=\"margin-left: 40px;\">\n  Indented<br/> \n</div>\n <br/>",
    "<div style=\"text-align: center;\">\n  Align Center<br/> \n</div>",
    "<div style=\"text-align: right;\">\n  Align Right<br/> \n</div>",
    "<a href=\"https://example.com/page\" target=\"_top\" class=\"descriptionLink\">Label</a>&nbsp;<br/><br/>",
    "&lt;Image: diagram.svg&gt;<br/>",
    "&nbsp;<span data-source=\"\\sqrt{a+b}\" data-inline=\"false\" class=\"polarion-rte-formula\"></span>&nbsp;<br/>",
    "Inline <span data-source=\"x^2 &amp; y\" class=\"polarion-rte-formula\"></span>&nbsp;formula<br/><br/>",
    "<table id=\"polarion_wiki macro name=table\" class=\"polarion-Document-table\" style=\"width: 100%;max-width: 1280px;margin-left: 0px;margin-right: auto;border: 1px solid #CCCCCC;empty-cells: show;border-collapse: collapse;max-width: 1280.0px;\">\n",
    "  <tbody>\n    <tr>\n",
    "      <th style=\"font-weight: bold;background-color: #F0F0F0;text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;border-top: 1px solid #CCCCCC;border-bottom: 1px solid #CCCCCC;border-right: 1px solid #CCCCCC;border-left: 1px solid #CCCCCC;padding: 5px;\">Name</th>\n",
    "      <th style=\"font-weight: bold;background-color: #F0F0F0;text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;border-top: 1px solid #CCCCCC;border-bottom: 1px solid #CCCCCC;border-right: 1px solid #CCCCCC;border-left: 1px solid #CCCCCC;padding: 5px;\">Value</th>\n",
    "    </tr>\n    <tr>\n",
    "      <td style=\"text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;border-top: 1px solid #CCCCCC;border-bottom: 1px solid #CCCCCC;border-right: 1px solid #CCCCCC;border-left: 1px solid #CCCCCC;padding: 5px;\">a</td>\n",
    "      <td style=\"text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;border-top: 1px solid #CCCCCC;border-bottom: 1px solid #CCCCCC;border-right: 1px solid #CCCCCC;border-left: 1px solid #CCCCCC;padding: 5px;\"><img src=\"workitemimg:2-c.jpg\" style=\"width: 100px;height: 74px;\"></td>\n",
    "    </tr>\n  </tbody>\n</table>\n",
    "<img title=\"a.png\" src=\"workitemimg:1-a.png\" style=\"max-width: 650px;\" data-lastmodified=\"2025-01-01T08:00:00.000+01:00\"/><br/>",
    "<p class=\"polarion-rte-caption-paragraph\">\n  Figure <span data-sequence=\"Figure\" class=\"polarion-rte-caption\">#</span> caption text\n</p>",
    "<img src=\"workitemimg:1-b.png\"/>"
);

#[test]
fn an_rte_like_description_needs_no_fallback() {
    let kept = md(RTE_SAMPLE);
    for expected in [
        "PROJ-1\n",
        "data-option-id=\"short\"></span>",
        "**bold**",
        "<span style=\"color: #FF0000;\">red</span>",
        "<span style=\"background-color: #FF0000;\">red background</span>",
        "`mono`",
        "<span style=\"font-size: 12pt; line-height: 1.5;\">12</span>",
        "1. first\n2. second",
        "> Indented",
        "<div style=\"text-align: center;\">\n\nAlign Center\n\n</div>",
        "[Label](https://example.com/page)",
        "$$\\sqrt{a+b}$$",
        "$x^2 & y$",
        "| Name | Value |",
        "![](workitemimg:1-a.png \"a.png\")",
        "<p class=\"polarion-rte-caption-paragraph\">",
    ] {
        assert!(kept.contains(expected), "{expected}: {kept}");
    }
    let dropped = md_drop(RTE_SAMPLE);
    for absent in ["<span", "<div", "<p", "<img"] {
        assert!(!dropped.contains(absent), "{absent}: {dropped}");
    }
    assert!(dropped.contains("PROJ\\-2"), "{dropped}");
}

#[test]
fn work_item_anchors_and_settings() {
    let url = "https://polarion.example.com/polarion/#/project/P/workitem?id=PROJ-7";
    assert_eq!(md(&format!("see <a href=\"{url}\"></a>")), "see PROJ-7");
    let off = ToMarkdownOptions {
        work_items: WorkItemIds::Off,
        ..keep()
    };
    assert_eq!(
        md_with(&format!("<a href=\"{url}\">PROJ-7</a>"), &off),
        format!("[PROJ-7]({url})")
    );
}

#[test]
fn every_filtered_element_is_dropped() {
    // (HTML parsers ignore `<frameset>` in a fragment: its text stays.)
    for name in [
        "iframe", "noembed", "noframes", "noscript", "object", "script", "style", "textarea",
        "title", "xmp",
    ] {
        let input = format!("a<{name} title=\"t\">x</{name}>b");
        for out in [md(&input), md_drop(&input)] {
            assert_eq!(out, "ab", "{name}");
        }
    }
    // Elements without content.
    for name in ["base", "embed", "frame", "link", "meta"] {
        let input = format!("a<{name} title=\"t\" src=\"u\" href=\"u\">b");
        for out in [md(&input), md_drop(&input)] {
            assert_eq!(out, "ab", "{name}");
        }
    }
}

#[test]
fn separators_inside_list_items() {
    // Between text blocks two breaks, around boxes none.
    assert_eq!(
        md("<ul><li>a<ul><li>b</li></ul>c</li></ul>"),
        "- a\n  \n  - b\n  \n  c"
    );
    assert_eq!(md("<ul><li>a<br/><br/>b</li></ul>"), "- a\n  \n  b");
    assert_eq!(
        md("<ul><li>a<pre>x</pre>b</li></ul>"),
        "- a\n  \n  ```\n  x\n  ```\n  \n  b"
    );
}

#[test]
fn heading_lines_split_their_paragraph() {
    let out = md("a<br/><span style=\"font-weight: bold; font-size: 14pt;\">Sub</span><br/>b");
    assert_eq!(out, "a\n\n### Sub\n\nb");
}

/// Quadratic work would take minutes here (debug builds take about a second per megabyte).
#[test]
fn large_documents_take_linear_time() {
    let mut big = String::new();
    while big.len() < 300_000 {
        big.push_str(RTE_SAMPLE);
        big.push_str("<br/><br/>");
    }
    let wide = "<span style=\"color: red;\">PROJ-1 x</span> ".repeat(5_000);
    for input in [big, wide] {
        let started = std::time::Instant::now();
        let report = to_markdown(&input, &keep(), &DiagramSources::new());
        let took = started.elapsed();
        assert_eq!((report.fallbacks, report.invalid), (0, 0));
        assert!(took < std::time::Duration::from_secs(60), "{took:?}");
    }
}

#[test]
fn random_rte_html_reaches_a_valid_fixpoint() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let sources: DiagramSources = [(
        "3-mermaid-0123456789abcdef.svg".to_owned(),
        "graph TD\n    A-->B".to_owned(),
    )]
    .into_iter()
    .collect();
    for case in 0..450 {
        let input = match case % 3 {
            0 => random_html(&mut rng, 0),
            1 => random_blocks(&mut rng, 0),
            _ => tag_soup(&mut rng),
        };
        for options in [keep(), dropping()] {
            let report = to_markdown(&input, &options, &sources);
            assert_eq!(report.invalid, 0, "case {case}: {input:?}");
            let html = to_html(&report.markdown, &forward(&options));
            let again = to_markdown(&html, &options, &DiagramSources::new());
            assert_eq!(
                again.markdown, report.markdown,
                "case {case}: {input:?}\nhtml: {html:?}"
            );
            assert_eq!(again.invalid, 0, "case {case}");
        }
    }
}

#[test]
fn random_markdown_reaches_a_valid_fixpoint() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    for case in 0..400 {
        let markdown = random_markdown(&mut rng);
        let input = html(&markdown);
        for options in [keep(), dropping()] {
            let report = to_markdown(&input, &options, &DiagramSources::new());
            assert_eq!(report.invalid, 0, "case {case}: {markdown:?}");
            let html = to_html(&report.markdown, &forward(&options));
            let again = to_markdown(&html, &options, &DiagramSources::new());
            assert_eq!(
                again.markdown, report.markdown,
                "case {case}: {markdown:?}\nhtml: {html:?}"
            );
        }
    }
}

/// Malformed HTML: tags opened and closed in any order, odd attributes, entities, comments.
fn tag_soup(rng: &mut Rng) -> String {
    const PIECES: [&str; 48] = [
        "<b>",
        "</b>",
        "<i>",
        "</i>",
        "<u>",
        "<s>",
        "</s>",
        "<sup>",
        "</sup>",
        "<sub>",
        "<span style=\"font-weight: bold; font-size: 22pt\">",
        "<span style=\"color: red\">",
        "<span style=\"font-family: monospace\">",
        "<span style='text-decoration: underline'>",
        "</span>",
        "<a href=\"u\">",
        "<a href=\"javascript:x\">",
        "</a>",
        "<div>",
        "<div style=\"margin-left: 40px\">",
        "<div style=\"text-align: center\">",
        "</div>",
        "<p>",
        "</p>",
        "<ul>",
        "<ol>",
        "<li>",
        "</li>",
        "</ul>",
        "<table>",
        "<tr>",
        "<th>",
        "<td>",
        "</table>",
        "<pre>",
        "</pre>",
        "<br>",
        "<img src=x onerror=y>",
        "<img src=\"workitemimg:1-a.png\">",
        "<!-- c -->",
        "<script>x</script>",
        "<span class=\"polarion-rte-link\" data-type=\"workItem\" data-item-id=\"PROJ-1\" data-option-id=\"long\">",
        "<span class=\"polarion-rte-formula\" data-source=\"$x\">",
        "&amp;&lt;&#39;&nbsp;&#0;&bogus;",
        " text ",
        "PROJ-2",
        "*_`[]",
        "\n\n",
    ];
    (0..1 + rng.below(25)).map(|_| rng.pick(&PIECES)).collect()
}

/// A long random run that also reports the blocks the self-check rejected:
/// `CASES=3000 SEEDS=1,2,3 cargo test -p polarion-markdown --lib long_random_run -- --ignored
/// --nocapture`. Every case must reach the fixpoint; fallbacks are listed.
#[test]
#[ignore]
fn long_random_run() {
    let seeds: Vec<u64> = std::env::var("SEEDS")
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![1, 2, 3]);
    let cases: usize = std::env::var("CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);
    let sources: DiagramSources = [(
        "3-mermaid-0123456789abcdef.svg".to_owned(),
        "graph TD\n    A-->B".to_owned(),
    )]
    .into_iter()
    .collect();
    let mut fallbacks = 0;
    let mut failures = 0;
    check::diagnostics::collect();
    for seed in seeds {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0x2545_F491_4F6C_DD1D));
        for case in 0..cases {
            let input = match case % 4 {
                0 => random_html(&mut rng, 0),
                1 => random_blocks(&mut rng, 0),
                2 => tag_soup(&mut rng),
                _ => html(&random_markdown(&mut rng)),
            };
            for options in [keep(), dropping()] {
                let report = to_markdown(&input, &options, &sources);
                assert_eq!(report.invalid, 0, "seed {seed} case {case}: {input:?}");
                let rejected = check::diagnostics::take();
                if report.fallbacks > 0 {
                    fallbacks += 1;
                    println!("fallback: seed {seed} case {case} {:?}", options.raw_html);
                    for entry in &rejected {
                        println!("{entry}");
                    }
                }
                let html = to_html(&report.markdown, &forward(&options));
                let again = to_markdown(&html, &options, &DiagramSources::new());
                check::diagnostics::take();
                if again.markdown != report.markdown {
                    failures += 1;
                    println!(
                        "not a fixpoint: seed {seed} case {case} {:?}: {input:?}\nfirst: {:?}\nhtml: {html:?}\nagain: {:?}",
                        options.raw_html, report.markdown, again.markdown
                    );
                }
            }
        }
    }
    println!("fallbacks {fallbacks}, not a fixpoint {failures}");
    assert_eq!(failures, 0);
}
