//! The visual model the round-trip tests compare HTML with (`support/visual.rs`): what must look
//! the same, what must not, and the shape of its dump.

#[path = "support/visual.rs"]
mod visual;

/// Every pair of `variants` looks the same.
fn same(variants: &[&str]) {
    for pair in variants.windows(2) {
        if let Some(difference) = visual::difference(pair[0], pair[1]) {
            panic!(
                "{:?} and {:?} should look the same\n{difference}",
                pair[0], pair[1]
            );
        }
    }
}

/// `a` and `b` look different.
fn differ(a: &str, b: &str) {
    assert!(
        visual::difference(a, b).is_some(),
        "{a:?} and {b:?} should look different, both are:\n{}",
        visual::render(a)
    );
}

const RTE_TABLE: &str = concat!(
    r#"<table id="polarion_wiki macro name=table" class="polarion-Document-table" style="width: 100%;max-width: 1280px;margin-left: 0px;margin-right: auto;border: 1px solid #CCCCCC;empty-cells: show;border-collapse: collapse;max-width: 1280.0px;">"#,
    "\n  <tbody>\n    <tr>\n      ",
    r#"<th style="font-weight: bold;background-color: #F0F0F0;text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;border-top: 1px solid #CCCCCC;border-bottom: 1px solid #CCCCCC;border-right: 1px solid #CCCCCC;border-left: 1px solid #CCCCCC;padding: 5px;">Name</th>"#,
    "\n    </tr>\n    <tr>\n      ",
    r#"<td style="text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;border-top: 1px solid #CCCCCC;border-bottom: 1px solid #CCCCCC;border-right: 1px solid #CCCCCC;border-left: 1px solid #CCCCCC;padding: 5px;">Value</td>"#,
    "\n    </tr>\n  </tbody>\n</table>"
);

const STANDARD_TABLE: &str = concat!(
    r#"<table id="polarion_wiki macro name=table" class="polarion-Document-table" style="width: 100%; max-width: 1280px; margin-left: 0px; margin-right: auto; border: 1px solid #CCCCCC; empty-cells: show; border-collapse: collapse;"><tbody><tr>"#,
    r#"<th style="font-weight: bold; background-color: #F0F0F0; text-align: left; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;">Name</th></tr><tr>"#,
    r#"<td style="text-align: left; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;">Value</td></tr></tbody></table>"#
);

mod looks_the_same {
    use super::*;

    #[test]
    fn break_spellings() {
        same(&["a<br>b", "a<br/>b", "a<br />b", "a</br>b", "a<BR>b"]);
    }

    #[test]
    fn attribute_order_and_style_spelling() {
        same(&[
            r#"<span style="font-weight: bold; color: #FF0000;" class="x">a</span>"#,
            r#"<span class="y" style="color:#ff0000;font-weight:bold">a</span>"#,
            r#"<span style="  COLOR : #FF0000 ; FONT-WEIGHT : BOLD ; ">a</span>"#,
            r#"<span style='font-weight: 700; color: rgb(255, 0, 0)'>a</span>"#,
        ]);
    }

    #[test]
    fn elements_and_styles_of_emphasis() {
        same(&[
            "<b>x</b>",
            "<strong>x</strong>",
            r#"<span style="font-weight: bold;">x</span>"#,
            r#"<span style="font-weight: bolder">x</span>"#,
        ]);
        same(&[
            "<i>x</i>",
            "<em>x</em>",
            r#"<span style="font-style: italic;">x</span>"#,
            r#"<span style="font-style: oblique">x</span>"#,
        ]);
        same(&[
            "<u>x</u>",
            "<ins>x</ins>",
            r#"<span style="text-decoration: underline;">x</span>"#,
            r#"<span style="text-decoration-line: underline">x</span>"#,
        ]);
        same(&[
            "<s>x</s>",
            "<del>x</del>",
            "<strike>x</strike>",
            r#"<span style="text-decoration: line-through;">x</span>"#,
        ]);
        same(&[
            "x<sup>2</sup>",
            r#"x<span style="vertical-align: super;">2</span>"#,
        ]);
        same(&[
            "x<sub>2</sub>",
            r#"x<span style="vertical-align: sub">2</span>"#,
        ]);
    }

    #[test]
    fn spellings_of_the_highlight() {
        same(&[
            "<mark>x</mark>",
            r#"<span style="background-color: #FFFF00;">x</span>"#,
            r#"<span style="background-color: #FF0;">x</span>"#,
            r#"<span style="background-color: yellow;">x</span>"#,
            r#"<span style="background-color: rgb(255, 255, 0);">x</span>"#,
            r#"<span style="background: yellow">x</span>"#,
        ]);
    }

    #[test]
    fn monospace_fonts() {
        same(&[
            "<code>x</code>",
            "<tt>x</tt>",
            r#"<span style="font-family: 'Courier New', Courier, monospace;">x</span>"#,
            r#"<span style="font-family: &#39;Courier New&#39;, Courier, monospace;">x</span>"#,
            r#"<span style="font-family: Consolas">x</span>"#,
        ]);
    }

    #[test]
    fn spellings_of_font_families_and_colours() {
        same(&[
            r#"<span style="font-family: &#39;Segoe UI&#39;, Selawik, sans-serif;">x</span>"#,
            r#"<span style="font-family:'Segoe UI',Selawik,sans-serif">x</span>"#,
            r#"<span style="font-family: &quot;segoe ui&quot; , selawik, SANS-SERIF">x</span>"#,
        ]);
        same(&[
            r#"<span style="color: #F00">x</span>"#,
            r#"<span style="color: red">x</span>"#,
            r#"<span style="color: rgb(255, 0, 0)">x</span>"#,
            r##"<font color="#FF0000">x</font>"##,
        ]);
    }

    #[test]
    fn ten_points_is_the_default_size() {
        same(&["x", r#"<span style="font-size: 10pt;">x</span>"#]);
    }

    #[test]
    fn line_heights() {
        same(&[
            r#"a <span style="font-size: 12pt; line-height: 1.5;">12</span>"#,
            r#"a <span style="font-size: 12pt;">12</span>"#,
            r#"a <span style="font-size: 16px">12</span>"#,
        ]);
    }

    #[test]
    fn collapsing_whitespace() {
        same(&[
            "a b",
            "a   b",
            "a\n\tb",
            "  a b  ",
            "\n<div>\n  a b\n</div>\n",
        ]);
        same(&["<div>a</div><div>b</div>", "<div>a</div>\n \n<div>b</div>"]);
        // The space between two runs keeps the style of neither.
        same(&["<b>a </b>b", "<b>a</b> b", "<b>a</b><span> </span>b"]);
        same(&["a <b> b</b>", "a <b>b</b>"]);
    }

    #[test]
    fn no_break_spaces_look_like_spaces() {
        same(&["a&nbsp;b", "a b", "a\u{a0}b"]);
        same(&["&nbsp;&nbsp;a", "\u{a0}\u{a0}a"]);
        // At the end of a line they are invisible.
        same(&["a&nbsp;<br/>b", "a<br/>b"]);
    }

    #[test]
    fn decorations_reach_every_descendant() {
        same(&[
            r#"<u>a <span style="text-decoration: none;">b</span></u>"#,
            "<u>a b</u>",
        ]);
        same(&[
            r#"<b>a <span style="font-weight: normal;">b</span></b>"#,
            "<b>a</b> b",
        ]);
        same(&[
            r#"<span style="font-weight: normal;">plain<br/><br/>text</span>"#,
            "plain<br/><br/>text",
        ]);
    }

    #[test]
    fn breaks_that_show_nothing() {
        // Before a block, at the end of a block, at the end of the document.
        same(&["a<br/><div>b</div>", "a<div>b</div>"]);
        same(&["<div>a<br/> </div>b", "<div>a</div>b"]);
        same(&["a<br/><br/>", "a<br/>", "a"]);
        same(&["<ul><li>a<br/></li></ul>", "<ul><li>a</li></ul>"]);
    }

    #[test]
    fn paragraph_margins_are_a_blank_line() {
        same(&["<p>a</p><p>b</p>", "a<br/><br/>b", "<p>a</p>b"]);
        same(&["<p>a</p>", "a"]);
    }

    #[test]
    fn a_blank_line_next_to_a_margin_merges_into_it() {
        same(&[
            "<ul><li>a</li></ul><ul><li>b</li></ul>",
            "<ul><li>a</li></ul><br/><ul><li>b</li></ul>",
            "<ul><li>a</li></ul>\n \n<ul><li>b</li></ul>",
        ]);
        same(&[
            "text<ul><li>a</li></ul>",
            "text<br/><br/><ul><li>a</li></ul>",
        ]);
        same(&["<pre>x</pre>after", "<pre>x</pre><br/>after"]);
    }

    #[test]
    fn indentation_adds_up() {
        same(&[
            r#"<div style="margin-left: 40px;"><div style="margin-left: 40px;">a</div></div>"#,
            r#"<div style="margin-left:80px">a</div>"#,
            r#"<div style="margin: 0 0 0 80px;">a</div>"#,
            r#"<div style="padding-left: 80px;">a</div>"#,
            r#"<div style="margin-left: 60pt;">a</div>"#,
        ]);
    }

    #[test]
    fn table_css() {
        same(&[RTE_TABLE, STANDARD_TABLE]);
    }

    #[test]
    fn image_size_limits_and_metadata() {
        same(&[
            r#"<img src="a.png" style="max-width: 650px;"/>"#,
            r#"<img src="a.png">"#,
            r#"<img class="x" src="a.png" data-lastmodified="2025-01-01T08:00:00.000+01:00"/>"#,
            r#"<img src="a.png" alt="" title="">"#,
        ]);
        same(&[
            r#"<img src="a.png" style="width: 100px;height: 74px;">"#,
            r#"<img src="a.png" width="100" height="74"/>"#,
        ]);
    }

    #[test]
    fn code_blocks_by_their_text() {
        same(&[
            concat!(
                r#"<pre class="language-rust" style="margin-left: 40px; font-family: 'Courier New', Courier, monospace;">"#,
                r#"<span style="font-weight:bold;color:#a71d5d;">let</span><span style="color:#323232;"> x = 1;"#,
                "\n</span><span>y</span></pre>"
            ),
            "<pre>let x = 1;\ny</pre>",
            "<pre><code>let x = 1;<br>y</code></pre>",
            r#"<div style="color:#000000;background-color:#FFFFFF;"><pre>let x = 1;<br/>y</pre></div>"#,
            concat!(
                r#"<div style="margin-left: 40px;"><span style="font-family: 'Courier New', Courier, monospace;">"#,
                "let x = 1;<br/>y</span></div>"
            ),
        ]);
        same(&[
            "<pre>a\n&nbsp;&nbsp;b</pre>",
            "<pre>a\n  b</pre>",
            concat!(
                r#"<div style="margin-left: 40px;"><span style="font-family: monospace;">"#,
                "a<br/>&nbsp;&nbsp;b</span></div>"
            ),
        ]);
        // HTML parsers drop one line break right after `<pre>`.
        same(&["<pre>\nx</pre>", "<pre>x</pre>", "<pre>x\n</pre>"]);
    }

    #[test]
    fn hidden_content() {
        same(&[
            concat!(
                "a<!-- note --><script>alert(1)</script><style>p { color: red; }</style>",
                r#"<iframe src="https://example.com"></iframe><object data="x.swf"></object>"#,
                r#"<embed src="y.swf"><noscript>no</noscript><span style="display: none;">gone</span>b"#
            ),
            "ab",
        ]);
    }

    #[test]
    fn heading_elements_are_heading_lines_of_their_level() {
        same(&[
            "<h1>Title</h1>",
            r#"<span style="font-weight: bold; font-size: 22pt; line-height: 1.2;">Title</span>"#,
        ]);
        same(&[
            "<h3>Sub</h3>",
            r#"<span style="font-weight: bold; font-size: 14pt; line-height: 1.2;">Sub</span>"#,
        ]);
        differ("<h1>Title</h1>", "<h2>Title</h2>");
    }

    #[test]
    fn deeply_nested_input_does_not_overflow_the_stack() {
        let deep = "<b>".repeat(20_000) + "x";
        same(&[&deep, "<b>x</b>"]);
    }

    #[test]
    fn heading_lines_by_level() {
        same(&[
            r#"<span style="font-weight: bold; font-size: 18pt; line-height: 1.2;">Title</span>"#,
            r#"<span style="font-weight: bold; font-size: 20pt;">Title</span>"#,
            r#"<span style="font-size: 18pt;">Title</span>"#,
            r#"<b><span style="font-size: 24px;">Title</span></b>"#,
        ]);
        same(&[
            r#"<span style="font-weight: bold; font-size: 10.5pt; line-height: 1.5;">Small</span>"#,
            r#"<span style="font-weight: bold; font-size: 10.5pt;">Small</span>"#,
        ]);
    }

    #[test]
    fn work_item_anchors_labelled_with_their_id() {
        same(&[
            r#"See <a href="https://polarion.example.com/polarion/#/project/PROJ/workitem?id=PROJ-7">PROJ-7</a>."#,
            r#"See <span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-7" data-option-id="long"></span>."#,
            r#"See <span data-option-id="long" data-item-id="PROJ-7" class="polarion-rte-link" data-type="workItem"></span>."#,
        ]);
    }

    #[test]
    fn diagrams_by_their_source() {
        same(&[
            r#"<img src="https://mermaid.ink/img/Z3JhcGggVEQKICAgIEEtLT5C" style="max-width: 650px;" />"#,
            "<pre class=\"language-mermaid\">graph TD\n    A--&gt;B</pre>",
            "<pre><code class=\"language-mermaid\">\ngraph TD   \n    A--&gt;B\n\n</code></pre>",
        ]);
    }

    #[test]
    fn formula_sources_as_written_or_escaped() {
        same(&[
            r#"<span class="polarion-rte-formula" data-source="a&#10;b" data-inline="false"></span>"#,
            "<span data-source=\"a\nb\" data-inline=\"false\" class=\"polarion-rte-formula\"></span>",
            r#"<span data-source="a<br/>b" data-inline="false" class="polarion-rte-formula"></span>"#,
            r#"<span data-source="a&lt;br&gt;b" data-inline="false" class="polarion-rte-formula"></span>"#,
        ]);
        same(&[
            r#"<span data-source=" x " class="polarion-rte-formula"></span>"#,
            r#"<span data-source="x" class="polarion-rte-formula"></span>"#,
        ]);
    }

    #[test]
    fn nothing_but_whitespace() {
        same(&[
            "",
            "   \n ",
            "<span></span>",
            "<div> </div>",
            "<!-- only a comment -->",
        ]);
    }
}

mod looks_different {
    use super::*;

    #[test]
    fn text() {
        differ("a", "b");
        differ("ab", "a b");
        differ("a&nbsp; b", "a b");
    }

    #[test]
    fn text_styles() {
        let plain = "x";
        for styled in [
            "<b>x</b>",
            "<i>x</i>",
            "<u>x</u>",
            "<s>x</s>",
            "<sup>x</sup>",
            "<sub>x</sub>",
            "<code>x</code>",
            "<mark>x</mark>",
            r#"<span style="font-size: 9pt;">x</span>"#,
            r#"<span style="color: #0000FF;">x</span>"#,
            r#"<span style="background-color: #FF0000;">x</span>"#,
            r#"<span style="font-family: Georgia, serif;">x</span>"#,
            r#"<span style="letter-spacing: 2px;">x</span>"#,
            r#"<a href="https://example.com">x</a>"#,
        ] {
            differ(plain, styled);
        }
        differ("<u>x</u>", "<s>x</s>");
        differ("<sup>x</sup>", "<sub>x</sub>");
        differ(
            r#"<span style="font-size: 11pt;">x</span>"#,
            r#"<span style="font-size: 12pt;">x</span>"#,
        );
        differ(
            r#"<span style="font-family: Arial;">x</span>"#,
            r#"<span style="font-family: Georgia;">x</span>"#,
        );
        differ(
            "<code>x</code>",
            r#"<span style="font-family: Arial;">x</span>"#,
        );
        differ(
            r#"<span style="background-color: #FFFF00;">x</span>"#,
            r#"<span style="background-color: #FF0000;">x</span>"#,
        );
        differ(r#"<a href="u">x</a>"#, r#"<a href="v">x</a>"#);
        differ(r#"<a href="u">x</a>"#, r#"<a href="u" title="t">x</a>"#);
    }

    #[test]
    fn heading_levels() {
        differ(
            r#"<span style="font-weight: bold; font-size: 14pt;">T</span>"#,
            r#"<span style="font-weight: bold; font-size: 18pt;">T</span>"#,
        );
        // Word body text in 12pt is no heading.
        differ(
            r#"<span style="font-size: 12pt;">T</span>"#,
            r#"<span style="font-weight: bold; font-size: 12pt;">T</span>"#,
        );
        differ(
            "T",
            r#"<span style="font-weight: bold; font-size: 11pt;">T</span>"#,
        );
    }

    #[test]
    fn line_breaks_and_blank_lines() {
        differ("a<br/>b", "a b");
        differ("a<br/><br/>b", "a<br/>b");
        differ("a<br/><br/><br/>b", "a<br/><br/>b");
        differ("<br/>a", "a");
        differ("<p>a</p><br/><br/><p>b</p>", "<p>a</p><p>b</p>");
        differ(
            "<ul><li>a</li></ul><br/><br/><ul><li>b</li></ul>",
            "<ul><li>a</li></ul><br/><ul><li>b</li></ul>",
        );
        // Tables have no margins of their own.
        differ(
            "a<br/><br/><table><tr><td>x</td></tr></table>",
            "a<table><tr><td>x</td></tr></table>",
        );
    }

    #[test]
    fn indentation_alignment_and_bars() {
        differ(
            r#"<div style="margin-left: 40px;">a</div>"#,
            r#"<div style="margin-left: 80px;">a</div>"#,
        );
        differ(
            r#"<div style="text-align: center;">a</div>"#,
            "<div>a</div>",
        );
        differ(
            r#"<div style="text-align: center;">a</div>"#,
            r#"<div style="text-align: right;">a</div>"#,
        );
        differ(
            r#"<div style="margin-left: 40px; border-left: 3px solid #0969DA; padding-left: 10px;">a</div>"#,
            r#"<div style="margin-left: 53px;">a</div>"#,
        );
        differ(
            r#"<div style="margin-left: 40px; border-left: 3px solid #0969DA; padding-left: 10px;">a</div>"#,
            r#"<div style="margin-left: 40px; border-left: 3px solid #1A7F37; padding-left: 10px;">a</div>"#,
        );
    }

    #[test]
    fn lists() {
        differ("<ul><li>a</li></ul>", "<ol><li>a</li></ol>");
        differ(r#"<ol start="3"><li>a</li></ol>"#, "<ol><li>a</li></ol>");
        differ(
            "<ul><li>a<ul><li>b</li></ul></li></ul>",
            "<ul><li>a</li><li>b</li></ul>",
        );
        differ("<ul><li>a</li><li></li></ul>", "<ul><li>a</li></ul>");
        differ(
            "<ul><li>a<br/><br/>b</li></ul>",
            "<ul><li>a</li><li>b</li></ul>",
        );
        differ(
            r#"<div style="margin-left: 40px;"><ul><li>a</li></ul></div>"#,
            "<ul><li>a</li></ul>",
        );
    }

    #[test]
    fn tables() {
        differ(
            r#"<table><tr><td colspan="2">a</td></tr></table>"#,
            "<table><tr><td>a</td><td></td></tr></table>",
        );
        differ(
            r#"<table><tr><td rowspan="2">a</td></tr></table>"#,
            "<table><tr><td>a</td></tr></table>",
        );
        differ(
            "<table><tr><td>a</td></tr></table>",
            "<table><tr><td>b</td></tr></table>",
        );
        differ(
            "<table><tr><td>a</td></tr></table>",
            "<table><tr><td>a</td></tr><tr><td></td></tr></table>",
        );
        differ(
            "<table><tr><th>a</th></tr></table>",
            "<table><tr><td>a</td></tr></table>",
        );
        differ(
            r#"<table><tr><td style="text-align: right;">a</td></tr></table>"#,
            "<table><tr><td>a</td></tr></table>",
        );
        differ(
            STANDARD_TABLE,
            &STANDARD_TABLE.replace("#F0F0F0", "#FFFFFF"),
        );
    }

    #[test]
    fn objects() {
        differ(r#"<img src="a.png">"#, r#"<img src="b.png">"#);
        differ(r#"<img src="a.png" alt="x">"#, r#"<img src="a.png">"#);
        differ(r#"<img src="a.png" title="x">"#, r#"<img src="a.png">"#);
        differ(
            r#"<img src="a.png" style="width: 100px;">"#,
            r#"<img src="a.png">"#,
        );
        differ(
            r#"<span class="polarion-rte-formula" data-source="a"></span>"#,
            r#"<span class="polarion-rte-formula" data-source="b"></span>"#,
        );
        differ(
            r#"<span class="polarion-rte-formula" data-source="a"></span>"#,
            r#"<span class="polarion-rte-formula" data-source="a" data-inline="false"></span>"#,
        );
        differ(
            r#"<span class="polarion-rte-link" data-type="workItem" data-item-id="PROJ-1" data-option-id="long"></span>"#,
            r#"<span class="polarion-rte-link" data-type="workItem" data-item-id="PROJ-1" data-option-id="short"></span>"#,
        );
        differ(
            r#"<span class="polarion-rte-link" data-type="workItem" data-item-id="PROJ-1" data-option-id="long"></span>"#,
            r#"<span class="polarion-rte-link" data-type="workItem" data-scope="OTHER" data-item-id="PROJ-1" data-option-id="long"></span>"#,
        );
        // A work item anchor with another label is a link.
        differ(
            r#"<a href="https://polarion.example.com/polarion/#/project/PROJ/workitem?id=PROJ-7">the task</a>"#,
            r#"<span class="polarion-rte-link" data-type="workItem" data-item-id="PROJ-7" data-option-id="long"></span>"#,
        );
        differ(
            "<pre class=\"language-mermaid\">graph TD\n    A--&gt;B</pre>",
            "<pre class=\"language-mermaid\">graph TD\n    A--&gt;C</pre>",
        );
        differ(
            "<pre class=\"language-mermaid\">graph TD</pre>",
            "<pre>graph TD</pre>",
        );
    }

    #[test]
    fn whitespace_in_code() {
        differ("<pre>a  b</pre>", "<pre>a b</pre>");
        differ("<pre>  a</pre>", "<pre>a</pre>");
        differ("<pre>a\n\nb</pre>", "<pre>a\nb</pre>");
        differ("<pre>a</pre>", "a");
    }
}

/// The dump the failure messages show.
#[test]
fn model_dump() {
    assert_eq!(
        visual::render(concat!(
            r#"<b>Bold</b> text<br/><br/><div style="margin-left: 40px;">quote</div><br/>"#,
            r#"<ol start="2"><li>two</li></ol><hr/><img src="a.png" alt="A"/>"#
        )),
        concat!(
            "line: \"Bold\"{bold} \" text\"\n",
            "space 1\n",
            "line indent=40: \"quote\"\n",
            "space 1\n",
            "line indent=40 marker=2.: \"two\"\n",
            "space 1\n",
            "rule indent=0\n",
            "space 1\n",
            "line: [image \"a.png\" alt=\"A\"]\n",
        )
    );
    assert_eq!(
        visual::render(STANDARD_TABLE),
        concat!(
            "table indent=0\n",
            "  row\n",
            "    cell background=#f0f0f0\n",
            "      line: \"Name\"{bold}\n",
            "  row\n",
            "    cell\n",
            "      line: \"Value\"\n",
        )
    );
}
