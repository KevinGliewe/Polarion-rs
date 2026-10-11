//! Polarion HTML to Markdown, case by case: headings, inline formatting, Polarion's link and
//! formula spans, code, diagrams and images, blocks, what drop mode discards, and whole
//! documents.
//!
//! Every conversion here is also checked for the properties of the whole document
//! ([`convert`]): comrak's `validate()` accepts every built tree, these well-formed inputs need no
//! fallback, the Markdown has no trailing newline, and it is a fixpoint
//! (`to_markdown(to_html(M)) == M`). Forward output of canonical Markdown comes back exactly
//! ([`round_trips`]). Expected Markdown is what comrak's writer produces: `- ` bullets, `-----`
//! rules, `\` hard breaks, blank lines inside quotes and list items written with their prefix
//! (`> ` and two spaces).

use polarion_markdown::{
    DiagramAttachments, DiagramSources, RawHtml, ToHtmlOptions, ToMarkdownOptions, WorkItemIds,
    mermaid, to_html, to_html_with_diagrams, to_markdown, to_markdown_report,
    to_markdown_with_diagrams,
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

fn ids(work_items: WorkItemIds, raw_html: RawHtml) -> ToMarkdownOptions {
    ToMarkdownOptions {
        raw_html,
        work_items,
    }
}

/// The options of the conversion back to HTML that go with `options`: the same work item ids,
/// and no highlighting (its time budget can make the HTML depend on the machine).
fn forward_options(options: &ToMarkdownOptions) -> ToHtmlOptions {
    ToHtmlOptions {
        work_items: options.work_items.clone(),
        highlight: false,
    }
}

/// Markdown converted to HTML with the options that go with `options`.
fn html_of(markdown: &str, options: &ToMarkdownOptions) -> String {
    to_html(markdown, &forward_options(options))
}

/// The Markdown of `html`, checked: every built tree valid, no fallback, no trailing newline,
/// and converting it to HTML and back gives it again.
fn convert(html: &str, options: &ToMarkdownOptions, sources: &DiagramSources) -> String {
    let report = to_markdown_report(html, options, sources);
    assert_eq!(
        report.invalid, 0,
        "invalid trees for {html:?} ({options:?}):\n{}",
        report.markdown
    );
    assert_eq!(
        report.fallbacks, 0,
        "fallbacks for {html:?} ({options:?}):\n{}",
        report.markdown
    );
    let markdown = report.markdown;
    assert!(
        !markdown.ends_with('\n'),
        "trailing newline for {html:?}: {markdown:?}"
    );
    let back = html_of(&markdown, options);
    let again = to_markdown_with_diagrams(&back, options, sources);
    assert_eq!(
        again, markdown,
        "not a fixpoint: {html:?} ({options:?}) gave {markdown:?}, whose HTML {back:?}"
    );
    markdown
}

/// [`convert`] in keep mode.
fn md(html: &str) -> String {
    convert(html, &keep(), &DiagramSources::new())
}

/// [`convert`] in drop mode.
fn md_drop(html: &str) -> String {
    convert(html, &drop_html(), &DiagramSources::new())
}

fn md_with(html: &str, options: &ToMarkdownOptions) -> String {
    convert(html, options, &DiagramSources::new())
}

/// The Markdown of odd input: valid trees and a fixpoint, fallbacks allowed.
fn lenient(html: &str, options: &ToMarkdownOptions) -> String {
    let report = to_markdown_report(html, options, &DiagramSources::new());
    assert_eq!(
        report.invalid, 0,
        "invalid trees for {html:?} ({options:?}):\n{}",
        report.markdown
    );
    let back = html_of(&report.markdown, options);
    assert_eq!(
        to_markdown(&back, options),
        report.markdown,
        "not a fixpoint: {html:?} ({options:?}), HTML {back:?}"
    );
    report.markdown
}

/// Runs `check` on every input on a thread of its own, allowing each `limit`: a conversion that
/// hangs fails the test, naming its input, instead of blocking the run.
fn each_within(inputs: &[&'static str], limit: std::time::Duration, check: fn(&str)) {
    let (sender, receiver) = std::sync::mpsc::channel();
    let inputs = inputs.to_vec();
    let worker = std::thread::spawn(move || {
        for input in inputs {
            let _ = sender.send(Some(input));
            check(input);
        }
        let _ = sender.send(None);
    });
    let mut current = None;
    loop {
        match receiver.recv_timeout(limit) {
            Ok(Some(input)) => current = Some(input),
            Ok(None) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("no result within {limit:?} for {current:?}")
            }
        }
    }
    if let Err(panic) = worker.join() {
        std::panic::resume_unwind(panic);
    }
}

/// Canonical Markdown without raw HTML comes back exactly from its HTML, in both modes.
fn round_trips(markdown: &str) {
    for options in [keep(), drop_html()] {
        round_trips_with(markdown, &options);
    }
}

/// Canonical Markdown comes back exactly from its HTML with these options: the HTML first (the
/// conversion is lossless on forward output), then the Markdown itself.
fn round_trips_with(markdown: &str, options: &ToMarkdownOptions) {
    let html = html_of(markdown, options);
    let back = convert(&html, options, &DiagramSources::new());
    assert_eq!(
        html_of(&back, options),
        html,
        "{markdown:?} came back as {back:?} ({options:?})"
    );
    assert_eq!(back, markdown, "from {html:?} ({options:?})");
}

/// Whether a line of the Markdown starts with `#` (a heading).
fn has_heading_line(markdown: &str) -> bool {
    markdown
        .lines()
        .any(|line| line.trim_start().starts_with('#'))
}

/// The raw HTML tags in Markdown (comrak escapes a `<` of text, so these are tags).
fn raw_tags(markdown: &str) -> Vec<&'static str> {
    [
        "<span", "<div", "<img", "<table", "<p ", "<p>", "<br", "<a ", "<sup", "<sub", "<pre",
        "<ul", "<ol", "<li",
    ]
    .into_iter()
    .filter(|tag| markdown.contains(tag))
    .collect()
}

/// The source of the first formula span in some HTML, and whether it is a display formula.
fn formula_in(html: &str) -> Option<(String, bool)> {
    let document = scraper::Html::parse_fragment(html);
    document
        .tree
        .nodes()
        .filter_map(|node| node.value().as_element())
        .find(|element| {
            element.attr("class").is_some_and(|class| {
                class
                    .split_whitespace()
                    .any(|c| c == "polarion-rte-formula")
            })
        })
        .map(|element| {
            (
                element.attr("data-source").unwrap_or_default().to_owned(),
                element.attr("data-inline") == Some("false"),
            )
        })
}

/// The `style` of the first `span` in some HTML.
fn first_span_style(html: &str) -> Option<String> {
    let document = scraper::Html::parse_fragment(html);
    document
        .tree
        .nodes()
        .filter_map(|node| node.value().as_element())
        .find(|element| element.name() == "span")
        .and_then(|element| element.attr("style"))
        .map(str::to_owned)
}

const MONO: &str = r#"<span style="font-family: 'Courier New', Courier, monospace;">"#;
const QUOTE: &str = r#"<div style="margin-left: 40px;">"#;
const TABLE_OPEN: &str = concat!(
    r#"<table id="polarion_wiki macro name=table" class="polarion-Document-table" "#,
    r#"style="width: 100%; max-width: 1280px; margin-left: 0px; margin-right: auto; "#,
    r#"border: 1px solid #CCCCCC; empty-cells: show; border-collapse: collapse;"><tbody>"#
);
const TABLE_CLOSE: &str = "</tbody></table>";
const TH: &str = r#"<th style="font-weight: bold; background-color: #F0F0F0; text-align: left; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;">"#;
const TD: &str = r#"<td style="text-align: left; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;">"#;
const TD_STYLE: &str =
    "text-align: left; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;";

/// The styles of the table Polarion's rich text editor writes.
const RTE_TABLE: &str = "width: 100%;max-width: 1280px;margin-left: 0px;margin-right: auto;border: 1px solid #CCCCCC;empty-cells: show;border-collapse: collapse;max-width: 1280.0px;";
const RTE_TH: &str = "font-weight: bold;background-color: #F0F0F0;text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;border-top: 1px solid #CCCCCC;border-bottom: 1px solid #CCCCCC;border-right: 1px solid #CCCCCC;border-left: 1px solid #CCCCCC;padding: 5px;";
const RTE_TD: &str = "text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;border-top: 1px solid #CCCCCC;border-bottom: 1px solid #CCCCCC;border-right: 1px solid #CCCCCC;border-left: 1px solid #CCCCCC;padding: 5px;";

/// A long work item link, as the forward direction writes it.
fn wi(id: &str) -> String {
    format!(
        r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="{id}" data-option-id="long"></span>"#
    )
}

/// A formula span, attributes in the forward direction's order.
fn formula(source: &str, display: bool) -> String {
    let inline = if display {
        r#" data-inline="false""#
    } else {
        ""
    };
    format!(r#"<span data-source="{source}"{inline} class="polarion-rte-formula"></span>"#)
}

/// A table with Polarion's styles (left aligned), header cells and data cells as given.
fn polarion_table(header: &[&str], rows: &[&[&str]]) -> String {
    let mut html = String::from(TABLE_OPEN);
    html.push_str("<tr>");
    for cell in header {
        html.push_str(&format!("{TH}{cell}</th>"));
    }
    html.push_str("</tr>");
    for row in rows {
        html.push_str("<tr>");
        for cell in *row {
            html.push_str(&format!("{TD}{cell}</td>"));
        }
        html.push_str("</tr>");
    }
    html.push_str(TABLE_CLOSE);
    html
}

/// A table as the rich text editor writes it: its styles, indented `tbody` and rows.
fn rte_table(header: &[&str], rows: &[&[&str]]) -> String {
    let mut html = format!(
        "<table id=\"polarion_wiki macro name=table\" class=\"polarion-Document-table\" style=\"{RTE_TABLE}\">\n  <tbody>\n    <tr>\n"
    );
    for cell in header {
        html.push_str(&format!("      <th style=\"{RTE_TH}\">{cell}</th>\n"));
    }
    html.push_str("    </tr>\n");
    for row in rows {
        html.push_str("    <tr>\n");
        for cell in *row {
            html.push_str(&format!("      <td style=\"{RTE_TD}\">{cell}</td>\n"));
        }
        html.push_str("    </tr>\n");
    }
    html.push_str("  </tbody>\n</table>");
    html
}

/// A description shaped like what Polarion's rich text editor stores (neutral content).
fn rte_document() -> String {
    [
        "<span style=\"font-weight: normal;\">Intro text of the item.<br/><br/>A second paragraph ending in a space <br/><br/></span>",
        &wi("PROJ-101"),
        "<br/><br/>",
        r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-102" data-option-id="short"></span> <br/><br/> <br/>"#,
        "<span style=\"font-weight: bold;\">Bold</span>, <span style=\"font-style: italic;\">italic</span>, ",
        "<span style=\"text-decoration: underline;\">underlined</span>, X<sup>2</sup>, H<sub>2</sub>O, ",
        "<span style=\"color: #FF0000;\">red</span> and <span style=\"background-color: #FF0000;\">marked</span>.<br/><br/>",
        "<span style=\"font-family: &#39;Segoe UI&#39;, Selawik, &#39;Open Sans&#39;, sans-serif;\">Segoe</span> ",
        "<span style=\"font-family: Arial, Helvetica, sans-serif;\">Arial</span> ",
        "<span style=\"font-family: &#39;Courier New&#39;, Courier, monospace;\">mono</span> ",
        "<span style=\"font-family: Georgia, serif;\">Georgia</span><br/><br/>",
        "Font Size <span style=\"font-size: 12pt;line-height: 1.5;\">12</span><br/><br/> \n",
        "<ol>\n  <li>First</li>\n  <li>Second</li>\n</ol>\n \n",
        "<ul>\n  <li>Bullet</li>\n</ul>\n <br/> \n",
        "<div style=\"margin-left: 40px;\">\n  Indented<br/> \n</div>\n <br/>\n",
        "<div style=\"text-align: center;\">\n  Align Center<br/> \n</div>\n",
        "<a href=\"https://example.com/spec\" target=\"_top\" class=\"descriptionLink\">Spec</a>&nbsp;<br/><br/>",
        "&lt;Image: diagram.svg&gt;<br/>",
        "&nbsp;<span data-source=\"\\sqrt{x}\" data-inline=\"false\" class=\"polarion-rte-formula\"></span>&nbsp;<br/>",
        "<span data-source=\"a^2\" class=\"polarion-rte-formula\"></span>&nbsp;and more<br/><br/>",
        &rte_table(
            &["H1", "H2"],
            &[&[
                "C1",
                "<img src=\"workitemimg:2-c.jpg\" style=\"width: 100px;height: 74px;\">",
            ]],
        ),
        "\n<img title=\"a.png\" src=\"workitemimg:1-a.png\" style=\"max-width: 650px;\" data-lastmodified=\"2025-01-01T08:00:00.000+01:00\"/><br/>",
        "<p class=\"polarion-rte-caption-paragraph\">\n  Figure <span data-sequence=\"Figure\" class=\"polarion-rte-caption\">#</span> caption text\n</p>",
    ]
    .concat()
}

mod headings {
    use super::*;

    fn bold_sized(size: &str, text: &str) -> String {
        format!(r#"<span style="font-weight: bold; font-size: {size}pt;">{text}</span>"#)
    }

    #[test]
    fn bold_text_sizes_give_heading_levels() {
        for (size, level) in [
            ("22", 1),
            ("18", 2),
            ("14", 3),
            ("12", 4),
            ("11", 5),
            ("10.5", 6),
            ("30", 1),
            ("20", 2),
            ("16", 3),
            ("14.5", 3),
            ("13", 4),
            ("11.5", 5),
            ("10.2", 6),
        ] {
            assert_eq!(
                md(&bold_sized(size, "Text")),
                format!("{} Text", "#".repeat(level)),
                "{size}pt"
            );
        }
        // 22pt is the first level, not the second.
        assert!(!md(&bold_sized("22", "Text")).starts_with("## "));
    }

    #[test]
    fn large_text_filling_its_line_is_a_heading_without_bold() {
        assert_eq!(
            md(r#"<span style="font-size: 22pt;">Text</span>"#),
            "# Text"
        );
        assert_eq!(
            md(r#"<span style="font-size: 18pt;">Text</span>"#),
            "## Text"
        );
        assert_eq!(
            md(r#"<span style="font-size: 14pt;">Text</span>"#),
            "### Text"
        );
    }

    #[test]
    fn a_small_size_inside_a_line_is_no_heading() {
        let html = r#"Text <span style="font-size: 12pt;">12</span>"#;
        let markdown = md(html);
        assert!(!has_heading_line(&markdown), "{markdown}");
        assert!(
            markdown.contains(r#"<span style="font-size: 12pt;">12</span>"#),
            "{markdown}"
        );
        assert_eq!(md_drop(html), "Text 12");
    }

    /// Word writes body text in 11pt and 12pt: not bold, it stays a size.
    #[test]
    fn body_text_sizes_filling_the_line_are_no_heading() {
        for size in ["11", "12"] {
            let html = format!(r#"<span style="font-size: {size}pt;">Text</span>"#);
            let markdown = md(&html);
            assert_eq!(markdown, html, "{size}pt");
            assert_eq!(html_of(&markdown, &keep()), html, "{size}pt");
            assert_eq!(md_drop(&html), "Text", "{size}pt");
        }
    }

    #[test]
    fn the_default_size_is_plain_text() {
        let html = r#"<span style="font-size: 10pt;">Text</span>"#;
        assert_eq!(md(html), "Text");
        assert_eq!(md_drop(html), "Text");
    }

    #[test]
    fn bold_without_a_heading_size_is_strong() {
        assert_eq!(md(&bold_sized("10", "Text")), "**Text**");
        assert_eq!(
            md(r#"<span style="font-weight: bold;">Text</span>"#),
            "**Text**"
        );
    }

    /// An empty heading is its marker (no trailing space at the end of the Markdown).
    #[test]
    fn empty_heading_spans_are_empty_headings() {
        assert_eq!(md(&bold_sized("22", "")), "#");
        assert_eq!(md(&bold_sized("22", " ")), "#");
    }

    #[test]
    fn forward_headings_come_back_exactly() {
        for markdown in [
            "# H1",
            "## H2",
            "### H3",
            "#### H4",
            "##### H5",
            "###### H6",
            "## Sec *it*",
            "# A `c`",
            "# PROJ-1",
            "# [a](u)",
            "# H1\n\n## H2\n\ntext",
        ] {
            round_trips(markdown);
        }
        // The forward spans carry a line height.
        assert_eq!(
            md(
                r#"<span style="font-weight: bold; font-size: 22pt; line-height: 1.2;">Title</span>"#
            ),
            "# Title"
        );
        assert_eq!(
            md(r#"<span style="font-weight: bold; font-size: 14pt; line-height: 1.2;">Sub</span>"#),
            "### Sub"
        );
    }

    #[test]
    fn a_heading_line_splits_its_paragraph() {
        let html = format!("before<br/>{}<br/>after", bold_sized("22", "Title"));
        assert_eq!(md(&html), "before\n\n# Title\n\nafter");
    }

    #[test]
    fn the_smallest_size_of_a_line_gives_its_level() {
        let html = format!("{} {}", bold_sized("22", "Big"), bold_sized("14", "small"));
        assert_eq!(md(&html), "### Big small");
    }

    #[test]
    fn default_size_text_keeps_a_line_from_being_a_heading() {
        let html = format!("{} plain", bold_sized("22", "Big"));
        let markdown = md(&html);
        assert!(
            !has_heading_line(&markdown)
                && markdown.contains("Big")
                && markdown.ends_with(" plain"),
            "{markdown}"
        );
        assert_eq!(md_drop(&html), "**Big** plain");
    }

    #[test]
    fn a_break_inside_a_heading_span_ends_the_heading() {
        assert_eq!(md(&bold_sized("22", "a<br/>b")), "# a\n\n# b");
    }

    #[test]
    fn heading_sizes_in_table_cells_stay_inline() {
        let cell = bold_sized("22", "Big");
        let html = polarion_table(&["H"], &[&[cell.as_str()]]);
        for markdown in [md(&html), md_drop(&html)] {
            assert!(
                !has_heading_line(&markdown)
                    && markdown.starts_with("| H |\n| --- |\n| ")
                    && markdown.contains("Big"),
                "{markdown}"
            );
        }
    }

    #[test]
    fn empty_headings_have_no_trailing_space_anywhere() {
        let empty = r#"<span style="font-weight: bold; font-size: 22pt;"></span>"#;
        assert_eq!(md(&format!("{empty}<br/><br/>after")), "#\n\nafter");
        assert_eq!(md_drop(&format!("{empty}<br/><br/>after")), "#\n\nafter");
        assert_eq!(md(&format!("a<br/><br/>{empty}<br/><br/>b")), "a\n\n#\n\nb");
    }

    #[test]
    fn extra_breaks_before_a_heading_end_the_block_before() {
        let h = |text: &str| {
            format!(r#"<span style="font-weight: bold; font-size: 22pt;">{text}</span>"#)
        };
        // At the end of the heading before, the breaks show at the size around them.
        assert_eq!(
            md(&format!("{}<br/><br/><br/>{}", h("a"), h("b"))),
            "# a<br/>\n\n# b"
        );
        assert_eq!(md(&format!("p<br/><br/><br/>{}", h("b"))), "p<br/>\n\n# b");
        assert_eq!(
            md_drop(&format!("{}<br/><br/><br/>{}", h("a"), h("b"))),
            "# a\n\n# b"
        );
    }

    #[test]
    fn sizes_around_a_heading_line_are_its_size() {
        // The text's own size makes the line a heading; a relative size around it is
        // overridden.
        let html = r#"<b><small><span style="font-size: 11pt;">&gt;</span></small></b>"#;
        assert_eq!(md(html), "##### \\>");
        assert_eq!(md_drop(html), "##### \\>");
    }

    #[test]
    fn a_task_marker_in_a_heading_stays_in_the_heading() {
        let html = r#"<ul><li><span style="font-weight: bold; font-size: 14pt;">☐ heading</span></li></ul>"#;
        assert_eq!(md(html), "- ### ☐ heading");
        assert_eq!(md_drop(html), "- ### ☐ heading");
        // Breaks between the marker and the heading stay after the marker.
        let html = r#"<ol><li>☐ <br/><span style="font-weight: bold; font-size: 18pt;">x</span></li></ol>"#;
        assert_eq!(md(html), "1. ☐<br/>\n   \n   ## x");
        assert_eq!(md_drop(html), "1. ## ☐ x");
        let html = r#"<ol><li>☐ <br/><br/><ul><li>x</li></ul></li></ol>"#;
        assert_eq!(md(html), "1. ☐<br/><br/>\n   - x");
    }
}

mod inline {
    use super::*;

    #[test]
    fn styles_become_emphasis() {
        for (style, markdown) in [
            ("font-weight: bold;", "**x**"),
            ("font-style: italic;", "*x*"),
            ("font-family: 'Courier New', Courier, monospace;", "`x`"),
            ("text-decoration: line-through;", "~~x~~"),
            ("text-decoration: underline;", "++x++"),
            ("background-color: #FFFF00;", "==x=="),
        ] {
            let html = format!(r#"<span style="{style}">x</span>"#);
            assert_eq!(md(&html), markdown, "{style}");
            assert_eq!(md_drop(&html), markdown, "{style}");
        }
    }

    #[test]
    fn yellow_backgrounds_are_highlights() {
        for color in ["#FFFF00", "#FF0", "yellow", "rgb(255, 255, 0)"] {
            let html = format!(r#"<span style="background-color: {color};">x</span>"#);
            assert_eq!(md(&html), "==x==", "{color}");
        }
    }

    #[test]
    fn superscript_and_subscript() {
        assert_eq!(md("X<sup>2</sup>"), "X^2^");
        assert_eq!(md("X<sub>2</sub>"), "X~2~");
    }

    #[test]
    fn semantic_elements_become_the_emphasis_of_their_style() {
        for (tag, markdown) in [
            ("b", "**x**"),
            ("strong", "**x**"),
            ("i", "*x*"),
            ("em", "*x*"),
            ("u", "++x++"),
            ("s", "~~x~~"),
            ("del", "~~x~~"),
            ("mark", "==x=="),
            ("code", "`x`"),
        ] {
            assert_eq!(md(&format!("<{tag}>x</{tag}>")), markdown, "<{tag}>");
        }
    }

    #[test]
    fn normal_weight_is_plain_text() {
        assert_eq!(
            md(r#"<span style="font-weight: normal;">text</span>"#),
            "text"
        );
    }

    #[test]
    fn colors_stay_raw_spans_in_keep_mode() {
        for html in [
            r#"<span style="color: #FF0000;">text</span>"#,
            r#"<span style="background-color: #FF0000;">text</span>"#,
        ] {
            let markdown = md(html);
            assert_eq!(markdown, html);
            assert_eq!(html_of(&markdown, &keep()), html);
            assert_eq!(md_drop(html), "text", "{html}");
        }
    }

    #[test]
    fn font_families_other_than_courier_are_no_code() {
        for (family, decoded) in [
            (
                "Arial, Helvetica, sans-serif",
                "Arial, Helvetica, sans-serif",
            ),
            ("Georgia, serif", "Georgia, serif"),
            (
                "'Times New Roman', Times, serif",
                "'Times New Roman', Times, serif",
            ),
            ("Verdana, Geneva, sans-serif", "Verdana, Geneva, sans-serif"),
            (
                "&#39;Segoe UI&#39;, Selawik, &#39;Open Sans&#39;, sans-serif",
                "'Segoe UI', Selawik, 'Open Sans', sans-serif",
            ),
        ] {
            let html = format!(r#"<span style="font-family: {family};">text</span>"#);
            let markdown = md(&html);
            assert!(!markdown.contains('`'), "{family}: {markdown}");
            assert!(
                markdown.starts_with("<span style=\"") && markdown.ends_with(">text</span>"),
                "{family}: {markdown}"
            );
            // The browser sees the same family.
            let style = first_span_style(&html_of(&markdown, &keep())).unwrap_or_default();
            assert!(
                style.contains(&format!("font-family: {decoded}")),
                "{family}: {style}"
            );
            assert_eq!(md_drop(&html), "text", "{family}");
        }
    }

    #[test]
    fn courier_written_with_character_references_is_code() {
        assert_eq!(
            md(
                r#"<span style="font-family: &#39;Courier New&#39;, Courier, monospace;">mono</span>"#
            ),
            "`mono`"
        );
    }

    #[test]
    fn links_become_markdown_links() {
        assert_eq!(
            md(r#"<a href="http://example.com" target="_top" class="descriptionLink">Link</a>"#),
            "[Link](http://example.com)"
        );
        assert_eq!(
            md(r#"<a href="http://example.com">Link</a>"#),
            "[Link](http://example.com)"
        );
        assert_eq!(
            md(r#"<a href="u" title="T" target="_top" class="descriptionLink">x</a>"#),
            "[x](u \"T\")"
        );
    }

    const WORK_ITEM_URL: &str =
        "https://polarion.example.com/polarion/#/project/PROJ/workitem?id=PROJ-7";

    #[test]
    fn links_to_a_work_item_labelled_with_its_id_are_bare_ids() {
        assert_eq!(
            md(&format!(r#"<a href="{WORK_ITEM_URL}">PROJ-7</a>"#)),
            "PROJ-7"
        );
        assert_eq!(
            md(&format!(
                r#"<a href="{WORK_ITEM_URL}" target="_top" class="descriptionLink">PROJ-7</a>"#
            )),
            "PROJ-7"
        );
        assert_eq!(
            md(&format!(r#"See <a href="{WORK_ITEM_URL}"></a>."#)),
            "See PROJ-7."
        );
        // Another label is kept.
        assert_eq!(
            md(&format!(r#"<a href="{WORK_ITEM_URL}">the spec</a>"#)),
            format!("[the spec]({WORK_ITEM_URL})")
        );
    }

    #[test]
    fn links_to_a_work_item_stay_links_where_its_id_would_not_be_linked() {
        let html = format!(r#"<a href="{WORK_ITEM_URL}">PROJ-7</a>"#);
        let link = format!(
            r#"<a href="{WORK_ITEM_URL}" target="_top" class="descriptionLink">PROJ-7</a>"#
        );
        for options in [
            ids(WorkItemIds::Off, RawHtml::Keep),
            ids(WorkItemIds::Prefixes(vec!["MP".into()]), RawHtml::Keep),
            ids(WorkItemIds::Off, RawHtml::Drop),
        ] {
            let markdown = md_with(&html, &options);
            assert!(
                markdown.starts_with("[PROJ") && markdown.ends_with(&format!("]({WORK_ITEM_URL})")),
                "{options:?}: {markdown}"
            );
            assert_eq!(html_of(&markdown, &options), link, "{options:?}");
        }
        // No boundary before it: the bare id would not be linked.
        let markdown = md(&format!(r#"x<a href="{WORK_ITEM_URL}">PROJ-7</a>"#));
        assert!(markdown.starts_with("x[PROJ"), "{markdown}");
    }

    #[test]
    fn whitespace_collapses_and_no_break_spaces_stay() {
        assert_eq!(md("a   b\n\t c"), "a b c");
        assert_eq!(md("  a  "), "a");
        assert_eq!(md("a&nbsp;&nbsp;b"), "a\u{a0}\u{a0}b");
        assert_eq!(html_of("a\u{a0}\u{a0}b", &keep()), "a&nbsp;&nbsp;b");
    }

    #[test]
    fn whitespace_moves_out_of_emphasis_and_raw_wrappers() {
        assert_eq!(
            md(r#"x<span style="font-weight: bold;"> a </span>y"#),
            "x **a** y"
        );
        assert_eq!(
            md(r#"x<span style="color: red;"> a </span>y"#),
            r#"x <span style="color: red;">a</span> y"#
        );
    }

    #[test]
    fn empty_emphasis_disappears() {
        assert_eq!(md(r#"a<span style="font-weight: bold;"></span>b"#), "ab");
        assert_eq!(md("a<b> </b>b"), "a b");
        assert_eq!(
            md(
                r#"a<span style="font-style: italic;"><span style="font-weight: bold;"></span></span>b"#
            ),
            "ab"
        );
    }

    #[test]
    fn spans_without_style_are_transparent() {
        assert_eq!(md("a<span>b</span>c"), "abc");
        assert_eq!(md(r#"a<span class="other">b</span>c"#), "abc");
    }

    #[test]
    fn forward_inline_formatting_comes_back_exactly() {
        for markdown in [
            "**Bold**",
            "*Italic*",
            "`code`",
            "~~x~~",
            "++x++",
            "==x==",
            "X^2^",
            "H~2~O",
            "x^a b^",
            "**a** **b**",
            "x **a** y",
            "~~a `b` c~~",
            "[Link](http://example.com)",
            "[x](u \"T\")",
            "[`x`](u)",
            "[PROJ-1](u)",
            "<https://a.b/c>",
            "<a@b.cd>",
            "[www.example.com](http://www.example.com)",
            "``a`b``",
            "a\\\nb",
            "x\\\\",
            "a \\* b",
            "snake\\_case",
            "a\u{a0}b",
            "See PROJ-123 for details",
            "**PROJ-1**",
        ] {
            round_trips(markdown);
        }
        round_trips_with("x <span style=\"color: red;\">a</span> y", &keep());
    }

    /// Formatting inside formatting of another kind, links inside formatting and formatting
    /// inside links come back nested as they were: the HTML nests the spans in that order, and
    /// the outer one is not always bold.
    #[test]
    fn nested_formatting_comes_back_exactly() {
        for markdown in [
            "**a *b* c**",
            "*a **b** c*",
            "==a **b** c==",
            "~~a *b* c~~",
            "++a ==b== c++",
            "**a ~~b~~**",
            "[*a*](u)",
            "[*a* b](u)",
            "*[a](u) b*",
            "**[a](u)**",
            "**`c` and $x$**",
            "*a PROJ-1 b*",
        ] {
            round_trips(markdown);
        }
    }

    #[test]
    fn bold_right_around_italic_comes_back_as_markdown() {
        // `***x***` reads back as italic around bold, which looks the same.
        for html in [
            "<b><i>xy</i></b>",
            "<strong><em>xy</em></strong>",
            r#"<span style="font-weight: bold;"><span style="font-style: italic;">xy</span></span>"#,
            "<i><b>xy</b></i>",
        ] {
            assert_eq!(md(html), "***xy***", "{html}");
            assert_eq!(md_drop(html), "***xy***", "{html}");
        }
        assert_eq!(
            md("<strong><em>bold italic</em></strong> text"),
            "***bold italic*** text"
        );
        assert_eq!(
            md("x <b><i>word word</i></b> rest"),
            "x ***word word*** rest"
        );
        assert_eq!(
            md(r#"<a href="http://x"><b><i>a</i></b></a>"#),
            "[***a***](http://x)"
        );
        round_trips("***a***");
        // With other formatting between them the nesting stays.
        round_trips("**++*x*++**");
        round_trips("***a* b**");
    }

    #[test]
    fn emphasis_starting_or_ending_with_plus_or_equals_stays_markdown() {
        // comrak looks through `+` and `=` for the character next to a delimiter run.
        assert_eq!(md("<b>+ a</b>"), "**\\+ a**");
        assert_eq!(md("<s>+</s>"), "~~\\+~~");
        assert_eq!(md("<s>=</s>"), "~~\\=~~");
        assert_eq!(md("<u>=</u>"), "++\\=++");
        assert_eq!(md("<b>a +</b>"), "**a \\+**");
        for html in [
            "<mark>+</mark>",
            "<i>+ ß</i>",
            "<em>+ a@b.c</em>",
            "<b>+    x y</b>",
            "<sup>+ /</sup>",
            "<sub>+ </sub>",
            "x <b>+</b>",
        ] {
            for markdown in [md(html), md_drop(html)] {
                assert!(raw_tags(&markdown).is_empty(), "{html}: {markdown}");
            }
        }
    }

    #[test]
    fn delimiters_next_to_marks_and_format_characters_are_judged_as_comrak_does() {
        // A soft hyphen, a zero-width space or a combining mark is neither punctuation nor
        // whitespace: these need no fallback.
        for html in [
            "<b>)</b>&shy;",
            "<s>)</s>&shy;",
            "<b>(a)</b>&shy;x",
            "<b>\"a\"</b>&#8203;x",
            "x&#8203;<b>\"a\"</b>",
            "e&#x301;<em>!</em>",
            "e&#x301;<b>$</b>",
            "e&#x301;<b>*</b>",
            "<b>a</b>&shy;b",
        ] {
            md(html);
            md_drop(html);
        }
        assert_eq!(md("<b>a</b>&shy;b"), "**a**\u{ad}b");
    }

    #[test]
    fn an_escaped_tilde_next_to_strikethrough_joins_no_delimiter() {
        round_trips("~~a\\~b~~\\~");
        assert_eq!(md("<s>~</s>"), "~~\\~~~");
    }

    #[test]
    fn a_link_whose_text_is_its_url_with_angle_brackets_is_no_autolink() {
        let html = r#"<a href="https://e.com/a&lt;b">https://e.com/a&lt;b</a>"#;
        assert_eq!(md(html), "[https://e.com/a\\<b](https://e.com/a\\<b)");
        assert_eq!(md_drop(html), "[https://e.com/a\\<b](https://e.com/a\\<b)");
        assert_eq!(
            md(r#"<a href="https://e.com/?a=1&amp;b=2">https://e.com/?a=1&amp;b=2</a>"#),
            "<https://e.com/?a=1&b=2>"
        );
    }

    #[test]
    fn font_size_attributes_and_size_elements_stay_raw_sizes() {
        assert_eq!(
            md(r#"<font size="5">s</font>"#),
            r#"<span style="font-size: x-large;">s</span>"#
        );
        assert_eq!(
            md(r#"<font size="-1">s</font>"#),
            r#"<span style="font-size: small;">s</span>"#
        );
        assert_eq!(
            md("<big>b</big> <small>s</small>"),
            r#"<span style="font-size: larger;">b</span> <span style="font-size: smaller;">s</span>"#
        );
        assert_eq!(md_drop(r#"<font size="5">s</font> <big>b</big>"#), "s b");
    }

    #[test]
    fn hidden_content_stays_hidden() {
        let html = r#"<span style="display: none;">hidden</span>visible"#;
        assert_eq!(md(html), html);
        assert_eq!(md_drop(html), "visible");
        assert_eq!(
            md(r#"<ul><li style="display: none;">x</li><li>y</li></ul>"#),
            "- y"
        );
        assert_eq!(md(r#"<ul style="display: none;"><li>x</li></ul>y"#), "y");
        assert_eq!(md("<p hidden>x</p>y"), "y");
        assert_eq!(md("<span hidden>x</span>y"), "y");
    }
}

mod spans {
    use super::*;

    #[test]
    fn long_work_item_links_are_bare_ids() {
        assert_eq!(md(&wi("PROJ-16026")), "PROJ-16026");
        assert_eq!(md_drop(&wi("PROJ-16026")), "PROJ-16026");
        assert_eq!(
            md(&format!("See {} for details", wi("PROJ-1"))),
            "See PROJ-1 for details"
        );
        let options = ids(WorkItemIds::Prefixes(vec!["PROJ".into()]), RawHtml::Keep);
        assert_eq!(md_with(&wi("PROJ-1"), &options), "PROJ-1");
    }

    #[test]
    fn links_the_forward_setting_would_not_make_are_raw_or_escaped() {
        let span = wi("PROJ-16026");
        for work_items in [
            WorkItemIds::Off,
            WorkItemIds::Prefixes(vec!["MP".into()]),
            WorkItemIds::Prefixes(vec![]),
        ] {
            let keep = md_with(&span, &ids(work_items.clone(), RawHtml::Keep));
            assert_eq!(keep, span, "{work_items:?}");
            assert_eq!(
                html_of(&keep, &ids(work_items.clone(), RawHtml::Keep)),
                span
            );
            assert_eq!(
                md_with(&span, &ids(work_items.clone(), RawHtml::Drop)),
                "PROJ\\-16026",
                "{work_items:?}"
            );
        }
    }

    /// The bare id is written only where the forward detector links it: elsewhere the span stays
    /// raw (keep mode) or becomes text written as that text alone would be (drop mode).
    #[test]
    fn a_link_becomes_a_bare_id_only_where_the_forward_pass_links_it() {
        let span = wi("PROJ-1");
        for (before, after) in [("(", ")"), ("", "."), ("x ", ", y"), ("“", "”"), ("", "")] {
            let html = format!("{before}{span}{after}");
            assert_eq!(md(&html), format!("{before}PROJ-1{after}"), "{html}");
        }
        for (before, after) in [("x", ""), ("", "5"), ("", ".5"), ("", "a"), ("x.", "")] {
            let html = format!("{before}{span}{after}");
            assert_eq!(md(&html), html, "{html}");
            assert_eq!(
                md_drop(&html),
                md_drop(&format!("{before}PROJ-1{after}")),
                "{html}"
            );
        }
    }

    #[test]
    fn adjacent_links_stay_links() {
        let html = format!("{}{}", wi("PROJ-1"), wi("PROJ-2"));
        assert_eq!(html_of(&md(&html), &keep()), html);
    }

    /// Link spans the forward direction never writes: other label options, other projects,
    /// revisions, cross references.
    const VARIANTS: [&str; 6] = [
        r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-16026" data-option-id="short"></span>"#,
        r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-16026" data-option-id="custom" data-custom-label="the spec"></span>"#,
        r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-16026" data-option-id="default"></span>"#,
        r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-scope="OTHER" data-item-id="PROJ-16026" data-option-id="long"></span>"#,
        r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-16026" data-revision="1234" data-option-id="long"></span>"#,
        r#"<span class="polarion-rte-link" data-type="crossReference" id="fake" data-item-id="PROJ-16026" data-option-id="long"></span>"#,
    ];

    /// Links to pages and URLs.
    const PAGE_LINKS: [&str; 2] = [
        r#"<span class="polarion-rte-link" data-type="richPage" id="fake" data-item-name="Home" data-space-name="_default" data-option-id="long"></span>"#,
        r#"<span class="polarion-rte-link" data-type="polarion" id="fake" data-url="https://example.com/x" data-option-id="long"></span>"#,
    ];

    #[test]
    fn other_link_variants_stay_raw_in_keep_mode() {
        for span in VARIANTS.iter().chain(&PAGE_LINKS) {
            let markdown = md(span);
            assert_eq!(markdown, *span);
            assert_eq!(html_of(&markdown, &keep()), *span);
        }
    }

    #[test]
    fn other_link_variants_are_their_id_in_drop_mode() {
        for span in VARIANTS {
            assert_eq!(md_drop(span), "PROJ\\-16026", "{span}");
        }
        for span in PAGE_LINKS {
            let markdown = md_drop(span);
            assert!(raw_tags(&markdown).is_empty(), "{span}: {markdown}");
        }
    }

    #[test]
    fn formula_spans_are_dollar_math() {
        assert_eq!(
            md(r#"<span class="polarion-rte-formula" data-source="\sqrt{x}"></span>"#),
            r"$\sqrt{x}$"
        );
        assert_eq!(
            md(
                r#"<span data-source="\sqrt{Block}" data-inline="false" class="polarion-rte-formula"></span>"#
            ),
            r"$$\sqrt{Block}$$"
        );
        assert_eq!(
            md(r#"<span class="polarion-rte-formula" data-source="a &amp; b"></span>"#),
            "$a & b$"
        );
    }

    #[test]
    fn formulas_dollars_cannot_delimit_are_code_math() {
        assert_eq!(md(&format!("{}2", formula("x", false))), "$`x`$2");
        assert_eq!(md(&formula("a$b", false)), "$`a$b`$");
        // Edge spaces: code math, written so that the spaces survive.
        let html = formula(" x ", false);
        let markdown = md(&html);
        assert!(
            markdown.starts_with("$`") && markdown.ends_with("`$"),
            "{markdown}"
        );
        assert_eq!(html_of(&markdown, &keep()), html);
    }

    #[test]
    fn multi_line_display_formulas_alone_are_math_blocks() {
        assert_eq!(md(&formula("a\nb", true)), "```math\na\nb\n```");
        assert_eq!(
            md(
                r#"<span data-source="a &amp; b&#10;c" data-inline="false" class="polarion-rte-formula"></span>"#
            ),
            "```math\na & b\nc\n```"
        );
        // Break tags written as text in a source are part of the source: kept exactly.
        for br in ["<br>", "<br/>"] {
            assert_eq!(
                md(&formula(&format!("a{br}b"), true)),
                format!("$$a{br}b$$"),
                "{br}"
            );
        }
    }

    #[test]
    fn formulas_markdown_cannot_hold_stay_raw_spans_in_both_modes() {
        let cases = [
            // A line break in an inline formula.
            (formula("a\nb", false), "a\nb", false),
            // A multi-line display formula next to text.
            (format!("x {}", formula("a\nb", true)), "a\nb", true),
            // `` `$ `` where code math is needed (a digit follows).
            (format!("{}1", formula("a`$b", false)), "a`$b", false),
        ];
        for (html, source, display) in cases {
            for options in [keep(), drop_html()] {
                let markdown = md_with(&html, &options);
                assert!(
                    markdown.contains("polarion-rte-formula") && !markdown.contains('\n'),
                    "{html} ({options:?}): {markdown}"
                );
                assert_eq!(
                    markdown.contains("&#10;"),
                    source.contains('\n'),
                    "{html} ({options:?}): {markdown}"
                );
                assert_eq!(
                    formula_in(&html_of(&markdown, &options)),
                    Some((source.to_owned(), display)),
                    "{html} ({options:?}): {markdown}"
                );
            }
        }
    }

    #[test]
    fn forward_formulas_come_back_exactly() {
        for markdown in [
            "$x$",
            "$$x$$",
            r"$\sqrt{x}$ and $$\int_0^1$$",
            "```math\na\nb\n```",
            "$`x`$2",
            "$`a$b`$",
            "$a & b$",
        ] {
            round_trips(markdown);
        }
    }

    #[test]
    fn formulas_in_rte_paragraphs() {
        let html = format!(
            "&nbsp;{}&nbsp;<br/>{}&nbsp;and more",
            formula(r"\sqrt{x}", true),
            formula("a^2", false)
        );
        let markdown = md(&html);
        assert!(
            markdown.contains(r"$$\sqrt{x}$$") && markdown.contains("$a^2$\u{a0}and more"),
            "{markdown}"
        );
    }

    #[test]
    fn content_inside_link_and_formula_spans_is_not_shown() {
        // Polarion shows these widgets from their attributes; a label rendered into the span
        // is not written again.
        let labelled = r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-option-id="long">PROJ-1 - Title</span>"#;
        assert_eq!(md(labelled), "PROJ-1");
        assert_eq!(md_drop(labelled), "PROJ-1");
        let short = r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-option-id="short">PROJ-1</span>"#;
        assert_eq!(
            md(short),
            r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-option-id="short"></span>"#
        );
        assert_eq!(md_drop(short), "PROJ\\-1");
        let formula = r#"<span class="polarion-rte-formula" data-source="x"><b>inside</b></span>"#;
        assert_eq!(md(formula), "$x$");
        assert_eq!(md_drop(formula), "$x$");
    }

    #[test]
    fn empty_work_item_anchors_stay_links_where_their_id_is_not_linked() {
        let html = r#"<a href="https://x/polarion/#/project/P/workitem?id=MP-1"></a> and <a href="https://x/polarion/#/project/P/workitem?id=PROJ-2"></a>"#;
        for raw_html in [RawHtml::Keep, RawHtml::Drop] {
            assert_eq!(
                md_with(html, &ids(WorkItemIds::Any, raw_html)),
                "MP-1 and PROJ-2"
            );
            assert_eq!(
                md_with(
                    html,
                    &ids(WorkItemIds::Prefixes(vec!["MP".into()]), raw_html)
                ),
                "MP-1 and [](https://x/polarion/#/project/P/workitem?id=PROJ-2)"
            );
            assert_eq!(
                md_with(html, &ids(WorkItemIds::Off, raw_html)),
                "[](https://x/polarion/#/project/P/workitem?id=MP-1) and [](https://x/polarion/#/project/P/workitem?id=PROJ-2)"
            );
        }
    }

    #[test]
    fn link_spans_naming_only_a_url_are_links_in_drop_mode() {
        let html =
            r#"<span class="polarion-rte-link" data-type="polarion" data-url="http://x/y"></span>"#;
        assert_eq!(md(html), html);
        assert_eq!(md_drop(html), "<http://x/y>");
        assert_eq!(md_drop(&format!("see {html} now")), "see <http://x/y> now");
        // Inside a link its URL is text.
        assert_eq!(
            md_drop(&format!(r#"<a href="http://a">a {html}</a>"#)),
            "[a http://x/y](http://a)"
        );
    }

    #[test]
    fn formulas_with_a_pipe_in_table_cells_stay_raw_spans() {
        for (source, display) in [("\\|x\\|_2", false), ("|x|", false), ("\\|x\\|", true)] {
            let html = polarion_table(&["Norm"], &[&[&formula(source, display)]]);
            for markdown in [md(&html), md_drop(&html)] {
                assert!(markdown.starts_with("| Norm |\n"), "{markdown}");
                assert!(markdown.contains("&#124;"), "{markdown}");
                assert_eq!(
                    formula_in(&html_of(&markdown, &keep())),
                    Some((source.to_owned(), display)),
                    "{markdown}"
                );
            }
        }
        // Outside tables the formula stays Markdown.
        assert_eq!(md(&formula("\\|x\\|_2", false)), "$\\|x\\|_2$");
    }

    #[test]
    fn carriage_returns_in_formulas_are_line_breaks() {
        let display = r#"<span data-source="a&#13;b" data-inline="false" class="polarion-rte-formula"></span>"#;
        assert_eq!(md(display), "```math\na\nb\n```");
        assert_eq!(md_drop(display), "```math\na\nb\n```");
        let inline = r#"x <span data-source="a&#13;&#10;b" class="polarion-rte-formula"></span>"#;
        for markdown in [md(inline), md_drop(inline)] {
            assert!(!markdown.contains('\r'), "{markdown:?}");
            assert_eq!(
                formula_in(&html_of(&markdown, &keep())).map(|(s, _)| s.replace('\r', "")),
                Some("a\nb".to_owned())
            );
        }
    }

    #[test]
    fn alert_titles_with_markup_and_references_come_back() {
        let alert = |title: &str| {
            format!(
                r##"<div style="margin-left: 40px; border-left: 3px solid #0969da; padding-left: 10px;"><span style="font-weight: bold; color: #0969da;">{title}</span><br/>body</div>"##
            )
        };
        let html = alert("&lt;b&gt; &amp;amp;");
        assert_eq!(md(&html), "> [!NOTE] <b> &amp;amp;\n> body");
        assert_eq!(md_drop(&html), "> [!NOTE] <b> &amp;amp;\n> body");
        assert_eq!(md(&alert("a\\*b &amp; c")), "> [!NOTE] a\\\\*b & c\n> body");
        round_trips("> [!WARNING] a & b\n> body");
    }

    #[test]
    fn alerts_whose_title_is_not_text_stay_their_div() {
        let html = format!(
            r##"<div style="margin-left: 40px; border-left: 3px solid #0969da; padding-left: 10px;"><span style="font-weight: bold; color: #0969da;">{}</span><br/>body</div>"##,
            wi("PROJ-7")
        );
        let markdown = md(&html);
        assert!(markdown.starts_with("<div style="), "{markdown}");
        assert!(markdown.contains("PROJ-7"), "{markdown}");
        assert_eq!(md_drop(&html), "**PROJ-7**\\\nbody");
    }
}

mod code {
    use super::*;

    #[test]
    fn an_indented_monospace_line_is_quoted_inline_code() {
        assert_eq!(
            md(&format!("{QUOTE}{MONO}let x = 1;</span></div>")),
            "> `let x = 1;`"
        );
    }

    /// Older converters wrote code blocks as an indented monospace span with breaks.
    #[test]
    fn an_indented_monospace_span_with_breaks_is_a_code_block() {
        assert_eq!(
            md(&format!(
                "{QUOTE}{MONO}line1<br/>line2<br/>line3</span></div>"
            )),
            "```\nline1\nline2\nline3\n```"
        );
    }

    #[test]
    fn pre_and_code_are_a_fenced_block() {
        let markdown = md("<pre><code>let x = 1;</code></pre>");
        assert_eq!(markdown, "```\nlet x = 1;\n```");
        assert!(!markdown.contains("<pre") && !markdown.contains("<code"));
    }

    #[test]
    fn the_language_comes_from_the_class_or_data_lang() {
        for html in [
            r#"<pre><code class="language-rust">let x = 1;</code></pre>"#,
            r#"<pre class="language-rust">let x = 1;</pre>"#,
            r#"<pre class="lang-rust">let x = 1;</pre>"#,
            r#"<pre data-lang="rust">let x = 1;</pre>"#,
        ] {
            assert_eq!(md(html), "```rust\nlet x = 1;\n```", "{html}");
        }
    }

    #[test]
    fn highlighted_forward_output_is_plain_code_again() {
        let markdown = "```rust\nfn main() {\n    let s = \"<&>\";\n}\n```";
        let html = to_html(markdown, &ToHtmlOptions::default());
        let back = md(&html);
        assert_eq!(back, markdown);
        assert!(!back.contains("color:"), "{back}");
    }

    #[test]
    fn spans_in_code_are_stripped() {
        assert_eq!(
            md(
                r#"<pre><code><span style="color:Blue;">let</span> x = <span style="color:DarkCyan;">1</span>;</code></pre>"#
            ),
            "```\nlet x = 1;\n```"
        );
    }

    #[test]
    fn a_highlighters_wrapper_div_is_transparent() {
        let markdown = md(
            r#"<div style="color:#000000;background-color:#FFFFFF;"><pre><span style="color:#0000FF;">let</span> x = <span>1</span>;</pre></div>"#,
        );
        assert_eq!(markdown, "```\nlet x = 1;\n```");
    }

    #[test]
    fn breaks_in_pre_are_line_breaks() {
        assert_eq!(md("<pre>a<br>b<br/>c</pre>"), "```\na\nb\nc\n```");
    }

    #[test]
    fn mermaid_pre_is_a_mermaid_fence() {
        assert_eq!(
            md("<pre class=\"language-mermaid\">graph TD;\n    A--&gt;B;</pre>"),
            "```mermaid\ngraph TD;\n    A-->B;\n```"
        );
        round_trips("```mermaid\ngraph TD\n    A-->B\n```");
    }

    #[test]
    fn forward_code_blocks_come_back_exactly() {
        for markdown in [
            "```\nline1\nline2\n```",
            "```rust\nlet x = 1;\n```",
            "```\n<a> & b\n```",
            "````\n```\n````",
            "```\n\nx\n```",
            "```\nx\n\n```",
            "```text\nPROJ-1 **not bold** $x$\n```",
            "```\na\n```\n\n```\nb\n```",
        ] {
            round_trips(markdown);
        }
    }

    #[test]
    fn carriage_returns_in_code_are_line_breaks() {
        assert_eq!(md("<pre>a&#13;b</pre>"), "```\na\nb\n```");
        assert_eq!(
            md("<pre>line1&#13;&#10;line2</pre>"),
            "```\nline1\nline2\n```"
        );
        assert_eq!(md_drop("<pre>a&#13;b</pre>"), "```\na\nb\n```");
        // In raw HTML it is written as a reference.
        assert_eq!(
            md(r#"<pre class="x" style="color: red;">a&#13;b</pre>"#),
            "```\na\nb\n```"
        );
    }

    #[test]
    fn characters_xml_does_not_allow_in_a_code_language_are_replaced() {
        let markdown = md(r#"<pre class="language-a&#1;b">x</pre>"#);
        assert_eq!(markdown, "```a\u{fffd}b\nx\n```");
    }

    #[test]
    fn code_holding_every_backtick_run_comrak_reads_is_no_code_span() {
        // A code span needs a backtick run its code lacks, of at most 80.
        let code: String = (1..=80).map(|n| "`".repeat(n) + "x").collect();
        let html = format!("<code>{code}</code>");
        let keep = md(&html);
        assert!(keep.starts_with(MONO), "{keep}");
        let dropped = md_drop(&html);
        assert!(raw_tags(&dropped).is_empty(), "{dropped}");
        // Runs 1 to 79: an 80 long run is free.
        let code: String = (1..80).map(|n| "`".repeat(n) + "x").collect();
        assert!(md(&format!("<code>{code}</code>")).starts_with(&"`".repeat(80)));
    }
}

mod diagrams {
    use super::*;

    const SOURCE: &str = "graph TD\n    A-->B";
    const FENCE: &str = "```mermaid\ngraph TD\n    A-->B\n```";

    fn file() -> String {
        mermaid::diagram(SOURCE).file_name
    }

    fn id() -> String {
        format!("1-{}", file())
    }

    fn sources() -> DiagramSources {
        [(id(), SOURCE.to_owned())].into_iter().collect()
    }

    fn attachment_image() -> String {
        format!(
            r#"<img src="workitemimg:{}" title="{}" style="max-width: 650px;"/>"#,
            id(),
            file()
        )
    }

    fn ink(payload: &str) -> String {
        format!(r#"<img src="https://mermaid.ink/img/{payload}" style="max-width: 650px;" />"#)
    }

    #[test]
    fn attachment_images_with_a_known_source_are_mermaid_fences() {
        let html = attachment_image();
        assert_eq!(convert(&html, &keep(), &sources()), FENCE);
        assert_eq!(convert(&html, &drop_html(), &sources()), FENCE);
    }

    #[test]
    fn attachment_images_without_a_source_stay_images() {
        assert_eq!(
            md(&attachment_image()),
            format!("![](workitemimg:{} \"{}\")", id(), file())
        );
    }

    #[test]
    fn attachment_urls_and_encoded_ids_find_their_sources() {
        let html = format!(
            r#"<img src="https://polarion.example.com/polarion/wi-attachment/PROJ/PROJ-1/{}" style="max-width: 650px;"/>"#,
            id()
        );
        assert_eq!(convert(&html, &keep(), &sources()), FENCE);
        let spaced: DiagramSources = [("2-my diagram.svg".to_owned(), SOURCE.to_owned())]
            .into_iter()
            .collect();
        let html = r#"<img src="workitemimg:2-my%20diagram.svg" style="max-width: 650px;"/>"#;
        assert_eq!(convert(html, &keep(), &spaced), FENCE);
    }

    /// The legacy format: the source in base64 in a mermaid.ink URL, plain or in the live
    /// editor's JSON.
    #[test]
    fn legacy_mermaid_ink_images_are_mermaid_fences() {
        use base64::Engine as _;
        use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
        let json = r#"{"code":"graph TD\n    A-->B","mermaid":{"theme":"default"}}"#;
        for payload in [
            STANDARD.encode(SOURCE),
            URL_SAFE_NO_PAD.encode(SOURCE),
            STANDARD.encode(json),
            format!("base64:{}", STANDARD.encode(SOURCE)),
        ] {
            assert_eq!(md(&ink(&payload)), FENCE, "{payload}");
        }
    }

    #[test]
    fn mermaid_ink_urls_that_do_not_decode_stay_images() {
        assert_eq!(
            md(&ink("pako:eNpLz")),
            "![](https://mermaid.ink/img/pako:eNpLz)"
        );
    }

    #[test]
    fn plain_images_are_markdown_images() {
        let markdown = md(r#"<img src="https://example.com/image.png" alt="test" />"#);
        assert_eq!(markdown, "![test](https://example.com/image.png)");
        assert!(!markdown.contains("mermaid"));
        assert_eq!(
            md(r#"<img src="workitemimg:1-b.png"/>"#),
            "![](workitemimg:1-b.png)"
        );
        // `data-lastmodified` is dropped, whatever the attribute order.
        for html in [
            r#"<img src="workitemimg:1-a.png" title="a.png" style="max-width: 650px;" data-lastmodified="2025-01-01T08:00:00.000+01:00"/>"#,
            r#"<img title="a.png" src="workitemimg:1-a.png" style="max-width: 650px;" data-lastmodified="2025-01-01T08:00:00.000+01:00"/>"#,
        ] {
            assert_eq!(md(html), r#"![](workitemimg:1-a.png "a.png")"#, "{html}");
        }
    }

    #[test]
    fn sized_images_stay_raw_in_keep_mode() {
        let html = r#"<img src="workitemimg:2-c.jpg" style="width: 100px;height: 74px;">"#;
        let markdown = md(html);
        assert!(
            markdown.starts_with("<img ")
                && markdown.contains(r#"src="workitemimg:2-c.jpg""#)
                && markdown.contains(r#"style="width: 100px;height: 74px;""#),
            "{markdown}"
        );
        assert_eq!(md_drop(html), "![](workitemimg:2-c.jpg)");
    }

    /// An image line followed by a caption line: one paragraph with a line break, which does not
    /// turn into an HTML block that swallows the caption.
    #[test]
    fn an_image_and_its_caption_line_stay_together() {
        let html =
            r#"<img src="workitemimg:1-a.png" style="max-width: 650px;"/><br/>Figure 1: caption"#;
        let markdown = md(html);
        assert_eq!(markdown, "![](workitemimg:1-a.png)\\\nFigure 1: caption");
        assert_eq!(html_of(&markdown, &keep()), html);
        let html = r#"<img src="workitemimg:2-c.jpg" style="width: 100px;height: 74px;"><br/>Figure 1: caption"#;
        let back = html_of(&md(html), &keep());
        assert!(
            back.starts_with("<img ") && back.ends_with("><br/>Figure 1: caption"),
            "{back}"
        );
    }

    #[test]
    fn diagrams_round_trip_through_their_attachments() {
        let markdown = "```mermaid\ngraph TD;\n    A-->B;\n```";
        let diagram = mermaid::diagram("graph TD;\n    A-->B;");
        let id = format!("1-{}", diagram.file_name);
        let attachments: DiagramAttachments = [(diagram.file_name.clone(), id.clone())]
            .into_iter()
            .collect();
        let sources: DiagramSources = [(id, diagram.source.clone())].into_iter().collect();
        let html = to_html_with_diagrams(markdown, &ToHtmlOptions::default(), &attachments);
        assert!(
            html.starts_with("<img src=\"workitemimg:1-mermaid-"),
            "{html}"
        );
        let back = convert(&html, &keep(), &sources);
        assert_eq!(back, markdown);
        assert_eq!(
            to_html_with_diagrams(&back, &ToHtmlOptions::default(), &attachments),
            html
        );
    }

    #[test]
    fn forward_images_come_back_exactly() {
        for markdown in [
            "![](a.png)",
            "![alt](a.png \"T\")",
            "![](workitemimg:1-a.png \"a.png\")",
            "![test](https://example.com/image.png)",
            "![](workitemimg:1-a.png)\\\nFigure 1: caption",
        ] {
            round_trips(markdown);
        }
    }

    #[test]
    fn image_placeholders_are_text() {
        assert_eq!(md("&lt;Image: diagram.svg&gt;"), "\\<Image: diagram.svg\\>");
    }
}

mod blocks {
    use super::*;

    #[test]
    fn breaks_separate_paragraphs_and_lines() {
        assert_eq!(md("Para1<br/><br/>Para2"), "Para1\n\nPara2");
        assert_eq!(md("a<br/>b"), "a\\\nb");
        assert_eq!(md("a<br>b"), "a\\\nb");
        assert_eq!(md("a</br>b"), "a\\\nb");
        assert_eq!(md("a<br><br/>b"), "a\n\nb");
        assert_eq!(md("a <br/> <br/> b"), "a\n\nb");
        assert_eq!(md("<p>a</p><p>b</p>"), "a\n\nb");
    }

    #[test]
    fn extra_breaks_are_kept_in_keep_mode() {
        for html in ["a<br/><br/><br/>b", "a<br/><br/><br/><br/>b"] {
            let markdown = md(html);
            assert!(markdown.contains("<br/>"), "{markdown}");
            assert_eq!(html_of(&markdown, &keep()), html);
            assert_eq!(md_drop(html), "a\n\nb");
        }
    }

    #[test]
    fn lists() {
        assert_eq!(
            md("<ol><li>Ordered</li><li>List</li></ol>"),
            "1. Ordered\n2. List"
        );
        assert_eq!(md("<ul><li>a</li><li>b</li></ul>"), "- a\n- b");
        assert_eq!(
            md(r#"<ol start="3"><li>a</li><li>b</li></ol>"#),
            "3. a\n4. b"
        );
        assert_eq!(md("<ul><li>a<ul><li>b</li></ul></li></ul>"), "- a\n  - b");
        assert_eq!(
            md("<ol>\n  <li>One</li>\n  <li>Two</li>\n</ol>"),
            "1. One\n2. Two"
        );
    }

    #[test]
    fn task_symbols_make_task_items() {
        assert_eq!(
            md("<ul><li>☐ open</li><li>☑ done</li><li>☒ crossed</li></ul>"),
            "- [ ] open\n- [x] done\n- [x] crossed"
        );
        assert_eq!(md("<ul><li>☐&nbsp;open</li></ul>"), "- [ ] open");
        assert_eq!(md("<ol><li>☐ a</li></ol>"), "1. [ ] a");
        assert_eq!(md("<ul><li>☐ a</li><li>b</li></ul>"), "- [ ] a\n- b");
    }

    /// A tight list would merge an item's blocks.
    #[test]
    fn items_with_several_blocks_make_loose_lists() {
        assert_eq!(md("<ul><li>a<br/><br/>b</li></ul>"), "- a\n  \n  b");
        round_trips("- a\n  \n  b");
        round_trips("- a\n  \n  ```\n  c\n  ```\n  \n  b");
        round_trips("- a\n  \n  > q");
    }

    #[test]
    fn forward_lists_come_back_exactly() {
        for markdown in [
            "- a\n- b",
            "1. a\n2. b",
            "3. a\n4. b",
            "- [ ] open\n- [x] done",
            "1. [ ] a",
            "- [ ] a\n- b",
            "- a\n  - b",
            "1. a\n2. b\n   - c",
            "- **a** PROJ-1",
            "- a\n\n<!-- end list -->\n\n- b",
            "- a\n\n<!-- end list -->\n\n```\nc\n```",
            "- a\n- b\n\n-----\n\n1. x",
        ] {
            round_trips(markdown);
        }
    }

    #[test]
    fn indent_divs_are_quotes() {
        assert_eq!(
            md(r#"<div style="margin-left: 40px;">Indented<br/> </div>"#),
            "> Indented"
        );
        assert_eq!(
            md(r#"<div style="margin-left: 80px;">Indented</div>"#),
            "> > Indented"
        );
        assert_eq!(md(r#"<div style="margin-left: 120px;">x</div>"#), "> > > x");
        assert_eq!(
            md("<div style=\"margin-left: 40px;\">\n  Indented<br/> \n</div>\n <br/>After"),
            "> Indented\n\nAfter"
        );
    }

    #[test]
    fn forward_quotes_come_back_exactly() {
        for markdown in [
            "> a",
            "> a\\\n> b",
            "> a\n> \n> b",
            "> > a",
            "> a\n> \n> > b",
            "> - a",
            "> `let x = 1;`",
        ] {
            round_trips(markdown);
        }
    }

    #[test]
    fn forward_alerts_come_back_exactly() {
        for kind in ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"] {
            round_trips(&format!("> [!{kind}]\n> body"));
        }
        round_trips("> [!NOTE]\n> a\n> \n> - b");
        // A directive writes the same HTML.
        assert_eq!(
            md(&html_of(":::warning\nx\n:::", &keep())),
            "> [!WARNING]\n> x"
        );
    }

    #[test]
    fn styled_divs_stay_raw_in_keep_mode() {
        for align in ["center", "right"] {
            let open = format!(r#"<div style="text-align: {align};">"#);
            let forward = format!("{open}Align {align}</div>");
            for html in [
                format!("{open}Align {align}<br/> </div>"),
                format!("{open}\n  Align {align}<br/> \n</div>"),
                forward.clone(),
            ] {
                let markdown = md(&html);
                assert_eq!(
                    markdown,
                    format!("{open}\n\nAlign {align}\n\n</div>"),
                    "{html}"
                );
                assert_eq!(html_of(&markdown, &keep()), forward, "{html}");
                assert_eq!(md_drop(&html), format!("Align {align}"), "{html}");
            }
        }
    }

    #[test]
    fn a_centered_div_holding_a_table_keeps_both() {
        let markdown = "<div style=\"text-align: center;\">\n\n| h |\n| --- |\n| c |\n\n</div>";
        round_trips_with(markdown, &keep());
        assert_eq!(
            md_drop(&html_of(markdown, &keep())),
            "| h |\n| --- |\n| c |"
        );
    }

    #[test]
    fn plain_divs_are_transparent() {
        assert_eq!(md("<div>a</div>"), "a");
        assert_eq!(md_drop("<div>a</div>"), "a");
    }

    #[test]
    fn rules() {
        assert_eq!(md("<hr/>"), "-----");
        assert_eq!(md("<hr>"), "-----");
        assert_eq!(md("a<br/><br/><hr/><br/>b"), "a\n\n-----\n\nb");
        for markdown in [
            "-----",
            "-----\n\n-----",
            "a\n\n-----\n\nb",
            "a\n\nb",
            "a\\\nb",
        ] {
            round_trips(markdown);
        }
    }

    #[test]
    fn polarion_tables_are_gfm_tables() {
        assert_eq!(
            md(&polarion_table(&["H1", "H2"], &[&["C1", "C2"]])),
            "| H1 | H2 |\n| --- | --- |\n| C1 | C2 |"
        );
        // The editor's styles and indentation.
        assert_eq!(
            md(&rte_table(&["H1", "H2"], &[&["C1", "C2"]])),
            "| H1 | H2 |\n| --- | --- |\n| C1 | C2 |"
        );
        for markdown in [
            "| H1 | H2 |\n| --- | --- |\n| C1 | C2 |",
            "| a | b |\n| :-: | --: |\n| 1 | 2 |",
            "| a | b |\n| --- | --- |\n| x |  |",
            "| **B** | `c` |\n| --- | --- |\n| PROJ-1 | $x$ |",
            "| h |\n| --- |\n| a\\|b |",
            "| h |\n| --- |",
        ] {
            round_trips(markdown);
        }
    }

    #[test]
    fn breaks_and_sized_images_in_cells_keep_a_gfm_table() {
        round_trips_with("| h |\n| --- |\n| a<br/>b |", &keep());
        let html = polarion_table(
            &["h"],
            &[&[r#"<img src="workitemimg:2-c.jpg" style="width: 100px;height: 74px;">"#]],
        );
        let markdown = md(&html);
        assert!(
            markdown.starts_with("| h |\n| --- |\n| <img "),
            "{markdown}"
        );
        assert_eq!(
            md_drop(&html),
            "| h |\n| --- |\n| ![](workitemimg:2-c.jpg) |"
        );
    }

    /// Tables GFM cannot hold: merged cells, block content, no header row, a row wider than the
    /// header, a data cell aligned unlike its header, cell styles of their own.
    fn odd_tables() -> Vec<String> {
        let td_center = TD.replace("text-align: left", "text-align: center");
        let th_red = TH.replace("#F0F0F0", "#FF0000");
        vec![
            format!(
                r#"{TABLE_OPEN}<tr>{TH}a</th>{TH}b</th></tr><tr><td colspan="2" style="{TD_STYLE}">x</td></tr>{TABLE_CLOSE}"#
            ),
            format!(
                r#"{TABLE_OPEN}<tr>{TH}a</th>{TH}b</th></tr><tr><td rowspan="2" style="{TD_STYLE}">x</td>{TD}y</td></tr><tr>{TD}z</td></tr>{TABLE_CLOSE}"#
            ),
            format!(
                "{TABLE_OPEN}<tr>{TH}h</th></tr><tr>{TD}<ul><li>x</li></ul></td></tr>{TABLE_CLOSE}"
            ),
            format!(
                "{TABLE_OPEN}<tr>{TD}a</td>{TD}b</td></tr><tr>{TD}1</td>{TD}2</td></tr>{TABLE_CLOSE}"
            ),
            format!("{TABLE_OPEN}<tr>{TH}a</th></tr><tr>{TD}1</td>{TD}2</td></tr>{TABLE_CLOSE}"),
            format!("{TABLE_OPEN}<tr>{TH}a</th></tr><tr>{td_center}1</td></tr>{TABLE_CLOSE}"),
            format!("{TABLE_OPEN}<tr>{th_red}a</th></tr><tr>{TD}1</td></tr>{TABLE_CLOSE}"),
        ]
    }

    #[test]
    fn tables_gfm_cannot_hold_stay_raw_in_keep_mode() {
        let mut tables = odd_tables();
        // Merged cells in the editor's indented layout.
        tables.push(
            rte_table(&["a", "b"], &[&["x"]]).replace("<td style", "<td colspan=\"2\" style"),
        );
        for html in tables {
            let markdown = md(&html);
            assert!(
                markdown.starts_with("<table") && markdown.ends_with("</table>"),
                "{markdown}"
            );
            assert!(
                !markdown.lines().any(|line| line.trim().is_empty()),
                "a blank line ends the HTML block: {markdown}"
            );
            assert_eq!(html_of(&markdown, &keep()), markdown);
        }
    }

    #[test]
    fn tables_gfm_cannot_hold_become_gfm_tables_in_drop_mode() {
        let tables = odd_tables();
        assert_eq!(md_drop(&tables[0]), "| a | b |\n| --- | --- |\n| x |  |");
        assert_eq!(md_drop(&tables[2]), "| h |\n| --- |\n| x |");
        assert_eq!(md_drop(&tables[3]), "| a | b |\n| --- | --- |\n| 1 | 2 |");
        for html in &tables {
            let markdown = md_drop(html);
            assert!(
                markdown.starts_with("| ") && raw_tags(&markdown).is_empty(),
                "{markdown}"
            );
        }
    }

    #[test]
    fn caption_paragraphs_stay_raw_in_keep_mode() {
        let html = "<p class=\"polarion-rte-caption-paragraph\">\n  Figure <span data-sequence=\"Figure\" class=\"polarion-rte-caption\">#</span> caption text\n</p>";
        let markdown = md(html);
        assert_eq!(markdown, html);
        assert_eq!(html_of(&markdown, &keep()), markdown);
        assert_eq!(md_drop(html), "Figure \\# caption text");
    }

    #[test]
    fn comments_and_filtered_elements_are_dropped() {
        for html in [
            "a<!-- note -->b",
            "a<script>alert(1)</script>b",
            "a<style>p { color: red; }</style>b",
            "a<iframe src=\"https://example.com\"></iframe>b",
            "a<object data=\"x.swf\"></object>b",
            "a<embed src=\"x.swf\">b",
            "a<noscript>n</noscript>b",
            "a<textarea>t</textarea>b",
            "a<title>t</title>b",
            "a<xmp>x</xmp>b",
            "a<noembed>n</noembed>b",
            "a<noframes>n</noframes>b",
            "a<base href=\"https://example.com/\">b",
            "a<link rel=\"stylesheet\" href=\"x.css\">b",
            "a<meta charset=\"utf-8\">b",
            "a<frameset><frame src=\"x\"></frameset>b",
        ] {
            assert_eq!(md(html), "ab", "{html}");
            assert_eq!(md_drop(html), "ab", "{html}");
        }
        // Everything after `<plaintext>` is its content.
        assert_eq!(md("a<plaintext>b <i>c</i>"), "a");
    }

    #[test]
    fn unknown_elements_are_transparent() {
        assert_eq!(md("<foo>x</foo>"), "x");
        assert_eq!(md("a <custom-tag>text</custom-tag> b"), "a text b");
    }

    #[test]
    fn block_styles_of_lists_do_not_become_spans() {
        for html in [
            r#"<ul style="margin-left: 20px;"><li>a</li></ul>"#,
            r#"<ul><li style="text-align: center;">a</li></ul>"#,
            r#"<ul><li style="background-color: #EEEEEE;">a</li></ul>"#,
            r#"<ul style="list-style-type: square;"><li>a</li></ul>"#,
        ] {
            assert_eq!(md(html), "- a", "{html}");
        }
        // Styles the text inherits stay.
        assert_eq!(
            md(r#"<ul style="color: #FF0000;"><li>a</li></ul>"#),
            r#"- <span style="color: #FF0000;">a</span>"#
        );
        assert_eq!(
            md(r#"<ol><li style="font-weight: bold;">a</li></ol>"#),
            "1. **a**"
        );
    }

    #[test]
    fn content_between_list_items_is_a_line_of_the_item_before() {
        assert_eq!(
            md("<ul><li>a</li>text<li>b</li></ul>"),
            "- a\\\n  text\n- b"
        );
        assert_eq!(md("<ol><li>a</li>tail</ol>"), "1. a\\\n   tail");
        assert_eq!(
            md("<ul><li>a</li><b>bold</b><li>b</li></ul>"),
            "- a\\\n  **bold**\n- b"
        );
        assert_eq!(md("<ul>text<li>b</li></ul>"), "- text\n- b");
    }

    #[test]
    fn block_elements_without_margins_end_lines() {
        assert_eq!(md("<div>a</div><div>b</div>"), "a\\\nb");
        assert_eq!(md_drop("<div>a</div><div>b</div>"), "a\\\nb");
        assert_eq!(md("a<div>b</div>c"), "a\\\nb\\\nc");
        assert_eq!(md("<div>a<br/></div><div>b</div>"), "a\\\nb");
        assert_eq!(md("<div>a</div><br/><div>b</div>"), "a\n\nb");
        // Elements with margins end paragraphs.
        assert_eq!(md("<p>a</p><p>b</p>"), "a\n\nb");
        assert_eq!(md("<p>a</p>b"), "a\n\nb");
        assert_eq!(
            md_drop(
                r#"<div style="text-align: center;">a</div><div style="text-align: right;">b</div>"#
            ),
            "a\\\nb"
        );
    }

    #[test]
    fn extra_breaks_before_boxes_are_kept_where_the_separators_leave_room() {
        assert_eq!(md("<br/><br/><hr/>"), "<br/>\n\n-----");
        assert_eq!(md("<br/><br/><pre>x</pre>"), "<br/>\n\n```\nx\n```");
        assert_eq!(
            md("<hr/><br/><br/><br/><br/><hr/>"),
            "-----\n\n<br/>\n<br/>\n\n-----"
        );
        assert_eq!(md_drop("<br/><br/><hr/>"), "-----");
        // In a list item no separator stands next to a box.
        let item = md("<ul><li><br/><ul><li>b</li></ul></li></ul>");
        assert_eq!(
            html_of(&item, &keep()),
            "<ul><li><br/><ul><li>b</li></ul></li></ul>"
        );
        assert_eq!(
            md_drop("<ul><li><br/><ul><li>b</li></ul></li></ul>"),
            "- - b"
        );
    }

    #[test]
    fn center_elements_are_centered_divs() {
        assert_eq!(
            md("<center>c</center>"),
            "<div style=\"text-align: center;\">\n\nc\n\n</div>"
        );
        assert_eq!(md_drop("<center>c</center>"), "c");
    }

    #[test]
    fn formatting_around_a_markdown_table_reaches_its_cells() {
        let html = "<b><table><tr><th>H</th></tr><tr><td>C</td></tr></table></b>";
        assert_eq!(md(html), "| **H** |\n| --- |\n| **C** |");
        assert_eq!(md_drop(html), "| **H** |\n| --- |\n| **C** |");
    }

    #[test]
    fn template_content_is_left_out_of_raw_tables() {
        let html = r#"<table><tr><td colspan="2"><template><img src=x onerror=alert(1)></template>t</td></tr></table>"#;
        assert_eq!(
            md(html),
            r#"<table><tbody><tr><td colspan="2">t</td></tr></tbody></table>"#
        );
        assert_eq!(md_drop(html), "| t |  |\n| --- | --- |");
    }

    #[test]
    fn carriage_returns_in_urls_are_left_out() {
        assert_eq!(
            md(r#"<a href="http://e.com/&#13;x">cr</a>"#),
            "[cr](http://e.com/x)"
        );
        assert_eq!(
            md(r#"<img src=" http://e.com/&#10;i.png "/>"#),
            "![](http://e.com/i.png)"
        );
    }
}

mod drop_mode {
    use super::*;

    #[test]
    fn formatting_markdown_cannot_express_is_discarded() {
        for html in [
            r#"<span style="color: #FF0000;">text</span>"#,
            r#"<span style="background-color: #FF0000;">text</span>"#,
            r#"<span style="font-family: Arial, Helvetica, sans-serif;">text</span>"#,
            r#"<span style="font-family: Georgia, serif;">text</span>"#,
            r#"<span style="font-size: 12pt;line-height: 1.5;">text</span>"#,
            r#"<span style="font-size: 11pt;">text</span>"#,
            r#"<span style="color: #FF0000; font-size: 12pt;">text</span>"#,
            r#"<span style="letter-spacing: 2px;">text</span>"#,
        ] {
            assert_eq!(md_drop(html), "text", "{html}");
        }
        // Representable formatting inside stays.
        assert_eq!(
            md_drop(
                r#"<span style="color: #FF0000;"><span style="font-weight: bold;">text</span></span>"#
            ),
            "**text**"
        );
    }

    #[test]
    fn links_the_forward_pass_would_not_make_are_text() {
        let options = ids(WorkItemIds::Off, RawHtml::Drop);
        assert_eq!(md_with(&wi("PROJ-16026"), &options), "PROJ\\-16026");
        assert_eq!(
            md_with(&format!("See {} here", wi("PROJ-1")), &options),
            "See PROJ\\-1 here"
        );
    }

    #[test]
    fn styled_blocks_lose_their_wrappers() {
        assert_eq!(
            md_drop(r#"<div style="text-align: center;">Align Center<br/> </div>"#),
            "Align Center"
        );
        assert_eq!(
            md_drop(r#"<div style="border: 1px solid red;">a<br/><br/>b</div>"#),
            "a\n\nb"
        );
    }

    #[test]
    fn extra_breaks_are_removed() {
        assert_eq!(md_drop("a<br/><br/><br/>b"), "a\n\nb");
        assert_eq!(md_drop("<br/><br/>a"), "a");
    }

    #[test]
    fn representable_formatting_survives() {
        for markdown in [
            "# Title",
            "**b** *i* `c` ~~s~~ ++u++ ==h== x^2^ H~2~O",
            "[Link](http://example.com) PROJ-1",
            "- a\n  - b",
            "| a | b |\n| :-: | --: |\n| 1 | 2 |",
            "> [!TIP]\n> body",
            "```rust\nlet x = 1;\n```",
            "$x$ and $$y$$",
        ] {
            round_trips_with(markdown, &drop_html());
        }
    }

    #[test]
    fn an_rte_document_keeps_no_raw_html() {
        let markdown = md_drop(&rte_document());
        assert!(
            raw_tags(&markdown).is_empty(),
            "{:?}\n{markdown}",
            raw_tags(&markdown)
        );
        for expected in [
            "Intro text of the item.\n\nA second paragraph ending in a space\n\nPROJ-101",
            "PROJ\\-102",
            "**Bold**, *italic*, ++underlined++, X^2^, H~2~O, red and marked.",
            "Segoe Arial `mono` Georgia",
            "Font Size 12",
            "1. First\n2. Second",
            "- Bullet",
            "> Indented",
            "Align Center",
            "[Spec](https://example.com/spec)",
            "| C1 | ![](workitemimg:2-c.jpg) |",
            "![](workitemimg:1-a.png \"a.png\")",
            "Figure \\# caption text",
        ] {
            assert!(markdown.contains(expected), "{expected:?}:\n{markdown}");
        }
    }
}

mod documents {
    use super::*;

    /// The document the behaviors are described with: headings, emphasis, a link span, code,
    /// a link, a font size, formulas, a table.
    fn combined() -> String {
        [
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
            "  <tbody>",
            "    <tr>",
            r#"      <th style="font-weight: bold;background-color: #F0F0F0;text-align: left;border: 1px solid #CCCCCC;padding: 5px;">H1</th>"#,
            r#"      <th style="font-weight: bold;background-color: #F0F0F0;text-align: left;border: 1px solid #CCCCCC;padding: 5px;">H2</th>"#,
            "    </tr>",
            "    <tr>",
            r#"      <td style="text-align: left;border: 1px solid #CCCCCC;padding: 5px;">C1</td>"#,
            r#"      <td style="text-align: left;border: 1px solid #CCCCCC;padding: 5px;">C2</td>"#,
            "    </tr>",
            "  </tbody>",
            "</table>",
            r#"<span style="font-weight: bold; font-size: 14pt; line-height: 1.2;">Sub</span><br/><br/>"#,
            "Final paragraph.",
        ]
        .join("\n")
    }

    #[test]
    fn the_combined_document() {
        let markdown = md(&combined());
        for expected in [
            "# Main Title",
            "## Section",
            "### Sub",
            "## Whole Line Heading",
            "Some **bold** and *italic* text.",
            "`inline code`",
            "[Link](http://example.com)",
            "PROJ-99999",
            r"$\sqrt{x}$",
            r"$$\int_{0}^{1}$$",
            "| H1 | H2 |\n| --- | --- |\n| C1 | C2 |",
            "Final paragraph.",
        ] {
            assert!(markdown.contains(expected), "{expected:?}:\n{markdown}");
        }
        assert!(
            !markdown.contains("polarion-rte-link") && !markdown.contains("PROJ\\-99999"),
            "{markdown}"
        );
        let font_size = markdown
            .lines()
            .find(|line| line.contains("Font Size"))
            .unwrap_or_default();
        assert!(font_size.starts_with("Font Size"), "{markdown}");
        md_drop(&combined());
    }

    #[test]
    fn an_rte_document() {
        let markdown = md(&rte_document());
        for expected in [
            "Intro text of the item.\n\nA second paragraph ending in a space\n\nPROJ-101",
            r#"data-item-id="PROJ-102" data-option-id="short""#,
            r#"**Bold**, *italic*, ++underlined++, X^2^, H~2~O, <span style="color: #FF0000;">red</span> and <span style="background-color: #FF0000;">marked</span>."#,
            "`mono`",
            "1. First\n2. Second",
            "- Bullet",
            "> Indented",
            "<div style=\"text-align: center;\">",
            "[Spec](https://example.com/spec)",
            "\\<Image: diagram.svg\\>",
            r"$$\sqrt{x}$$",
            "$a^2$\u{a0}and more",
            "| H1 | H2 |\n| --- | --- |\n| C1 | <img",
            "![](workitemimg:1-a.png \"a.png\")",
            "polarion-rte-caption-paragraph",
        ] {
            assert!(markdown.contains(expected), "{expected:?}:\n{markdown}");
        }
        for unexpected in ["`Arial`", "`Segoe`", "`Georgia`", "data-lastmodified"] {
            assert!(
                !markdown.contains(unexpected),
                "{unexpected:?}:\n{markdown}"
            );
        }
        assert!(!has_heading_line(&markdown), "{markdown}");
    }

    #[test]
    fn ids_in_plain_text_are_never_links() {
        assert_eq!(
            md(&html_of("See PROJ-123 for details", &keep())),
            "See PROJ-123 for details"
        );
        for work_items in [
            WorkItemIds::Any,
            WorkItemIds::Prefixes(vec!["PROJ".into()]),
            WorkItemIds::Off,
        ] {
            let markdown = md_with(
                "See PROJ-123 for details",
                &ids(work_items.clone(), RawHtml::Keep),
            );
            assert_eq!(markdown, "See PROJ\\-123 for details", "{work_items:?}");
            let html = to_html(&markdown, &ToHtmlOptions::default());
            assert!(!html.contains("polarion-rte-link"), "{html}");
        }
    }

    #[test]
    fn empty_and_blank_input_give_empty_markdown() {
        for html in [
            "",
            " ",
            "\n\t \n",
            "<span></span>",
            "<!-- c -->",
            "<div></div>",
        ] {
            assert_eq!(md(html), "", "{html:?}");
            assert_eq!(md_drop(html), "", "{html:?}");
        }
    }

    #[test]
    fn canonical_documents_come_back_exactly() {
        round_trips(
            "# Title\n\nSome **bold** text.\n\n- a\n- b\n\n| h |\n| --- |\n| c |\n\n> quote\n\n```rust\nlet x = 1;\n```\n\n-----\n\nEnd PROJ-1.",
        );
        round_trips(
            "## Steps\n\n1. [ ] first `step`\n2. [x] second\n   - detail with $x$\n\n> [!WARNING]\n> Mind PROJ-7.\n\n![](workitemimg:1-a.png \"a.png\")\\\nFigure 1: caption",
        );
    }

    #[test]
    fn the_conversion_functions_agree() {
        let html = rte_document();
        for options in [keep(), drop_html()] {
            let markdown = to_markdown(&html, &options);
            assert_eq!(
                to_markdown_with_diagrams(&html, &options, &DiagramSources::new()),
                markdown
            );
            assert_eq!(
                to_markdown_report(&html, &options, &DiagramSources::new()).markdown,
                markdown
            );
        }
    }

    #[test]
    fn odd_input_converts_to_a_fixpoint() {
        let inputs = [
            "<",
            ">",
            "</",
            "<>",
            "</>",
            "<<<>>>",
            "<b>unclosed",
            "<span style=\"color: red;\">unclosed",
            "a</span>b",
            "</div>x",
            "<b><i>x</b>y</i>",
            "<div><span>a</div>b</span>",
            "<table><tr><td>x",
            "<ul><li>a<li>b",
            "<li>x</li>",
            "<td>x</td>",
            "<tr><td>x</td></tr>",
            "<pre>",
            "<pre></pre>",
            "<pre>\n\n</pre>",
            "<a href=\"x\">",
            "<a>x</a>",
            "<a href=\"\">x</a>",
            "<img>",
            "<img src=\"\">",
            "&#0;",
            "&#xD800;",
            "&#99999999;",
            "&bogus;",
            "&",
            "&amp",
            "a\u{0}b",
            "<![CDATA[x]]>",
            "<?php x ?>",
            "<!DOCTYPE html>",
            "<!---->",
            "<!-- open",
            "<html><body>x</body></html>",
            "<head><title>t</title></head>x",
            "<svg><text>x</text></svg>",
            "<math><mi>x</mi></math>",
            "<select><option>x</option></select>",
            "<form><input value=\"x\"></form>",
            "<template>x</template>",
            "<details><summary>s</summary>d</details>",
            "<h1>Heading</h1>",
            "<h2>x</h2><h3>y</h3>",
            "<blockquote>q</blockquote>",
            "<dl><dt>t</dt><dd>d</dd></dl>",
            "<span style=\"\">x</span>",
            "<span style=\"font-weight\">x</span>",
            "<span style=\";;;\">x</span>",
            "<span style=\"font-size: abcpt;\">x</span>",
            "<span style=\"font-size: 1e40pt;\">x</span>",
            "<span style=\"font-size: -5pt;\">x</span>",
            "<span style=\"font-size: NaNpt;\">x</span>",
            "<span style=\"font-size: 0pt;\">x</span>",
            "<div style=\"margin-left: -40px;\">x</div>",
            "<div style=\"margin-left: 1e9px;\">x</div>",
            "<div style=\"margin-left: 99999999999px;\">x</div>",
            "<ol start=\"-5\"><li>x</li></ol>",
            "<ol start=\"1000000000\"><li>x</li></ol>",
            "<ol start=\"99999999999999999999\"><li>x</li></ol>",
            "<ol start=\"abc\"><li>x</li></ol>",
            "<table><tr><th colspan=\"0\">x</th></tr></table>",
            "<span class=\"polarion-rte-link\" data-type=\"workItem\" data-option-id=\"long\"></span>",
            "<span class=\"polarion-rte-link\" data-type=\"workItem\" data-item-id=\"\" data-option-id=\"long\"></span>",
            "<span class=\"polarion-rte-link\" data-type=\"workItem\" data-item-id=\"not an id\" data-option-id=\"long\"></span>",
            "<span class=\"polarion-rte-formula\"></span>",
            "<span class=\"polarion-rte-formula\" data-source=\"\"></span>",
            "<span class=\"polarion-rte-formula\" data-source=\"$\"></span>",
            "<span class=\"polarion-rte-formula\" data-source=\"`\"></span>",
            "<span class=\"polarion-rte-formula\" data-source=\"$$\" data-inline=\"false\"></span>",
            "<span class=\"polarion-rte-formula\" data-source=\"a&#10;&#10;b\" data-inline=\"false\"></span>",
            "<span style=\"font-family: 'Courier New', Courier, monospace;\"> </span>",
            "<span style=\"font-family: 'Courier New', Courier, monospace;\">`</span>",
            "<span style=\"font-family: 'Courier New', Courier, monospace;\">a<br/><br/>b</span>",
            "<a href=\"u\"><a href=\"v\">x</a></a>",
            "<sup><sup>x</sup></sup>",
            "<b><b><b>x</b></b></b>",
            "x<br/>",
            "<br/>",
            "<br/><br/><br/>",
            "&nbsp;",
            "&nbsp;<br/>&nbsp;",
            "<p class=\"x\"></p>",
            "<div style=\"text-align: center;\"></div>",
        ];
        each_within(&inputs, std::time::Duration::from_secs(30), |html| {
            for options in [keep(), drop_html()] {
                lenient(html, &options);
            }
        });
    }

    /// The forward direction replaces characters XML does not allow, so the Markdown must not
    /// depend on them either.
    #[test]
    fn characters_xml_does_not_allow_still_give_a_fixpoint() {
        let inputs = [
            "a\u{1}b",
            "a\u{b}b",
            "a\u{c}b",
            "a\u{1b}[31mred",
            "a\u{fffe}b",
            "a\u{ffff}b",
            "<span style=\"color: red;\">\u{1}</span>",
            "<pre>\u{1b}[0m</pre>",
            "<span data-source=\"\u{1}\" class=\"polarion-rte-formula\"></span>",
            "<img src=\"workitemimg:\u{1}.png\">",
            "<a href=\"u\u{1}\">x</a>",
            "<span class=\"polarion-rte-link\" data-type=\"workItem\" id=\"fake\" data-item-id=\"PROJ-1\u{1}\" data-option-id=\"short\"></span>",
        ];
        each_within(&inputs, std::time::Duration::from_secs(30), |html| {
            for options in [keep(), drop_html()] {
                lenient(html, &options);
            }
        });
    }

    #[test]
    fn long_documents_convert() {
        let started = std::time::Instant::now();
        let markdown = "| a | b |\n| --- | --- |\n".to_owned() + &"| 1 | PROJ-2 |\n".repeat(1_000);
        let back = md(&html_of(&markdown, &keep()));
        assert_eq!(back, markdown.trim_end());
        let html = format!("{} ", wi("PROJ-1")).repeat(3_000);
        assert_eq!(md(&html).matches("PROJ-1").count(), 3_000);
        let html = "Paragraph<br/><br/>".repeat(2_000);
        assert_eq!(md(&html).matches("Paragraph").count(), 2_000);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(120),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn unicode_whitespace_alone_comes_back() {
        for options in [keep(), drop_html()] {
            for (html, markdown) in [
                ("&#x3000;", "\u{3000}"),
                ("&#x3000;<br/><br/>", "\u{3000}"),
                ("&#x2003;", "\u{2003}"),
            ] {
                let got = md_with(html, &options);
                if options.raw_html == RawHtml::Keep && html.ends_with("<br/>") {
                    assert_eq!(got, "\u{3000}<br/><br/>");
                } else {
                    assert_eq!(got, markdown, "{html:?}");
                }
            }
        }
    }

    #[test]
    fn a_byte_order_mark_starting_a_line_is_left_out() {
        // comrak drops one starting its input, HTML parsers one starting theirs; it shows
        // nothing.
        assert_eq!(md("&#xFEFF;bom"), "bom");
        assert_eq!(md_drop("&#xFEFF;bom"), "bom");
        assert_eq!(md("a<br/><br/>&#xFEFF;b"), "a\n\nb");
        assert_eq!(md("a&#xFEFF;b"), "a\u{feff}b");
    }

    #[test]
    fn text_nested_past_the_depth_limit_keeps_its_words() {
        let nested =
            |depth: usize| format!("a{}b{}c", "<span>".repeat(depth), "</span>".repeat(depth));
        assert_eq!(md(&nested(64)), "abc");
        assert_eq!(md(&nested(65)), "abc");
        assert_eq!(md(&nested(200)), "abc");
        let spaced = format!("a {}b{} c", "<span>".repeat(80), "</span>".repeat(80));
        assert_eq!(md(&spaced), "a b c");
        let blocks = format!("a{}b{}c", "<div>".repeat(80), "</div>".repeat(80));
        assert_eq!(md(&blocks), "a\\\nb\\\nc");
    }
}
