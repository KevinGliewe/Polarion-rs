//! Round trips between Markdown and Polarion HTML, on whole documents.
//!
//! Fixtures (`tests/fixtures/`):
//! - `corpus.md`: canonical Markdown (what comrak's writer produces with the crate's options, plus
//!   the escapes the conversion adds) using every construct both directions map. Converting it to
//!   HTML and back gives it back unchanged, and the HTML is a fixpoint.
//! - `lossy.md`: constructs the round trip cannot keep exactly (footnotes, definition lists,
//!   explicit left alignment, other spellings of the same construct, ...);
//!   `lossy.expected.md` is the Markdown they come back as (a custom alert title comes back as
//!   it was).
//! - `rte_reference.html`: a description as Polarion's rich text editor stores it;
//!   `legacy_output.html`: HTML older Markdown converters wrote (code as an indented monospace
//!   span, a highlighter's wrapper around `pre`, a mermaid.ink image, `pre` with a `code` child);
//!   `samples/*.html`: small documents, one case each.
//! - `golden/NAME.md` and `golden/NAME.drop.md`: the Markdown of each HTML fixture with formatting
//!   Markdown cannot express kept as HTML and dropped. `UPDATE_GOLDEN=1` rewrites them (review
//!   the diff before committing).
//!
//! Every HTML fixture, in both modes: the golden file, no block rebuilt by the self-check, no
//! built tree comrak's `validate()` rejects, and the Markdown is a fixpoint (converting it to HTML
//! and back gives it again). With formatting kept, the HTML of the Markdown looks like the
//! fixture: `support/visual.rs` compares what a reader sees, independently of the crate, and
//! documents what it ignores and why (the conversion rewrites Polarion's table CSS, style
//! spellings and code highlighting, so strings cannot be compared).

#[path = "support/visual.rs"]
mod visual;

use std::fs;
use std::path::{Path, PathBuf};

use polarion_markdown::{
    DiagramAttachments, DiagramSources, RawHtml, ToHtmlOptions, ToMarkdownOptions, WorkItemIds,
    mermaid, to_html, to_html_with_diagrams, to_markdown, to_markdown_report,
    to_markdown_with_diagrams,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// A fixture file (path relative to `tests/fixtures`), line endings as `\n` whatever the checkout
/// made of them.
fn read(relative: &str) -> String {
    let path = fixtures().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// A Markdown fixture without the final line break of the file (the conversion writes none).
fn read_markdown(relative: &str) -> String {
    read(relative).trim_end_matches('\n').to_owned()
}

fn html_options(work_items: &WorkItemIds) -> ToHtmlOptions {
    ToHtmlOptions {
        work_items: work_items.clone(),
        ..ToHtmlOptions::default()
    }
}

/// Where two texts first differ: the line, both versions of it and, in long lines, the column.
fn first_difference(expected: &str, actual: &str) -> String {
    let want: Vec<&str> = expected.split('\n').collect();
    let got: Vec<&str> = actual.split('\n').collect();
    let line = (0..want.len().max(got.len()))
        .find(|&i| want.get(i) != got.get(i))
        .unwrap_or(0);
    let show = |text: Option<&&str>| text.map_or("<end of text>".to_owned(), |t| format!("{t:?}"));
    let mut message = format!(
        "first difference in line {}:\n  expected: {}\n  actual:   {}",
        line + 1,
        show(want.get(line)),
        show(got.get(line))
    );
    if let (Some(a), Some(b)) = (want.get(line), got.get(line))
        && a.len().max(b.len()) > 120
    {
        let column = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
        let excerpt = |s: &str| -> String {
            s.chars()
                .skip(column.saturating_sub(40))
                .take(100)
                .collect()
        };
        message.push_str(&format!(
            "\n  at column {}:\n  expected: …{:?}\n  actual:   …{:?}",
            column + 1,
            excerpt(a),
            excerpt(b)
        ));
    }
    message
}

/// Panics with `context` and the first difference unless the texts are equal.
fn assert_text(context: &str, expected: &str, actual: &str) {
    if expected != actual {
        panic!(
            "{context}\n{}\n--- expected ---\n{expected}\n--- actual ---\n{actual}\n---",
            first_difference(expected, actual)
        );
    }
}

/// HTML with a line break after each `<br/>`, for readable differences.
fn html_lines(html: &str) -> String {
    html.replace("<br/>", "<br/>\n")
}

fn update_golden() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v == "1")
}

/// Compares `markdown` with `golden/{name}{suffix}.md` or, with `UPDATE_GOLDEN=1`, writes it.
fn golden(failures: &mut Vec<String>, name: &str, suffix: &str, markdown: &str) {
    let relative = format!("golden/{name}{suffix}.md");
    let path = fixtures().join(&relative);
    if update_golden() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("cannot create {}: {e}", parent.display()));
        }
        fs::write(&path, format!("{markdown}\n"))
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        return;
    }
    let Ok(stored) = fs::read_to_string(&path) else {
        failures.push(format!(
            "tests/fixtures/{relative} is missing. Create the golden files with \
             `UPDATE_GOLDEN=1 cargo test -p polarion-markdown --test round_trip` \
             (PowerShell: `$env:UPDATE_GOLDEN=1; cargo test -p polarion-markdown --test round_trip`) \
             and review them."
        ));
        return;
    };
    let stored = stored.replace("\r\n", "\n");
    let stored = stored.strip_suffix('\n').unwrap_or(&stored);
    if stored != markdown {
        failures.push(format!(
            "tests/fixtures/{relative} differs from the conversion (expected: the file, actual: \
             the conversion); if the change is intended, rewrite it with UPDATE_GOLDEN=1 and \
             review the diff.\n{}",
            first_difference(stored, markdown)
        ));
    }
}

/// Every check of an HTML fixture (`name` relative to `tests/fixtures`, without `.html`), in
/// both modes, with `work_items` in both directions; all failures are reported together.
fn check_html_fixture(name: &str, work_items: &WorkItemIds) {
    let html = read(&format!("{name}.html"));
    let mut failures = Vec::new();
    for raw_html in [RawHtml::Keep, RawHtml::Drop] {
        let (mode, suffix) = match raw_html {
            RawHtml::Keep => ("formatting kept", ""),
            RawHtml::Drop => ("formatting dropped", ".drop"),
        };
        let context = format!("{name}.html, {mode}");
        let options = ToMarkdownOptions {
            raw_html,
            work_items: work_items.clone(),
        };
        let report = to_markdown_report(&html, &options, &DiagramSources::new());
        let markdown = report.markdown.as_str();
        if report.invalid != 0 {
            failures.push(format!(
                "{context}: {} built comrak trees fail validate()",
                report.invalid
            ));
        }
        if report.fallbacks != 0 {
            failures.push(format!(
                "{context}: {} blocks needed the self-check's conservative rebuild or raw HTML",
                report.fallbacks
            ));
        }
        let back = to_html(markdown, &html_options(work_items));
        let again = to_markdown(&back, &options);
        if again != markdown {
            failures.push(format!(
                "{context}: the Markdown is no fixpoint (expected: the Markdown, actual: the \
                 Markdown converted to HTML and back)\n{}",
                first_difference(markdown, &again)
            ));
        }
        if raw_html == RawHtml::Keep
            && let Some(difference) = visual::difference(&html, &back)
        {
            failures.push(format!(
                "{context}: the Markdown converted back to HTML does not look like the fixture \
                 (expected: the fixture, actual: the HTML of the Markdown)\n{difference}"
            ));
        }
        golden(&mut failures, name, suffix, markdown);
    }
    if !failures.is_empty() {
        panic!("{}", failures.join("\n\n"));
    }
}

#[test]
fn canonical_markdown_survives_the_round_trip() {
    let corpus = read_markdown("corpus.md");
    let html = to_html(&corpus, &ToHtmlOptions::default());
    let report = to_markdown_report(&html, &ToMarkdownOptions::default(), &DiagramSources::new());
    assert_text(
        "tests/fixtures/corpus.md converted to HTML and back (expected: the corpus)",
        &corpus,
        &report.markdown,
    );
    assert_eq!(report.invalid, 0, "corpus: built trees fail validate()");
    assert_eq!(report.fallbacks, 0, "corpus: blocks needed a fallback");
    let again = to_html(&report.markdown, &ToHtmlOptions::default());
    assert_text(
        "corpus: the HTML is no fixpoint (expected: the corpus's HTML, actual: after one more \
         round trip)",
        &html_lines(&html),
        &html_lines(&again),
    );
}

/// comrak's options as the crate sets them (`src/parse.rs`).
fn comrak_options() -> comrak::Options<'static> {
    let mut options = comrak::Options::default();
    let e = &mut options.extension;
    e.strikethrough = true;
    e.table = true;
    e.autolink = true;
    e.tasklist = true;
    e.superscript = true;
    e.subscript = true;
    e.highlight = true;
    e.insert = true;
    e.math_dollars = true;
    e.math_code = true;
    e.footnotes = true;
    e.description_lists = true;
    e.alerts = true;
    e.block_directive = true;
    options.parse.escaped_char_spans = true;
    options.render.prefer_fenced = true;
    options.render.width = 0;
    options
}

/// The corpus keeps every construct the round trip must restore.
#[test]
fn the_corpus_covers_every_construct() {
    use comrak::nodes::{ListType, NodeValue, TableAlignment};

    let corpus = read_markdown("corpus.md");
    let arena = comrak::Arena::new();
    let root = comrak::parse_document(&arena, &corpus, &comrak_options());
    let mut found = std::collections::BTreeSet::new();
    for node in root.descendants() {
        let in_item = node.parent().is_some_and(|p| {
            matches!(
                p.data.borrow().value,
                NodeValue::Item(_) | NodeValue::TaskItem(_)
            )
        });
        let in_quote = node
            .parent()
            .is_some_and(|p| matches!(p.data.borrow().value, NodeValue::BlockQuote));
        let mut labels: Vec<String> = Vec::new();
        match &node.data.borrow().value {
            NodeValue::Heading(heading) => labels.push(format!("heading {}", heading.level)),
            NodeValue::LineBreak => labels.push("hard break".into()),
            NodeValue::Strong => labels.push("strong".into()),
            NodeValue::Emph => labels.push("emphasis".into()),
            NodeValue::Strikethrough => labels.push("strikethrough".into()),
            NodeValue::Insert => labels.push("underline".into()),
            NodeValue::Highlight => labels.push("highlight".into()),
            NodeValue::Superscript => labels.push("superscript".into()),
            NodeValue::Subscript => labels.push("subscript".into()),
            NodeValue::Code(code) if code.literal.contains('`') => {
                labels.push("code span with backticks".into());
            }
            NodeValue::Code(_) => labels.push("code span".into()),
            NodeValue::Link(link) => {
                let label: String = node
                    .children()
                    .filter_map(|c| match &c.data.borrow().value {
                        NodeValue::Text(t) => Some(t.to_string()),
                        _ => None,
                    })
                    .collect();
                labels.push(if !link.title.is_empty() {
                    "link with a title".into()
                } else if label == link.url {
                    "autolink".into()
                } else {
                    "link".into()
                });
            }
            NodeValue::Image(_) => labels.push("image".into()),
            NodeValue::Math(math) if !math.dollar_math => labels.push("code math".into()),
            NodeValue::Math(math) if math.display_math => labels.push("display math".into()),
            NodeValue::Math(_) => labels.push("inline math".into()),
            NodeValue::CodeBlock(code) => labels.push(
                match code.info.as_str() {
                    "" => "code block",
                    "math" => "math block",
                    "mermaid" => "mermaid block",
                    _ => "code block with a language",
                }
                .into(),
            ),
            NodeValue::List(list) => {
                if list.is_task_list {
                    labels.push("task list".into());
                } else if list.list_type == ListType::Ordered {
                    labels.push("ordered list".into());
                } else {
                    labels.push("bullet list".into());
                }
                if list.list_type == ListType::Ordered && list.start != 1 {
                    labels.push("ordered list from another number".into());
                }
                if !list.tight {
                    labels.push("loose list".into());
                }
                if in_item {
                    labels.push("nested list".into());
                }
                if in_quote {
                    labels.push("list in a quote".into());
                }
            }
            NodeValue::TaskItem(task) => labels.push(
                if task.symbol.is_some() {
                    "done task"
                } else {
                    "open task"
                }
                .into(),
            ),
            NodeValue::BlockQuote if in_quote => labels.push("nested quote".into()),
            NodeValue::BlockQuote => labels.push("quote".into()),
            NodeValue::Alert(alert) => labels.push(format!("alert {:?}", alert.alert_type)),
            NodeValue::Table(table) => {
                if table.alignments.contains(&TableAlignment::Center) {
                    labels.push("centred column".into());
                }
                if table.alignments.contains(&TableAlignment::Right) {
                    labels.push("right-aligned column".into());
                }
            }
            NodeValue::ThematicBreak => labels.push("thematic break".into()),
            NodeValue::HtmlInline(_) => labels.push("inline HTML".into()),
            NodeValue::HtmlBlock(_) => labels.push("HTML block".into()),
            NodeValue::Escaped => {
                for child in node.children() {
                    if let NodeValue::Text(t) = &child.data.borrow().value {
                        labels.push(format!("escaped {t}"));
                    }
                }
            }
            NodeValue::Text(text) => {
                if text.contains('\u{a0}') {
                    labels.push("no-break space".into());
                }
                if !text.is_ascii() {
                    labels.push("non-ASCII text".into());
                }
                if text.contains("PROJ-123") {
                    labels.push("work item id".into());
                }
            }
            _ => {}
        }
        found.extend(labels);
    }
    let mut required: Vec<String> = (1..=6).map(|level| format!("heading {level}")).collect();
    required.extend(
        [
            "hard break",
            "strong",
            "emphasis",
            "strikethrough",
            "underline",
            "highlight",
            "superscript",
            "subscript",
            "code span",
            "code span with backticks",
            "link",
            "link with a title",
            "autolink",
            "image",
            "work item id",
            "inline math",
            "display math",
            "code math",
            "math block",
            "code block",
            "code block with a language",
            "mermaid block",
            "bullet list",
            "ordered list",
            "ordered list from another number",
            "task list",
            "open task",
            "done task",
            "nested list",
            "loose list",
            "list in a quote",
            "quote",
            "nested quote",
            "alert Note",
            "alert Tip",
            "alert Important",
            "alert Warning",
            "alert Caution",
            "centred column",
            "right-aligned column",
            "thematic break",
            "inline HTML",
            "HTML block",
            "non-ASCII text",
            "no-break space",
        ]
        .map(str::to_owned),
    );
    required.extend(
        ["$", "^", "=", "+", "&", "|", ".", ":", "-", "*", "_"].map(|c| format!("escaped {c}")),
    );
    let missing: Vec<&String> = required.iter().filter(|r| !found.contains(*r)).collect();
    assert!(
        missing.is_empty(),
        "tests/fixtures/corpus.md lacks {missing:?}"
    );
}

#[test]
fn lossy_constructs_come_back_as_expected() {
    let lossy = read_markdown("lossy.md");
    let expected = read_markdown("lossy.expected.md");
    let report = to_markdown_report(
        &to_html(&lossy, &ToHtmlOptions::default()),
        &ToMarkdownOptions::default(),
        &DiagramSources::new(),
    );
    assert_text(
        "tests/fixtures/lossy.md converted to HTML and back (expected: lossy.expected.md)",
        &expected,
        &report.markdown,
    );
    assert_eq!(report.invalid, 0, "lossy.md: built trees fail validate()");
    assert_eq!(report.fallbacks, 0, "lossy.md: blocks needed a fallback");
    let again = to_markdown(
        &to_html(&expected, &ToHtmlOptions::default()),
        &ToMarkdownOptions::default(),
    );
    assert_text(
        "lossy.expected.md is no fixpoint (expected: the file, actual: after a round trip)",
        &expected,
        &again,
    );
}

#[test]
fn polarion_editor_output() {
    check_html_fixture("rte_reference", &WorkItemIds::Any);
}

#[test]
fn older_converter_output() {
    check_html_fixture("legacy_output", &WorkItemIds::Any);
}

/// `WorkItemIds::Any` unless a setting is given.
macro_rules! work_items {
    () => {
        WorkItemIds::Any
    };
    ($ids:expr) => {
        $ids
    };
}

/// One test per sample (`tests/fixtures/samples/NAME.html`), optionally with the work item
/// setting both directions use, and the list [`SAMPLES`].
macro_rules! samples {
    ($($name:ident $(: $ids:expr)?),* $(,)?) => {
        mod samples {
            use super::*;
            $(
                #[test]
                fn $name() {
                    check_html_fixture(
                        concat!("samples/", stringify!($name)),
                        &work_items!($($ids)?),
                    );
                }
            )*
        }

        /// The samples with a test.
        const SAMPLES: &[&str] = &[$(stringify!($name)),*];
    };
}

samples! {
    aligned_and_styled_divs,
    caption_paragraph,
    centered_div_with_table,
    code_blocks,
    dropped_elements,
    formula_with_ampersand_and_newline,
    formulas,
    heading_levels_by_size,
    heading_sized_text_in_cells,
    heading_sizes,
    horizontal_rules,
    image_in_table_cell,
    image_with_caption_line,
    images,
    inline_styles,
    link_span_variants,
    list_items_with_several_blocks,
    lists,
    merged_table_cells,
    mixed_br_forms,
    nested_indents,
    paragraph_elements,
    paragraphs_and_breaks,
    plain_text_with_markdown_characters,
    quotes_and_alerts,
    tables,
    unknown_elements,
    whitespace_and_nbsp,
    word_body_text_11pt,
    work_item_anchors_and_links,
    work_item_links_with_prefixes: WorkItemIds::Prefixes(vec!["MP".to_owned()]),
}

/// A sample file without a test would be checked by nothing.
#[test]
fn every_sample_has_a_test() {
    let mut files: Vec<String> = fs::read_dir(fixtures().join("samples"))
        .expect("tests/fixtures/samples")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()? == "html").then(|| path.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect();
    files.sort();
    let mut listed: Vec<String> = SAMPLES.iter().map(|s| (*s).to_owned()).collect();
    listed.sort();
    assert_eq!(
        files, listed,
        "the files in tests/fixtures/samples and the samples! list in round_trip.rs differ"
    );
}

/// A golden file whose fixture is gone would go stale unnoticed.
#[test]
fn every_golden_file_has_a_fixture() {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                walk(&path, &format!("{prefix}{name}/"), out);
            } else {
                out.push(format!("{prefix}{name}"));
            }
        }
    }
    let mut golden = Vec::new();
    walk(&fixtures().join("golden"), "", &mut golden);
    for file in golden {
        let stem = file
            .strip_suffix(".drop.md")
            .or_else(|| file.strip_suffix(".md"))
            .unwrap_or_else(|| panic!("tests/fixtures/golden/{file}: not a golden file"));
        assert!(
            fixtures().join(format!("{stem}.html")).is_file(),
            "tests/fixtures/golden/{file} has no fixture tests/fixtures/{stem}.html"
        );
    }
}

mod diagrams {
    use super::*;

    /// The Markdown of `html` with these diagram sources.
    fn back(html: &str, sources: &[(&str, &str)]) -> String {
        let sources: DiagramSources = sources
            .iter()
            .map(|(id, source)| ((*id).to_owned(), (*source).to_owned()))
            .collect();
        to_markdown_with_diagrams(html, &ToMarkdownOptions::default(), &sources)
    }

    /// A mermaid block becomes an image of its attachment, and the attachment's source makes it
    /// the same block again; without the source it stays an image of the attachment.
    #[test]
    fn through_attachments_and_back() {
        let source = "graph TD;\n    A-->B;";
        let fence = format!("```mermaid\n{source}\n```");
        for markdown in [
            fence.clone(),
            format!("Before the diagram.\n\n{fence}\n\nAfter the diagram."),
            format!("> {}", fence.replace('\n', "\n> ")),
        ] {
            let diagrams = mermaid::diagrams(&markdown);
            assert_eq!(diagrams.len(), 1, "{markdown}");
            let file = &diagrams[0].file_name;
            let id = format!("1-{file}");
            let attachments: DiagramAttachments =
                [(file.clone(), id.clone())].into_iter().collect();
            let html = to_html_with_diagrams(&markdown, &ToHtmlOptions::default(), &attachments);
            assert!(
                html.contains(&format!("src=\"workitemimg:{id}\"")),
                "{markdown}\n{html}"
            );
            assert_text(
                &format!("{markdown:?} through the attachment {id} and back"),
                &markdown,
                &back(&html, &[(&id, &diagrams[0].source)]),
            );
            let image = to_markdown(&html, &ToMarkdownOptions::default());
            assert!(
                image.contains(&format!("![](workitemimg:{id} \"{file}\")")),
                "without its source the diagram stays an image:\n{image}"
            );
        }
    }

    /// Attachment ids are matched percent-decoded, in `workitemimg:` and `/wi-attachment/` URLs.
    #[test]
    fn by_any_reference_to_the_attachment() {
        let source = "graph LR\n    Sensor-->Gateway";
        let fence = format!("```mermaid\n{source}\n```");
        let file = mermaid::diagram(source).file_name;
        let id = "4-gateway diagram & more.svg";
        let attachments: DiagramAttachments = [(file.clone(), id.to_owned())].into_iter().collect();
        let html = to_html_with_diagrams(&fence, &ToHtmlOptions::default(), &attachments);
        assert!(
            html.contains("workitemimg:4-gateway%20diagram%20%26%20more.svg"),
            "{html}"
        );
        assert_text(
            "a percent-encoded attachment id",
            &fence,
            &back(&html, &[(id, source)]),
        );
        let html = format!(
            r#"<img src="https://polarion.example.com/polarion/wi-attachment/PROJ/PROJ-1/7-{file}?revision=12" style="max-width: 650px;"/>"#
        );
        assert_text(
            "a /wi-attachment/ URL",
            &fence,
            &back(&html, &[(&format!("7-{file}"), source)]),
        );
    }

    /// Legacy mermaid.ink images carry their source (standard base64, padded).
    #[test]
    fn from_legacy_mermaid_ink_images() {
        let html = r#"<img src="https://mermaid.ink/img/Z3JhcGggTFIKICAgIFNlbnNvci0tPkdhdGV3YXkKICAgIEdhdGV3YXktLT5TZXJ2ZXI=" style="max-width: 650px;" />"#;
        assert_text(
            "a mermaid.ink image",
            "```mermaid\ngraph LR\n    Sensor-->Gateway\n    Gateway-->Server\n```",
            &to_markdown(html, &ToMarkdownOptions::default()),
        );
    }
}

/// Blank HTML is empty Markdown; Markdown never ends with a line break.
#[test]
fn blank_html_is_empty_markdown() {
    for html in ["", " ", "\n\t \n"] {
        assert_eq!(
            to_markdown(html, &ToMarkdownOptions::default()),
            "",
            "{html:?}"
        );
    }
    let markdown = to_markdown("text<br/><br/>", &ToMarkdownOptions::default());
    assert!(!markdown.ends_with('\n'), "{markdown:?}");
}
