//! Random input through the conversion to Markdown, from fixed seeds (no new dependency):
//! - RTE-like HTML, [`HTML_CASES`] cases: no panic, every built tree valid, and the Markdown a
//!   fixpoint (`to_markdown(to_html(M)) == M`), in keep and drop mode and under each id setting;
//! - the HTML of random Markdown made of the forward constructs, [`FLAT_MARKDOWN_CASES`] cases
//!   of plain compositions and [`NESTED_MARKDOWN_CASES`] with formatting nested in formatting:
//!   the Markdown converts back to exactly that HTML (the conversion is lossless on forward
//!   output), and it is a fixpoint.
//!
//! A failure names the case's seed and prints its input shrunk by dropping chunks while the same
//! kind of failure persists. `FUZZ_CASES=N` runs N cases per test instead of the defaults, and
//! `FUZZ_SEED=<hex>` runs only the case with that seed. The defaults take about a minute in a
//! debug build.

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, PanicHookInfo};
use std::sync::Once;
use std::time::{Duration, Instant};

use base64::Engine as _;
use polarion_markdown::{
    DiagramAttachments, DiagramSources, RawHtml, ToHtmlOptions, ToMarkdownOptions, WorkItemIds,
    mermaid, to_html, to_html_with_diagrams, to_markdown_report, to_markdown_with_diagrams,
};

/// Cases of [`rte_like_html_converts_to_a_fixpoint`].
const HTML_CASES: usize = 2_000;
/// Cases of [`forward_output_comes_back_exactly`].
const FLAT_MARKDOWN_CASES: usize = 600;
/// Cases of [`forward_output_with_nested_formatting_comes_back_exactly`].
const NESTED_MARKDOWN_CASES: usize = 400;

/// What shrinking may spend per phase.
const SHRINK_ATTEMPTS: usize = 2_000;
const SHRINK_TIME: Duration = Duration::from_secs(30);

#[test]
fn rte_like_html_converts_to_a_fixpoint() {
    run(
        "RTE-like HTML",
        0x2545_F491_4F6C_DD1D,
        HTML_CASES,
        html_case,
        check_html,
    );
}

/// Markdown whose formatting holds only words: the HTML it gives comes back exactly.
#[test]
fn forward_output_comes_back_exactly() {
    run(
        "forward output",
        0x9E37_79B9_7F4A_7C15,
        FLAT_MARKDOWN_CASES,
        |rng| markdown_case(rng, false),
        check_markdown,
    );
}

/// Markdown with formatting and links inside other formatting (never inside the same kind),
/// so the order in which the reverse direction nests them matters.
#[test]
fn forward_output_with_nested_formatting_comes_back_exactly() {
    run(
        "forward output with nested formatting",
        0xD1B5_4A32_D192_ED03,
        NESTED_MARKDOWN_CASES,
        |rng| markdown_case(rng, true),
        check_markdown,
    );
}

/// A deterministic xorshift generator.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    /// True once in `n` times.
    fn chance(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// The seed of case `case` of a test: neighboring cases get unrelated seeds (splitmix64).
fn case_seed(test: u64, case: usize) -> u64 {
    let mut z = test.wrapping_add((case as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The seeds of the cases a test runs: the one `FUZZ_SEED` names, or `FUZZ_CASES` (else
/// `cases`) of them.
fn seeds(test: u64, cases: usize) -> Vec<u64> {
    if let Some(seed) = std::env::var("FUZZ_SEED")
        .ok()
        .and_then(|seed| u64::from_str_radix(seed.trim().trim_start_matches("0x"), 16).ok())
    {
        return vec![seed];
    }
    let cases = std::env::var("FUZZ_CASES")
        .ok()
        .and_then(|cases| cases.trim().parse().ok())
        .unwrap_or(cases);
    (0..cases).map(|case| case_seed(test, case)).collect()
}

thread_local! {
    /// Panics on this thread are expected ([`quietly`]): recorded, not printed.
    static QUIET: Cell<bool> = const { Cell::new(false) };
    static LAST_PANIC: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Installs, once, a panic hook that records the panics of threads running [`quietly`] and
/// leaves those of the others to the previous hook.
fn install_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
            if QUIET.with(Cell::get) {
                LAST_PANIC.with(|last| *last.borrow_mut() = info.to_string());
            } else {
                previous(info);
            }
        }));
    });
}

/// `f()`, or the message and location of its panic.
fn quietly<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    install_hook();
    QUIET.with(|quiet| quiet.set(true));
    let result = std::panic::catch_unwind(AssertUnwindSafe(f));
    QUIET.with(|quiet| quiet.set(false));
    result.map_err(|_| LAST_PANIC.with(|last| last.borrow().clone()))
}

/// What went wrong with a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    /// A conversion panicked.
    Panic,
    /// comrak's `validate()` rejected a tree the conversion built.
    Invalid,
    /// The Markdown converts to other HTML than the forward output it came from.
    Lossy,
    /// Converting the Markdown to HTML and back gives other Markdown.
    NotFixpoint,
}

#[derive(Debug)]
struct Problem {
    failure: Failure,
    details: String,
}

fn problem(failure: Failure, details: String) -> Problem {
    Problem { failure, details }
}

/// `f()`, or a [`Failure::Panic`] saying that `what` panicked.
fn guarded<T>(what: &str, f: impl FnOnce() -> T) -> Result<T, Problem> {
    quietly(f).map_err(|message| problem(Failure::Panic, format!("{what} panicked: {message}")))
}

/// The part of `s` around byte `at`.
fn around(s: &str, at: usize) -> &str {
    let mut start = at.saturating_sub(60).min(s.len());
    while !s.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + 60).min(s.len());
    while !s.is_char_boundary(end) {
        end += 1;
    }
    &s[start..end]
}

/// Where two strings first differ, with the text around it.
fn difference(a: &str, b: &str) -> String {
    let at = a
        .char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or(a.len().min(b.len()), |((i, _), _)| i);
    format!(
        "first difference at byte {at}:\n  {:?}\n  {:?}",
        around(a, at),
        around(b, at)
    )
}

/// The options of the conversion back to HTML that go with `options` (no highlighting: its time
/// budget can make the HTML depend on the machine).
fn forward_options(options: &ToMarkdownOptions) -> ToHtmlOptions {
    ToHtmlOptions {
        work_items: options.work_items.clone(),
        highlight: false,
    }
}

/// A generated input.
#[derive(Debug, Clone)]
struct Case {
    /// The input in pieces (tags and text of HTML, lines of Markdown), which shrinking drops.
    pieces: Vec<String>,
    /// What the pieces are joined with.
    separator: &'static str,
    options: ToMarkdownOptions,
    /// Markdown: mermaid blocks are written as images of attachments.
    attach: bool,
}

type Check = fn(&str, &Case) -> Result<(), Problem>;

/// Generates and checks every case of a test; the first failure is shrunk and reported.
fn run(name: &str, test: u64, cases: usize, generate: impl Fn(&mut Rng) -> Case, check: Check) {
    for seed in seeds(test, cases) {
        let case = generate(&mut Rng::new(seed));
        let input = case.pieces.join(case.separator);
        if let Err(found) = check(&input, &case) {
            fail(name, seed, &case, &found, check);
        }
    }
}

/// Shrinks a failing case and panics with its seed, the shrunk input and what went wrong.
fn fail(name: &str, seed: u64, case: &Case, found: &Problem, check: Check) -> ! {
    let kind = found.failure;
    let fails = |pieces: &[String]| matches!(check(&pieces.join(case.separator), case), Err(p) if p.failure == kind);
    let mut shrunk = shrink(case.pieces.clone(), &fails);
    if case.separator.is_empty() {
        // HTML: then single characters.
        let chars: Vec<String> = shrunk.concat().chars().map(String::from).collect();
        shrunk = shrink(chars, &fails);
    }
    let input = shrunk.join(case.separator);
    let details = match check(&input, case) {
        Err(problem) => problem.details,
        Ok(()) => "(the shrunk input passes)".to_owned(),
    };
    panic!(
        "{name}: case {seed:#018x} fails ({kind:?}) with {:?}; FUZZ_SEED={seed:x} runs it alone\n\
         shrunk input:\n{input:?}\n{details}\n\noriginal input:\n{:?}\n{}",
        case.options,
        case.pieces.join(case.separator),
        found.details
    );
}

/// `pieces` without the chunks whose removal keeps `fails` true: chunks of the whole length
/// first, then halved down to single pieces, within [`SHRINK_ATTEMPTS`] and [`SHRINK_TIME`].
fn shrink(mut pieces: Vec<String>, fails: &dyn Fn(&[String]) -> bool) -> Vec<String> {
    let started = Instant::now();
    let mut attempts = 0;
    let mut chunk = pieces.len().max(1);
    loop {
        let mut removed = false;
        let mut i = 0;
        while i < pieces.len() {
            if attempts >= SHRINK_ATTEMPTS || started.elapsed() > SHRINK_TIME {
                return pieces;
            }
            attempts += 1;
            let end = (i + chunk).min(pieces.len());
            let candidate: Vec<String> =
                pieces[..i].iter().chain(&pieces[end..]).cloned().collect();
            if fails(&candidate) {
                pieces = candidate;
                removed = true;
            } else {
                i = end;
            }
        }
        if !removed {
            if chunk == 1 {
                return pieces;
            }
            chunk = chunk.div_ceil(2);
        }
    }
}

/// The diagram the generated images show.
const DIAGRAM: &str = "graph TD\n    A-->B";

fn diagram_id() -> String {
    format!("1-{}", mermaid::diagram(DIAGRAM).file_name)
}

/// The sources of the diagrams the generated HTML shows.
fn diagram_sources() -> DiagramSources {
    [(diagram_id(), DIAGRAM.to_owned())].into_iter().collect()
}

fn check_html(html: &str, case: &Case) -> Result<(), Problem> {
    let options = &case.options;
    let sources = diagram_sources();
    let report = guarded("to_markdown", || {
        to_markdown_report(html, options, &sources)
    })?;
    if report.invalid != 0 {
        return Err(problem(
            Failure::Invalid,
            format!(
                "{} invalid trees; the Markdown:\n{}",
                report.invalid, report.markdown
            ),
        ));
    }
    let forward = forward_options(options);
    let back = guarded("to_html of the Markdown", || {
        to_html(&report.markdown, &forward)
    })?;
    let again = guarded("to_markdown of that HTML", || {
        to_markdown_with_diagrams(&back, options, &sources)
    })?;
    if again != report.markdown {
        return Err(problem(
            Failure::NotFixpoint,
            format!(
                "the Markdown:\n{}\nits HTML:\n{back}\nthat HTML's Markdown:\n{again}\n{}",
                report.markdown,
                difference(&report.markdown, &again)
            ),
        ));
    }
    Ok(())
}

fn check_markdown(markdown: &str, case: &Case) -> Result<(), Problem> {
    let options = &case.options;
    let forward = forward_options(options);
    let diagrams = guarded("mermaid::diagrams", || mermaid::diagrams(markdown))?;
    let attachments: DiagramAttachments = if case.attach {
        diagrams
            .iter()
            .map(|d| (d.file_name.clone(), format!("1-{}", d.file_name)))
            .collect()
    } else {
        DiagramAttachments::new()
    };
    let sources: DiagramSources = diagrams
        .iter()
        .map(|d| (format!("1-{}", d.file_name), d.source.clone()))
        .collect();
    let html = guarded("to_html", || {
        to_html_with_diagrams(markdown, &forward, &attachments)
    })?;
    let report = guarded("to_markdown", || {
        to_markdown_report(&html, options, &sources)
    })?;
    if report.invalid != 0 {
        return Err(problem(
            Failure::Invalid,
            format!(
                "{} invalid trees; the HTML:\n{html}\nits Markdown:\n{}",
                report.invalid, report.markdown
            ),
        ));
    }
    let back = guarded("to_html of the Markdown", || {
        to_html_with_diagrams(&report.markdown, &forward, &attachments)
    })?;
    if back != html {
        return Err(problem(
            Failure::Lossy,
            format!(
                "the HTML:\n{html}\nits Markdown:\n{}\nthat Markdown's HTML:\n{back}\n{}",
                report.markdown,
                difference(&html, &back)
            ),
        ));
    }
    let plain = guarded("to_html of the Markdown without attachments", || {
        to_html(&report.markdown, &forward)
    })?;
    let again = guarded("to_markdown of that HTML", || {
        to_markdown_with_diagrams(&plain, options, &sources)
    })?;
    if again != report.markdown {
        return Err(problem(
            Failure::NotFixpoint,
            format!(
                "the Markdown:\n{}\nits HTML:\n{plain}\nthat HTML's Markdown:\n{again}\n{}",
                report.markdown,
                difference(&report.markdown, &again)
            ),
        ));
    }
    Ok(())
}

// --- RTE-like HTML -------------------------------------------------------------------------

/// Text: words, Markdown syntax, id shapes, character references (valid and not), non-ASCII.
const WORDS: &[&str] = &[
    "word",
    "Text",
    "more",
    "a",
    "x",
    "Intro",
    "PROJ-1",
    "MP-42",
    "MY_PROJ-12",
    "UTF-8",
    "A-1",
    "PROJ-1.5",
    "(PROJ-7)",
    "a*b",
    "_x_",
    "**",
    "~",
    "~~",
    "^",
    "$5",
    "$",
    "==",
    "++",
    "|",
    "#",
    "# x",
    "1.",
    "1)",
    "-",
    "+",
    ":",
    "!",
    "@",
    "a@b.cd",
    "www.x.y",
    "http://x.y",
    "`",
    "\\",
    "[x]",
    "[x](y)",
    "&lt;",
    "&gt;",
    "&amp;",
    "&lt;b&gt;",
    "&#39;",
    "&quot;",
    "&#10;",
    "&nbsp;",
    "&#160;",
    "&bogus;",
    "&#x41;",
    "&#0;",
    "&",
    "<",
    ">",
    "ä",
    "日本",
    "😀",
    "☐ ",
    "☑ ",
    "Figure 1:",
    "---",
    "***",
    "&lt;Image: diagram.svg&gt;",
];

/// Between inline pieces.
const SPACES: &[&str] = &[" ", " ", " ", "", "  ", "\n", " \n  ", "&nbsp;", "\t"];

/// Between blocks.
const SEPARATORS: &[&str] = &[
    "<br/><br/>",
    "<br/><br/>",
    "<br/>",
    "",
    "\n",
    "<br/><br/><br/>",
    "<br><br/>",
    " <br/> <br/> ",
    "\n \n",
    "&nbsp;<br/>",
    "<br/>\n",
];

/// Inline styles: representable, raw, both, and broken ones.
const STYLES: &[&str] = &[
    "font-weight: bold;",
    "font-style: italic;",
    "font-family: 'Courier New', Courier, monospace;",
    "font-family: &#39;Courier New&#39;, Courier, monospace;",
    "text-decoration: line-through;",
    "text-decoration: underline;",
    "text-decoration: underline line-through;",
    "background-color: #FFFF00;",
    "background-color: yellow;",
    "background-color: #FF0;",
    "background-color: rgb(255, 255, 0);",
    "font-weight: normal;",
    "font-weight: 700;",
    "font-style: oblique;",
    "color: #FF0000;",
    "background-color: #FF0000;",
    "font-size: 12pt;line-height: 1.5;",
    "font-size: 11pt;",
    "font-size: 10pt;",
    "font-size: 14pt;",
    "font-weight: bold; font-size: 22pt; line-height: 1.2;",
    "font-weight: bold; font-size: 18pt;",
    "font-weight: bold; font-size: 10.5pt;",
    "font-family: Arial, Helvetica, sans-serif;",
    "font-family: &#39;Segoe UI&#39;, Selawik, &#39;Open Sans&#39;, sans-serif;",
    "font-weight: bold; font-style: italic;",
    "color: #FF0000; font-weight: bold;",
    "vertical-align: super;",
    "letter-spacing: 2px;",
    "",
    "font-size:",
    "color: red; color: blue;",
];

const TAGS: &[&str] = &[
    "b", "strong", "i", "em", "u", "s", "del", "mark", "code", "sup", "sub", "span", "ins",
    "strike", "tt", "kbd", "small", "big", "abbr", "q", "cite", "font",
];

const LINK_SPANS: &[&str] = &[
    r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-1" data-option-id="long"></span>"#,
    r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="MP-42" data-option-id="long"></span>"#,
    r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="MY_PROJ-12" data-option-id="long"></span>"#,
    r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="UTF-8" data-option-id="long"></span>"#,
    r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-2" data-option-id="short"></span>"#,
    r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-3" data-option-id="custom" data-custom-label="the spec"></span>"#,
    r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-4" data-option-id="default"></span>"#,
    r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-scope="OTHER" data-item-id="OTHER-5" data-option-id="long"></span>"#,
    r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="PROJ-6" data-revision="1234" data-option-id="long"></span>"#,
    r#"<span class="polarion-rte-link" data-type="crossReference" id="fake" data-item-id="PROJ-7" data-option-id="long"></span>"#,
    r#"<span class="polarion-rte-link" data-type="richPage" id="fake" data-item-name="Home" data-space-name="_default" data-option-id="long"></span>"#,
    r#"<span class="polarion-rte-link" data-type="polarion" id="fake" data-url="https://example.com/x" data-option-id="long"></span>"#,
    r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="" data-option-id="long"></span>"#,
    r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="not an id" data-option-id="long"></span>"#,
    r#"<span data-type="workItem" data-item-id="PROJ-8" data-option-id="long"></span>"#,
];

/// `data-source` values as written in HTML.
const FORMULA_SOURCES: &[&str] = &[
    "x",
    "\\sqrt{x}",
    "a &amp; b",
    "a&#10;b",
    "a\nb",
    " x ",
    "a$b",
    "a`b",
    "",
    "$$",
    "x<br/>y",
    "x<br>y",
    "ä²",
    "a_b^c",
    "\\frac{1}{2}",
    "&quot;q&quot;",
    "1",
];

const ANCHORS: &[&str] = &[
    "<a href=\"http://example.com\" target=\"_top\" class=\"descriptionLink\">",
    "<a href=\"https://polarion.example.com/polarion/#/project/PROJ/workitem?id=PROJ-7\">",
    "<a href=\"javascript:alert(1)\">",
    "<a>",
    "<a href=\"u\" title=\"T\">",
    "<a href=\"mailto:a@b.cd\">",
    "<a href=\"\">",
    "<a href=\"https://example.com/a?b=1&amp;c=2\" target=\"_blank\">",
];

const IMAGES: &[&str] = &[
    "<img src=\"workitemimg:1-a.png\" style=\"max-width: 650px;\"/>",
    "<img title=\"a.png\" src=\"workitemimg:1-a.png\" style=\"max-width: 650px;\" data-lastmodified=\"2025-01-01T08:00:00.000+01:00\"/>",
    "<img src=\"workitemimg:1-b.png\"/>",
    "<img src=\"workitemimg:2-c.jpg\" style=\"width: 100px;height: 74px;\">",
    "<img src=\"https://example.com/image.png\" alt=\"test\" />",
    "<img src=\"https://mermaid.ink/img/pako:eNpLz\" />",
    "<img src=\"javascript:alert(1)\">",
    "<img src=\"x.png\" onerror=\"alert(1)\">",
    "<img>",
    "<img src=\"\">",
    "<img src=\"a b.png\" alt=\"a &quot;q&quot;\" title=\"t&amp;\">",
    "<img src=\"workitemimg:3-my%20image.png\" class=\"x\"/>",
];

/// Contents of `pre`: code, highlighter spans, breaks, entities.
const CODE: &[&str] = &[
    "let x = 1;",
    "fn main() {}",
    "&lt;a&gt; &amp; b",
    "<span style=\"color:#0000FF;\">fn</span>",
    "<span>1</span>",
    "\n",
    "<br>",
    "<br/>",
    "  indented",
    "A--&gt;B",
    "graph TD",
    "\t",
    "PROJ-1",
    "`",
    "```",
    "$x$",
    "\n\n",
    "**not bold**",
    "&nbsp;",
];

/// Lines of an older converter's code block (a monospace span in an indent div).
const CODE_LINES: &[&str] = &[
    "line1",
    "let x = 1;",
    "if a &lt; b {",
    "}",
    "  indented",
    "PROJ-1",
    "x = y * 2",
    "`tick`",
];

const STRAY: &[&str] = &[
    "</div>",
    "</span>",
    "</p>",
    "</li>",
    "</ul>",
    "</table>",
    "</td>",
    "</tr>",
    "<div>",
    "<span style=\"color: red;\">",
    "<b>",
    "<ul>",
    "<li>",
    "<td>",
    "<tr>",
    "</a>",
    "<a href=\"u\">",
    "</pre>",
    "<pre>",
    "<table>",
    "</b>",
    "<i>",
];

/// Comments, elements that are dropped, other markup.
const FILTERED: &[&str] = &[
    "<!-- note -->",
    "<script>alert(1)</script>",
    "<style>p{color:red}</style>",
    "<iframe src=\"x\"></iframe>",
    "<object data=\"x\"></object>",
    "<embed src=\"x\">",
    "<noscript>n</noscript>",
    "<!DOCTYPE html>",
    "<?pi?>",
    "<![CDATA[x]]>",
    "<textarea>t</textarea>",
    "<meta charset=\"utf-8\">",
    "<title>t</title>",
    "<!--",
];

const ENTITIES: &[&str] = &[
    "&nbsp;",
    "&amp;",
    "&lt;",
    "&gt;",
    "&quot;",
    "&#39;",
    "&#10;",
    "&#160;",
    "&#xA0;",
    "&bogus;",
    "&#0;",
    "&#xD800;",
    "&#99999999;",
    "&amp",
    "&copy;",
    "&mdash;",
    "&#x1F600;",
];

/// Characters XML does not allow, and line endings.
const CONTROLS: &[&str] = &[
    "\u{1}", "\u{b}", "\u{c}", "\u{1b}", "\u{0}", "\u{fffe}", "\r", "\r\n",
];

/// Alert colors and titles (the forward pattern, and one with a title of its own).
const ALERTS: &[(&str, &str)] = &[
    ("#0969DA", "Note"),
    ("#1A7F37", "Tip"),
    ("#8250DF", "Important"),
    ("#9A6700", "Warning"),
    ("#CF222E", "Caution"),
    ("#0969DA", "Custom title"),
];

const DIV_STYLES: &[&str] = &[
    "text-align: center;",
    "text-align: right;",
    "border: 1px solid red;",
    "color: #FF0000;",
    "margin-left: 40px;",
    "margin-left: 80px;",
    "margin-left: 20px;",
    "padding: 5px;",
    "text-align: justify;",
];

const SIZES: &[&str] = &[
    "22", "18", "14", "12", "11", "10.5", "10", "16", "9", "14.5",
];

const FORWARD_TABLE: &str = concat!(
    r#"<table id="polarion_wiki macro name=table" class="polarion-Document-table" "#,
    r#"style="width: 100%; max-width: 1280px; margin-left: 0px; margin-right: auto; "#,
    r#"border: 1px solid #CCCCCC; empty-cells: show; border-collapse: collapse;">"#
);
const RTE_TABLE: &str = "<table id=\"polarion_wiki macro name=table\" class=\"polarion-Document-table\" style=\"width: 100%;max-width: 1280px;margin-left: 0px;margin-right: auto;border: 1px solid #CCCCCC;empty-cells: show;border-collapse: collapse;max-width: 1280.0px;\">";

/// ` style="…"` of header cells.
const TH_STYLES: &[&str] = &[
    r#" style="font-weight: bold; background-color: #F0F0F0; text-align: left; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;""#,
    r#" style="font-weight: bold; background-color: #F0F0F0; text-align: center; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;""#,
    " style=\"font-weight: bold;background-color: #F0F0F0;text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;border-top: 1px solid #CCCCCC;border-bottom: 1px solid #CCCCCC;border-right: 1px solid #CCCCCC;border-left: 1px solid #CCCCCC;padding: 5px;\"",
    " style=\"font-weight: bold;\"",
    " style=\"font-weight: bold; background-color: #FF0000;\"",
];

/// ` style="…"` of data cells.
const TD_STYLES: &[&str] = &[
    r#" style="text-align: left; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;""#,
    r#" style="text-align: center; vertical-align: top; height: 12px; border: 1px solid #CCCCCC; padding: 5px;""#,
    " style=\"text-align: left;vertical-align: top;height: 12px;border: 1px solid #CCCCCC;border-top: 1px solid #CCCCCC;border-bottom: 1px solid #CCCCCC;border-right: 1px solid #CCCCCC;border-left: 1px solid #CCCCCC;padding: 5px;\"",
    "",
];

/// RTE-like HTML: the shapes Polarion's rich text editor writes, mixed at random, with raw and
/// representable styles, runs of breaks, malformed tables, unclosed and misnested tags,
/// character references and stray text.
struct HtmlGen<'r> {
    rng: &'r mut Rng,
    out: Vec<String>,
}

impl HtmlGen<'_> {
    fn push(&mut self, piece: &str) {
        self.out.push(piece.to_owned());
    }

    fn pick(&mut self, items: &[&str]) {
        let piece = self.rng.pick(items);
        self.push(piece);
    }

    /// Enough pieces: what follows is short.
    fn full(&self) -> bool {
        self.out.len() > 150
    }

    /// A closing tag, mostly: one in twelve is left out.
    fn close(&mut self, tag: &str) {
        if !self.rng.chance(12) {
            self.push(tag);
        }
    }

    fn document(&mut self) {
        for _ in 0..1 + self.rng.below(6) {
            self.block(0);
            self.pick(SEPARATORS);
        }
    }

    fn blocks(&mut self, depth: usize) {
        for i in 0..1 + self.rng.below(3) {
            if i > 0 {
                self.pick(SEPARATORS);
            }
            self.block(depth);
        }
    }

    fn block(&mut self, depth: usize) {
        if depth > 3 || self.full() {
            self.inline_run(depth);
            return;
        }
        match self.rng.below(24) {
            0..=7 => self.inline_run(depth),
            8 => {
                self.pick(&[
                    "<div style=\"margin-left: 40px;\">",
                    "<div style=\"margin-left: 80px;\">",
                    "<div style=\"margin-left: 40px;\">\n  ",
                ]);
                self.blocks(depth + 1);
                self.close("</div>");
            }
            9 => self.alert(depth),
            10 => {
                let style = self.rng.pick(DIV_STYLES);
                self.push(&format!("<div style=\"{style}\">"));
                self.blocks(depth + 1);
                if self.rng.chance(2) {
                    self.push("<br/> \n");
                }
                self.close("</div>");
            }
            11 => {
                self.push("<div>");
                self.blocks(depth + 1);
                self.close("</div>");
            }
            12 => {
                self.push("<div style=\"color:#000000;background-color:#FFFFFF;\">");
                self.pre();
                self.close("</div>");
            }
            13 => self.pre(),
            14 => self.list(depth),
            15 | 16 => self.table(depth),
            17 => self.pick(&["<hr/>", "<hr>", "<hr style=\"width: 50%;\">"]),
            18 => {
                self.pick(&[
                    "<p class=\"polarion-rte-caption-paragraph\">\n  Figure <span data-sequence=\"Figure\" class=\"polarion-rte-caption\">#</span> ",
                    "<p>",
                    "<p style=\"text-align: center;\">",
                    "<p class=\"x\">",
                ]);
                self.inline_run(depth + 1);
                self.pick(&["\n</p>", "</p>", ""]);
            }
            19 => {
                self.image();
                if self.rng.chance(2) {
                    self.pick(&["<br/>", "<br>"]);
                    self.inline_run(depth + 1);
                }
            }
            20 => {
                self.push("<div style=\"margin-left: 40px;\"><span style=\"font-family: 'Courier New', Courier, monospace;\">");
                for i in 0..1 + self.rng.below(4) {
                    if i > 0 {
                        self.push("<br/>");
                    }
                    self.pick(CODE_LINES);
                }
                self.push("</span></div>");
            }
            21 => self.pick(STRAY),
            22 => self.pick(FILTERED),
            _ => self.heading(depth),
        }
    }

    fn heading(&mut self, depth: usize) {
        let size = self.rng.pick(SIZES);
        match self.rng.below(3) {
            0 => {
                self.push(&format!(
                    "<span style=\"font-weight: bold; font-size: {size}pt; line-height: 1.2;\">"
                ));
                self.inline_run(depth + 1);
                self.close("</span>");
            }
            1 => {
                self.push(&format!("<span style=\"font-size: {size}pt;\">"));
                self.inline_run(depth + 1);
                self.close("</span>");
            }
            _ => {
                let level = 1 + self.rng.below(6);
                self.push(&format!("<h{level}>"));
                self.inline_run(depth + 1);
                self.close(&format!("</h{level}>"));
            }
        }
    }

    fn alert(&mut self, depth: usize) {
        let (color, title) = ALERTS[self.rng.below(ALERTS.len())];
        self.push(&format!(
            "<div style=\"margin-left: 40px; border-left: 3px solid {color}; padding-left: 10px;\"><span style=\"font-weight: bold; color: {color};\">{title}</span><br/>"
        ));
        self.blocks(depth + 1);
        self.close("</div>");
    }

    fn pre(&mut self) {
        const PRE: &[(&str, &str)] = &[
            ("<pre>", "</pre>"),
            ("<pre class=\"language-rust\">", "</pre>"),
            (
                "<pre class=\"language-mermaid\" style=\"margin-left: 40px; font-family: 'Courier New', Courier, monospace;\">",
                "</pre>",
            ),
            (
                "<pre style=\"margin-left: 40px; font-family: 'Courier New', Courier, monospace;\">",
                "</pre>",
            ),
            ("<pre><code>", "</code></pre>"),
            ("<pre><code class=\"language-python\">", "</code></pre>"),
            ("<pre class=\"lang-js\">", "</pre>"),
            ("<pre data-lang=\"sql\">", "</pre>"),
        ];
        let (open, end) = PRE[self.rng.below(PRE.len())];
        self.push(open);
        for _ in 0..1 + self.rng.below(5) {
            self.pick(CODE);
        }
        self.close(end);
    }

    fn list(&mut self, depth: usize) {
        const LISTS: &[(&str, &str)] = &[
            ("<ul>", "</ul>"),
            ("<ol>", "</ol>"),
            ("<ol start=\"3\">", "</ol>"),
            ("<ol start=\"0\">", "</ol>"),
        ];
        let (open, end) = LISTS[self.rng.below(LISTS.len())];
        self.push(open);
        for _ in 0..1 + self.rng.below(4) {
            self.pick(&["\n  <li>", "<li>"]);
            if self.rng.chance(4) {
                self.pick(&["☐ ", "☑ ", "☒ ", "☐&nbsp;", "☐"]);
            }
            match self.rng.below(6) {
                0..=2 => self.inline_run(depth + 1),
                3 => {
                    self.inline_run(depth + 1);
                    self.pick(&["<br/><br/>", "<br/>"]);
                    self.inline_run(depth + 1);
                }
                4 => {
                    self.inline_run(depth + 1);
                    if depth < 3 {
                        self.list(depth + 1);
                    }
                }
                _ => self.block(depth + 1),
            }
            self.close("</li>");
        }
        self.pick(&["\n", ""]);
        self.close(end);
    }

    fn table(&mut self, depth: usize) {
        let polarion = !self.rng.chance(3);
        if polarion {
            self.pick(&[FORWARD_TABLE, RTE_TABLE]);
        } else {
            self.pick(&[
                "<table>",
                "<table border=\"1\">",
                "<table style=\"width: 80%;\">",
            ]);
        }
        let tbody = !self.rng.chance(4);
        if tbody {
            self.pick(&["<tbody>", "\n  <tbody>"]);
        }
        let columns = 1 + self.rng.below(3);
        for row in 0..1 + self.rng.below(3) {
            self.pick(&["<tr>", "\n    <tr>"]);
            let header = row == 0 && !self.rng.chance(5);
            let cells = match self.rng.below(10) {
                0 => columns + 1,
                1 => columns.saturating_sub(1).max(1),
                _ => columns,
            };
            for _ in 0..cells {
                let tag = if header { "th" } else { "td" };
                let style = match (polarion, header) {
                    (false, _) => "",
                    (true, true) => self.rng.pick(TH_STYLES),
                    (true, false) => self.rng.pick(TD_STYLES),
                };
                let span = if self.rng.chance(10) {
                    self.rng
                        .pick(&[" colspan=\"2\"", " rowspan=\"2\"", " colspan=\"0\""])
                } else {
                    ""
                };
                self.push(&format!("<{tag}{span}{style}>"));
                if self.rng.chance(10) {
                    self.block(depth + 1);
                } else {
                    self.inline_run(depth + 1);
                }
                self.close(&format!("</{tag}>"));
            }
            self.close("</tr>");
        }
        if tbody {
            self.close("</tbody>");
        }
        self.close("</table>");
    }

    fn image(&mut self) {
        match self.rng.below(5) {
            0 => {
                let file = mermaid::diagram(DIAGRAM).file_name;
                self.push(&format!(
                    "<img src=\"workitemimg:{}\" title=\"{file}\" style=\"max-width: 650px;\"/>",
                    diagram_id()
                ));
            }
            1 => {
                let payload = base64::engine::general_purpose::STANDARD.encode(DIAGRAM);
                self.push(&format!(
                    "<img src=\"https://mermaid.ink/img/{payload}\" style=\"max-width: 650px;\" />"
                ));
            }
            _ => self.pick(IMAGES),
        }
    }

    fn formula(&mut self) {
        let source = self.rng.pick(FORMULA_SOURCES);
        let display = if self.rng.chance(3) {
            " data-inline=\"false\""
        } else {
            ""
        };
        let span = if self.rng.chance(2) {
            format!(
                "<span data-source=\"{source}\"{display} class=\"polarion-rte-formula\"></span>"
            )
        } else {
            format!(
                "<span class=\"polarion-rte-formula\"{display} data-source=\"{source}\"></span>"
            )
        };
        self.push(&span);
        if self.rng.chance(4) {
            self.pick(&["1", "&nbsp;", "$", "x"]);
        }
    }

    fn inline_run(&mut self, depth: usize) {
        for i in 0..1 + self.rng.below(6) {
            if i > 0 {
                self.pick(SPACES);
            }
            self.inline(depth);
        }
    }

    fn inline(&mut self, depth: usize) {
        if depth > 6 || self.full() {
            self.pick(WORDS);
            return;
        }
        match self.rng.below(26) {
            0..=9 => self.pick(WORDS),
            10..=12 => {
                let style = self.rng.pick(STYLES);
                self.push(&format!("<span style=\"{style}\">"));
                self.inline_run(depth + 1);
                self.close("</span>");
            }
            13 => {
                let tag = self.rng.pick(TAGS);
                self.push(&format!("<{tag}>"));
                self.inline_run(depth + 1);
                self.close(&format!("</{tag}>"));
            }
            14 => self.pick(LINK_SPANS),
            15 => self.formula(),
            16 => {
                self.pick(ANCHORS);
                self.inline_run(depth + 1);
                self.close("</a>");
            }
            17 => self.image(),
            18 => self.pick(&[
                "<br/>",
                "<br>",
                "</br>",
                "<br/><br/>",
                "<br/><br/><br/>",
                "<br/> <br/>",
            ]),
            19 => {
                // Misnested: a browser repairs it.
                self.push("<b><i>");
                self.pick(WORDS);
                self.push("</b>");
                self.pick(WORDS);
                self.push("</i>");
            }
            20 => self.pick(STRAY),
            21 => self.pick(ENTITIES),
            22 => {
                if self.rng.chance(3) {
                    self.pick(CONTROLS);
                } else {
                    self.pick(WORDS);
                }
            }
            23 => {
                self.push("<font color=\"red\">");
                self.inline_run(depth + 1);
                self.close("</font>");
            }
            _ => {
                self.pick(&["X", "H", "x "]);
                let tag = self.rng.pick(&["sup", "sub"]);
                self.push(&format!("<{tag}>"));
                self.pick(WORDS);
                self.close(&format!("</{tag}>"));
            }
        }
    }
}

fn html_case(rng: &mut Rng) -> Case {
    let pieces = {
        let mut generator = HtmlGen {
            rng: &mut *rng,
            out: Vec::new(),
        };
        generator.document();
        generator.out
    };
    let raw_html = if rng.chance(2) {
        RawHtml::Keep
    } else {
        RawHtml::Drop
    };
    let work_items = match rng.below(5) {
        0 => WorkItemIds::Prefixes(vec!["PROJ".into()]),
        1 => WorkItemIds::Off,
        _ => WorkItemIds::Any,
    };
    Case {
        pieces,
        separator: "",
        options: ToMarkdownOptions {
            raw_html,
            work_items,
        },
        attach: false,
    }
}

// --- Markdown of the forward constructs ----------------------------------------------------

/// Words of text: plain, id shapes, escaped syntax, character references, non-ASCII. None
/// starts a block or ends in a backslash.
const MD_WORDS: &[&str] = &[
    "alpha",
    "beta",
    "gamma",
    "delta",
    "Text",
    "UTF-8",
    "a\\*b",
    "snake\\_case",
    "x\\[1\\]",
    "&copy;",
    "AT\\&T",
    "1.5",
    "a\u{a0}b",
    "ä",
    "😀",
    "e.g.",
    "(see)",
    "a,b",
    "\"quoted\"",
    "it's",
    "50%",
    "x/y",
    "a-b",
    "issue\\#1",
    "\\$5",
    "a\\^b",
    "\\=\\=",
    "\\+\\+",
    "www\\.example.com",
    "http\\://example.com",
    "\\<tag\\>",
    "\\~x",
    "x\\!",
    "a\\@b",
    "\\|",
    "\\&#65;",
];

/// Words for emphasis without nesting: letters only.
const MD_PLAIN: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "Text", "item", "value", "x", "y", "word",
];

const MD_CODE: &[&str] = &[
    "`x`",
    "`let x = 1;`",
    "`` a`b ``",
    "`<b>`",
    "`**not**`",
    "`PROJ-1`",
    "`$x$`",
    "`a = b`",
];

const MD_MATH: &[&str] = &[
    "$x$",
    "$\\sqrt{x}$",
    "$a_b$",
    "$$x+1$$",
    "$`a$b`$",
    "$x^2$",
    "$`x`$",
];

const MD_IDS: &[&str] = &["PROJ-12", "MP-7", "MY\\_PROJ-12", "PROJ\\-9", "(PROJ-3)"];

const MD_URLS: &[&str] = &[
    "http://example.com",
    "https://example.com/a?b=1&c=2",
    "u",
    "#anchor",
    "http://example.com \"T\"",
    "<a b.html>",
];

/// A bare `www.` autolink takes in the delimiters and backslashes right after it, so a word
/// follows it.
const MD_AUTOLINKS: &[&str] = &["<https://a.b/c>", "www.example.com here", "<a@b.cd>"];

const MD_IMAGES: &[&str] = &[
    "![alt](i.png)",
    "![](workitemimg:1-a.png \"a.png\")",
    "![a b](a.png \"T\")",
];

/// Raw spans the conversion keeps as they are (keep mode only).
const RAW_SPANS: &[&str] = &[
    "<span style=\"color: #FF0000;\">",
    "<span style=\"font-family: Georgia, serif;\">",
    "<span style=\"background-color: #FF0000;\">",
];

const MD_CODE_LINES: &[&str] = &[
    "let x = 1;",
    "fn main() {}",
    "    indented",
    "",
    "a < b && c > d",
    "PROJ-1 **not bold** $x$",
    "<div>",
    "\ttab",
    "~~~",
    "x = y * 2",
];

const MD_MATH_LINES: &[&str] = &["a^2 + b^2", "\\frac{1}{2}", "x", "\\sqrt{y}", "a & b \\\\"];

const MD_DIAGRAMS: &[&str] = &[
    "graph TD\n    A-->B",
    "graph LR\n    X-->Y\n    Y-->Z",
    "sequenceDiagram\n    A->>B: hi",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Strong,
    Emph,
    Strike,
    Insert,
    Highlight,
    Link,
}

fn with(active: &[Style], style: Style) -> Vec<Style> {
    let mut styles = active.to_vec();
    styles.push(style);
    styles
}

/// Markdown made of the constructs the forward direction maps, as lines.
struct MarkdownGen<'r> {
    rng: &'r mut Rng,
    /// Formatting and links may hold other formatting (never of their own kind).
    nested: bool,
    /// Raw HTML may appear (keep mode).
    raw: bool,
}

impl MarkdownGen<'_> {
    fn document(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        for i in 0..1 + self.rng.below(5) {
            if i > 0 {
                lines.push(String::new());
            }
            lines.extend(self.block(0));
        }
        lines
    }

    fn block(&mut self, depth: usize) -> Vec<String> {
        let kinds = if depth > 1 { 4 } else { 15 };
        match self.rng.below(kinds) {
            0..=3 => self.paragraph(),
            4 => {
                // A heading is bold already.
                let level = 1 + self.rng.below(6);
                vec![format!(
                    "{} {}",
                    "#".repeat(level),
                    self.inline(0, &[Style::Strong])
                )]
            }
            5 => {
                let lang = self.rng.pick(&["", "", "rust", "text", "python", "js"]);
                let mut lines = vec![format!("```{lang}")];
                for _ in 0..1 + self.rng.below(3) {
                    lines.push(self.rng.pick(MD_CODE_LINES).to_owned());
                }
                lines.push("```".to_owned());
                lines
            }
            6 => {
                let mut lines = vec!["```math".to_owned()];
                for _ in 0..1 + self.rng.below(3) {
                    lines.push(self.rng.pick(MD_MATH_LINES).to_owned());
                }
                lines.push("```".to_owned());
                lines
            }
            7 => {
                let mut lines = vec!["```mermaid".to_owned()];
                lines.extend(self.rng.pick(MD_DIAGRAMS).lines().map(str::to_owned));
                lines.push("```".to_owned());
                lines
            }
            8 => {
                let mut inner = self.block(depth + 1);
                if self.rng.chance(3) {
                    inner.push(String::new());
                    inner.extend(self.block(depth + 1));
                }
                quoted(inner)
            }
            9 => {
                let kind = self
                    .rng
                    .pick(&["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"]);
                let mut lines = vec![format!("> [!{kind}]")];
                lines.extend(quoted(self.block(depth + 1)));
                lines
            }
            10 | 11 => self.list(depth),
            12 => self.table(),
            13 => vec!["-----".to_owned()],
            _ => {
                let image = self.rng.pick(MD_IMAGES).to_owned();
                if self.rng.chance(2) {
                    vec![format!("{image}\\"), self.inline(0, &[])]
                } else {
                    vec![image]
                }
            }
        }
    }

    /// One to three lines, joined by soft or hard breaks.
    fn paragraph(&mut self) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for _ in 0..1 + self.rng.below(3) {
            if self.rng.chance(2)
                && let Some(last) = lines.last_mut()
            {
                last.push('\\');
            }
            lines.push(self.inline(0, &[]));
        }
        lines
    }

    fn list(&mut self, depth: usize) -> Vec<String> {
        let ordered = self.rng.chance(2);
        let start = if self.rng.chance(4) { 3 } else { 1 };
        let task = self.rng.chance(3);
        let mut lines = Vec::new();
        for i in 0..1 + self.rng.below(3) {
            let marker = if ordered {
                format!("{}. ", start + i)
            } else {
                "- ".to_owned()
            };
            let indent = " ".repeat(marker.len());
            let mut body = self.block(depth + 1);
            // A task marker belongs to a paragraph: before another block (a table, a fence, ...)
            // it would turn that block into paragraph text.
            let check = if task && body.first().is_some_and(|line| starts_paragraph(line)) {
                self.rng.pick(&["[ ] ", "[x] "])
            } else {
                ""
            };
            if depth < 2 && self.rng.chance(4) {
                if self.rng.chance(2) {
                    body.push(String::new());
                }
                body.extend(self.list(depth + 1));
            }
            for (j, line) in body.into_iter().enumerate() {
                if j == 0 {
                    lines.push(format!("{marker}{check}{line}"));
                } else if line.is_empty() {
                    lines.push(line);
                } else {
                    lines.push(format!("{indent}{line}"));
                }
            }
        }
        lines
    }

    fn table(&mut self) -> Vec<String> {
        let columns = 1 + self.rng.below(3);
        let header: Vec<String> = (0..columns).map(|_| self.cell()).collect();
        let alignments: Vec<&str> = (0..columns)
            .map(|_| self.rng.pick(&["---", "---", ":--", ":-:", "--:"]))
            .collect();
        let mut lines = vec![row(&header), row(&alignments)];
        for _ in 0..self.rng.below(4) {
            let cells = match self.rng.below(6) {
                0 => columns + 1,
                1 => columns.saturating_sub(1).max(1),
                _ => columns,
            };
            let cells: Vec<String> = (0..cells).map(|_| self.cell()).collect();
            lines.push(row(&cells));
        }
        lines
    }

    fn cell(&mut self) -> String {
        if self.rng.chance(6) {
            return String::new();
        }
        let mut cell = self.inline(0, &[]);
        if self.raw && self.rng.chance(5) {
            cell.push_str("<br/>");
            cell.push_str(self.rng.pick(MD_PLAIN));
        }
        cell
    }

    fn words(&mut self) -> String {
        let n = 1 + self.rng.below(2);
        (0..n)
            .map(|_| self.rng.pick(MD_PLAIN))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// One to four inline items separated by spaces; `active`: the formatting around them.
    fn inline(&mut self, depth: usize, active: &[Style]) -> String {
        let n = 1 + self.rng.below(4);
        (0..n)
            .map(|_| self.item(depth, active))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn item(&mut self, depth: usize, active: &[Style]) -> String {
        let nest = self.nested && depth < 3;
        let in_link = active.contains(&Style::Link);
        match self.rng.below(18) {
            0..=6 => self.rng.pick(MD_WORDS).to_owned(),
            7 => self.rng.pick(MD_CODE).to_owned(),
            8 => self.rng.pick(MD_MATH).to_owned(),
            9 => self.rng.pick(MD_IDS).to_owned(),
            10 if !in_link => {
                let text = if nest {
                    self.inline(depth + 1, &with(active, Style::Link))
                } else {
                    self.words()
                };
                let url = self.rng.pick(MD_URLS);
                format!("[{text}]({url})")
            }
            11 if !in_link => self.rng.pick(MD_AUTOLINKS).to_owned(),
            12 => self.rng.pick(MD_IMAGES).to_owned(),
            13 if self.raw && !self.nested => {
                let open = self.rng.pick(RAW_SPANS);
                let words = self.words();
                format!("{open}{words}</span>")
            }
            14 => {
                let mark = self.rng.pick(&["^", "~"]);
                let word = self.rng.pick(MD_PLAIN);
                format!("{mark}{word}{mark}")
            }
            _ => {
                let free: Vec<Style> = [
                    Style::Strong,
                    Style::Emph,
                    Style::Strike,
                    Style::Insert,
                    Style::Highlight,
                ]
                .into_iter()
                .filter(|style| !active.contains(style))
                .collect();
                if free.is_empty() {
                    return self.rng.pick(MD_PLAIN).to_owned();
                }
                let style = free[self.rng.below(free.len())];
                let content = if nest {
                    self.inline(depth + 1, &with(active, style))
                } else {
                    self.words()
                };
                let mark = match style {
                    Style::Strong => "**",
                    Style::Emph => "*",
                    Style::Strike => "~~",
                    Style::Insert => "++",
                    Style::Highlight => "==",
                    Style::Link => unreachable!("links are made above"),
                };
                format!("{mark}{content}{mark}")
            }
        }
    }
}

/// Lines of a block inside a quote.
fn quoted(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| {
            if line.is_empty() {
                ">".to_owned()
            } else {
                format!("> {line}")
            }
        })
        .collect()
}

/// Whether a generated block starting with this line is a paragraph (or an image line).
fn starts_paragraph(line: &str) -> bool {
    let ordered = line.split_once(". ").is_some_and(|(number, _)| {
        !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
    });
    !(ordered
        || ["|", "```", ">", "#", "- ", "-----"]
            .iter()
            .any(|p| line.starts_with(p)))
}

/// A table row of these cells.
fn row<S: AsRef<str>>(cells: &[S]) -> String {
    let cells: Vec<&str> = cells.iter().map(AsRef::as_ref).collect();
    format!("| {} |", cells.join(" | "))
}

fn markdown_case(rng: &mut Rng, nested: bool) -> Case {
    let keep = !rng.chance(3);
    let pieces = MarkdownGen {
        rng: &mut *rng,
        nested,
        raw: keep,
    }
    .document();
    let work_items = if rng.chance(4) {
        WorkItemIds::Prefixes(vec!["PROJ".into(), "MP".into()])
    } else {
        WorkItemIds::Any
    };
    Case {
        pieces,
        separator: "\n",
        options: ToMarkdownOptions {
            raw_html: if keep { RawHtml::Keep } else { RawHtml::Drop },
            work_items,
        },
        attach: rng.chance(2),
    }
}

/// The generators and the shrinker themselves (they run without the conversion to Markdown).
mod tools {
    use super::*;

    #[test]
    fn shrinking_keeps_what_the_failure_needs() {
        let pieces: Vec<String> = "abcdefghij".chars().map(String::from).collect();
        let fails = |p: &[String]| {
            let s = p.concat();
            s.contains('c') && s.contains('h')
        };
        assert_eq!(shrink(pieces, &fails).concat(), "ch");
        let fails = |_: &[String]| true;
        assert!(shrink(vec!["x".to_owned(); 5], &fails).is_empty());
    }

    #[test]
    fn panics_are_caught_and_recorded() {
        let boom = || -> u8 { panic!("boom") };
        let message = quietly(boom).unwrap_err();
        assert!(message.contains("boom"), "{message}");
        assert_eq!(quietly(|| 7).unwrap(), 7);
    }

    #[test]
    fn differences_are_located() {
        assert!(difference("abc", "abd").contains("byte 2"));
        assert!(difference("ab", "abc").contains("byte 2"));
        assert!(difference("ä€x", "ä€y").contains("byte 5"));
    }

    /// The generators are deterministic, make the shapes they should, and the Markdown they make
    /// converts to HTML without trouble.
    #[test]
    fn generators_are_deterministic_and_varied() {
        let html: Vec<String> = (0..200)
            .map(|case| html_case(&mut Rng::new(case_seed(1, case))).pieces.concat())
            .collect();
        let again: Vec<String> = (0..200)
            .map(|case| html_case(&mut Rng::new(case_seed(1, case))).pieces.concat())
            .collect();
        assert_eq!(html, again);
        let all = html.concat();
        for shape in [
            "<table",
            "<ul>",
            "<pre",
            "polarion-rte-link",
            "polarion-rte-formula",
            "<img",
            "margin-left: 40px",
            "<br/>",
            "&nbsp;",
            "font-size",
        ] {
            assert!(all.contains(shape), "no {shape} in 200 cases");
        }
        let longest = html.iter().map(String::len).max().unwrap_or(0);
        assert!(longest < 64 * 1024, "{longest} bytes");

        let mut markdown = String::new();
        for case in 0..200 {
            for nested in [false, true] {
                let case = markdown_case(&mut Rng::new(case_seed(2, case)), nested);
                let text = case.pieces.join("\n");
                let _ = to_html(&text, &ToHtmlOptions::default());
                markdown.push_str(&text);
            }
        }
        for shape in ["**", "| ", "> [!", "```mermaid", "- ", "$", "](", "#"] {
            assert!(markdown.contains(shape), "no {shape} in 400 cases");
        }
    }
}
