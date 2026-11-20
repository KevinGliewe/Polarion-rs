//! The mermaid module: hashes and file names, the diagrams of a document, the source embedded in
//! the SVG, rendering (feature `mermaid`) and legacy mermaid.ink URLs.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use polarion_markdown::mermaid::{self, Diagram, MermaidError, NAMESPACE};

const SIMPLE: &str = "graph TD\n    A-->B";

/// An SVG as a renderer writes it.
const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"20\" viewBox=\"0 0 10 20\"><rect x=\"0\" y=\"0\" width=\"10\" height=\"20\" fill=\"#FFFFFF\"/></svg>";

#[test]
fn pinned_hash_and_file_name() {
    assert_eq!(mermaid::hash(SIMPLE), "744d43fa10d8c687");
    assert_eq!(
        mermaid::file_name("744d43fa10d8c687"),
        "mermaid-744d43fa10d8c687.svg"
    );
    assert_eq!(
        mermaid::diagram(SIMPLE),
        Diagram {
            source: SIMPLE.to_owned(),
            hash: "744d43fa10d8c687".to_owned(),
            file_name: "mermaid-744d43fa10d8c687.svg".to_owned(),
        }
    );
}

#[test]
fn normalization_variants_hash_equal() {
    for variant in [
        "graph TD\r\n    A-->B",
        "graph TD\r    A-->B",
        "graph TD   \n    A-->B\t \t",
        "\n\n  \t\ngraph TD\n    A-->B\n\n \n",
        "graph TD\n    A-->B\n",
        "\r\ngraph TD  \r\n    A-->B  \r\n\r\n",
    ] {
        assert_eq!(mermaid::normalize(variant), SIMPLE, "{variant:?}");
        assert_eq!(mermaid::hash(variant), "744d43fa10d8c687", "{variant:?}");
        assert_eq!(mermaid::diagram(variant), mermaid::diagram(SIMPLE));
    }
    // Indentation and inner blank lines belong to the source.
    for different in [
        "graph TD\n  A-->B",
        "graph TD\n\n    A-->B",
        "  graph TD\n    A-->B",
    ] {
        assert_ne!(
            mermaid::hash(different),
            mermaid::hash(SIMPLE),
            "{different:?}"
        );
    }
    assert_eq!(mermaid::normalize(" \n\t\r\n"), "");
}

#[test]
fn diagrams_in_document_order_once_each() {
    let markdown = "\
# Title

```mermaid
graph TD
    A-->B
```

``` Mermaid
sequenceDiagram
  A->>B: hi
```

```mermaid
```

```mermaid
  \t
```

```js
let x = 1;
```

- item

  ```MERMAID title
  graph TD
      A-->B
  ```

> ~~~mermaid
> pie
>   \"a\" : 1
> ~~~

```mermaidjs
graph LR
  X-->Y
```

    indented code, not a fence
";
    let found = mermaid::diagrams(markdown);
    let sources: Vec<&str> = found.iter().map(|d| d.source.as_str()).collect();
    assert_eq!(
        sources,
        [SIMPLE, "sequenceDiagram\n  A->>B: hi", "pie\n  \"a\" : 1"]
    );
    for d in &found {
        assert_eq!(*d, mermaid::diagram(&d.source));
    }
    assert!(mermaid::diagrams("no diagrams\n\n```\nplain\n```\n").is_empty());
}

/// What `diagrams` finds is what `to_html_with_diagrams` writes as images: a block nested so deep
/// that the conversion writes it as text is not uploaded for nothing.
#[test]
fn diagrams_are_the_blocks_written_as_images() {
    let quoted = |levels: usize| {
        format!("```mermaid\n{SIMPLE}\n```")
            .lines()
            .map(|line| format!("{}{line}", "> ".repeat(levels)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let options = polarion_markdown::ToHtmlOptions::default();
    for levels in [0, 3, 20] {
        let markdown = quoted(levels);
        let found = mermaid::diagrams(&markdown);
        assert_eq!(found, [mermaid::diagram(SIMPLE)], "{levels}");
        let attached = [(found[0].file_name.clone(), "1-a.svg".to_owned())].into();
        let html = polarion_markdown::to_html_with_diagrams(&markdown, &options, &attached);
        assert!(
            html.contains("src=\"workitemimg:1-a.svg\""),
            "{levels}: {html}"
        );
    }
    let markdown = quoted(100);
    assert!(mermaid::diagrams(&markdown).is_empty());
    let all = [(mermaid::diagram(SIMPLE).file_name, "1-a.svg".to_owned())].into();
    let html = polarion_markdown::to_html_with_diagrams(&markdown, &options, &all);
    assert!(!html.contains("<img"), "{html}");
}

/// Characters XML does not allow are replaced before hashing, as in the HTML and the embedded
/// source, so a diagram read back from its SVG hashes as it did when it was written.
#[test]
fn normalization_replaces_characters_xml_does_not_allow() {
    let source = "graph TD\n    A[first\u{b}second] --> B[page\u{c}break\u{0}]";
    let normalized = mermaid::normalize(source);
    assert_eq!(
        normalized,
        "graph TD\n    A[first\u{fffd}second] --> B[page break\u{fffd}]"
    );
    let svg = mermaid::embed_source(SVG, &normalized);
    let back = mermaid::source_of(&svg).expect("embedded source");
    assert_eq!(back, normalized);
    assert_eq!(mermaid::hash(&back), mermaid::hash(source));
}

#[test]
fn diagram_file_names() {
    assert!(mermaid::is_diagram_file("mermaid-0123456789abcdef.svg"));
    assert!(mermaid::is_diagram_file(
        &mermaid::diagram(SIMPLE).file_name
    ));
    for name in [
        "mermaid-0123456789ABCDEF.svg",
        "mermaid-0123456789abcde.svg",
        "mermaid-0123456789abcdef0.svg",
        "mermaid-0123456789abcdeg.svg",
        "mermaid-0123456789abcdef.SVG",
        "mermaid-0123456789abcdef.png",
        "mermaid-0123456789abcdef.svg.svg",
        "Mermaid-0123456789abcdef.svg",
        "1-mermaid-0123456789abcdef.svg",
        "mermaid-.svg",
        "diagram.svg",
        "",
    ] {
        assert!(!mermaid::is_diagram_file(name), "{name}");
    }
}

#[test]
fn source_is_embedded_after_the_root_start_tag() {
    let metadata = |escaped: &str| {
        format!(
            "<metadata><mermaid xmlns=\"{NAMESPACE}\" version=\"1\">{escaped}</mermaid></metadata>"
        )
    };
    let source = "a<b&c>d \"q\" 'a'";
    let escaped = "a&lt;b&amp;c&gt;d \"q\" 'a'";
    assert_eq!(
        mermaid::embed_source(SVG, source),
        SVG.replacen(
            "viewBox=\"0 0 10 20\">",
            &format!("viewBox=\"0 0 10 20\">{}", metadata(escaped)),
            1
        )
    );
    // The start tag ends at the first `>` outside quotes.
    assert_eq!(
        mermaid::embed_source("<svg a=\"x>y\" b='>'><g/></svg>", "S"),
        format!("<svg a=\"x>y\" b='>'>{}<g/></svg>", metadata("S"))
    );
    // A self-closing root gets an end tag.
    assert_eq!(
        mermaid::embed_source("<svg xmlns=\"http://www.w3.org/2000/svg\"/>", "S"),
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\">{}</svg>",
            metadata("S")
        )
    );
    assert_eq!(
        mermaid::embed_source("<s:svg a=\"1\" />", "S"),
        format!("<s:svg a=\"1\" >{}</s:svg>", metadata("S"))
    );
    // A prolog comes before the root.
    let prolog = "<?xml version=\"1.0\"?>\n<!-- generated -->\n<!DOCTYPE svg>\n";
    assert_eq!(
        mermaid::embed_source(&format!("{prolog}<svg><g/></svg>"), "S"),
        format!("{prolog}<svg>{}<g/></svg>", metadata("S"))
    );
    // A carriage return survives XML parsers as a reference; characters XML does not allow are
    // replaced.
    assert!(mermaid::embed_source(SVG, "a\r\nb").contains(">a&#13;\nb</mermaid>"));
    assert_eq!(
        mermaid::source_of(&mermaid::embed_source(SVG, "a\u{1}b\u{FFFF}c\u{0}")).as_deref(),
        Some("a\u{FFFD}b\u{FFFD}c\u{FFFD}")
    );
    // Without a root start tag there is no place for it.
    for input in ["", "plain text", "<!-- only a comment -->", "<svg a=\"1\""] {
        assert_eq!(mermaid::embed_source(input, "S"), input);
    }
}

/// Sources that need escaping or look like markup.
const TRICKY: &[&str] = &[
    SIMPLE,
    "graph TD\n  A[\"x < y\"] --> B[\"a & b\"]",
    "flowchart LR\n  A[\"He said #quot;hi#quot;\"] --> B['single']",
    "flowchart LR\n  A[\"]]> and <![CDATA[ x ]]>\"] --> B",
    "flowchart LR\n  A[\"line<br>break\"] --> B[\"other<br/>one\"]",
    "graph TD\n  A[Grüße, 日本語, emoji 🎉] --> B",
    "graph TD\n  A[\"&amp; &lt; &#38; &#x26; &quot; &nbsp;\"] --> B",
    "%%{init: {\"theme\": \"dark\"}}%%\ngraph TD\n\tA-->B",
    "sequenceDiagram\n  Alice->>Bob: </mermaid></metadata><mermaid xmlns=\"urn:polarion-rs:mermaid\" version=\"1\">fake",
    "a\n\nb",
    "x\u{85}y\u{2028}z\u{a0}",
];

#[test]
fn embedded_source_round_trips() {
    for source in TRICKY {
        let svg = mermaid::embed_source(SVG, source);
        assert_eq!(mermaid::source_of(&svg).as_deref(), Some(*source), "{svg}");
        // The source is character data: nothing in it is markup.
        let start = svg.find("version=\"1\">").unwrap() + "version=\"1\">".len();
        let end = svg.find("</mermaid>").unwrap();
        assert!(!svg[start..end].contains('<'), "{svg}");
    }
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn every_source_of_xml_characters_round_trips() {
    let alphabet: Vec<char> = "ab Z09<>&;#\"'=/\\[]{}!-%:\t\n\r\u{a0}é😀?xX"
        .chars()
        .collect();
    let mut state = 0x2545_F491_4F6C_DD1D;
    for _ in 0..2_000 {
        let len = (xorshift(&mut state) % 60) as usize;
        let raw: String = (0..len)
            .map(|_| alphabet[(xorshift(&mut state) % alphabet.len() as u64) as usize])
            .collect();
        for source in [mermaid::normalize(&raw), raw.clone()] {
            let svg = mermaid::embed_source(SVG, &source);
            assert_eq!(
                mermaid::source_of(&svg).as_deref(),
                Some(source.as_str()),
                "{raw:?}"
            );
        }
    }
}

#[test]
fn source_of_finds_the_element_by_string_search() {
    let ns = NAMESPACE;
    let cases: Vec<(String, Option<&str>)> = vec![
        // The rest of the document need not be well-formed.
        (
            format!(
                "<svg <g unclosed=\"x><metadata><mermaid xmlns=\"{ns}\" version=\"1\">A--&gt;B</mermaid>"
            ),
            Some("A-->B"),
        ),
        // Attribute order and quotes do not matter, nor whitespace in the end tag.
        (
            format!("<mermaid version='1' xmlns='{ns}'>x</mermaid >"),
            Some("x"),
        ),
        (
            format!(
                "<mermaid xmlns=\"{ns}\" version=\"1\">&lt;&gt;&amp;&quot;&apos;&#65;&#x42;</mermaid>"
            ),
            Some("<>&\"'AB"),
        ),
        (
            format!("<mermaid xmlns=\"{ns}\" version=\"1\">a<![CDATA[<b> & c]]>d</mermaid>"),
            Some("a<b> & cd"),
        ),
        (format!("<mermaid xmlns=\"{ns}\" version=\"1\"/>"), Some("")),
        // The first one of ours wins.
        (
            format!(
                "<mermaid xmlns=\"other\" version=\"1\">no</mermaid><mermaid xmlns=\"{ns}\" version=\"1\">yes</mermaid><mermaid xmlns=\"{ns}\" version=\"1\">later</mermaid>"
            ),
            Some("yes"),
        ),
        // Not ours, or not complete.
        (
            format!("<mermaid xmlns=\"{ns}\" version=\"2\">x</mermaid>"),
            None,
        ),
        (format!("<mermaid xmlns=\"{ns}\">x</mermaid>"), None),
        (
            format!("<mermaidx xmlns=\"{ns}\" version=\"1\">x</mermaidx>"),
            None,
        ),
        (format!("<mermaid xmlns=\"{ns}\" version=\"1\">x"), None),
        (
            format!("<mermaid xmlns=\"{ns}\" version=\"1\">x</mermaidx>"),
            None,
        ),
        (
            format!("<mermaid xmlns=\"{ns}\" version=\"1\">a<b/>c</mermaid>"),
            None,
        ),
        (format!("<mermaid xmlns=\"{ns}\" version=\"1\""), None),
        (SVG.to_owned(), None),
        (String::new(), None),
    ];
    for (svg, expected) in cases {
        assert_eq!(mermaid::source_of(&svg).as_deref(), expected, "{svg}");
    }
}

#[cfg(feature = "mermaid")]
mod rendering {
    use std::fmt::Write as _;

    use super::*;

    fn render(source: &str) -> String {
        mermaid::render_svg(source).unwrap_or_else(|e| panic!("{source:?}: {e}"))
    }

    /// Whether XML 1.0 allows the character in a document.
    fn is_xml_char(c: char) -> bool {
        matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
    }

    /// The SVG without the embedded source.
    fn markup(svg: &str) -> String {
        let start = svg.find("<metadata>").expect("metadata");
        let end = svg.find("</metadata>").expect("metadata end") + "</metadata>".len();
        format!("{}{}", &svg[..start], &svg[end..])
    }

    /// The attributes of the first element with `id` in `markup`, e.g. a gantt task's bar.
    fn element<'a>(markup: &'a str, id: &str) -> &'a str {
        let at = markup
            .find(&format!(" id=\"{id}\""))
            .unwrap_or_else(|| panic!("no element {id}: {markup}"));
        let start = markup[..at].rfind('<').unwrap();
        let end = at + markup[at..].find('>').unwrap();
        &markup[start..end]
    }

    /// A number attribute of an element as [`element`] returns it.
    fn number(element: &str, name: &str) -> f64 {
        let from = element
            .find(&format!(" {name}=\""))
            .unwrap_or_else(|| panic!("no {name}: {element}"))
            + name.len()
            + 3;
        let to = from + element[from..].find('"').unwrap();
        element[from..to].parse().unwrap()
    }

    #[test]
    fn output_is_svg_with_the_source() {
        let svg = render(SIMPLE);
        assert!(svg.starts_with("<svg "), "{svg}");
        assert!(svg.ends_with("</svg>"), "{svg}");
        let root_end = svg.find('>').unwrap() + 1;
        assert!(
            svg[root_end..].starts_with(&format!(
                "<metadata><mermaid xmlns=\"{NAMESPACE}\" version=\"1\">graph TD\n    A--&gt;B</mermaid></metadata>"
            )),
            "{svg}"
        );
        assert_eq!(mermaid::source_of(&svg).as_deref(), Some(SIMPLE));
    }

    #[test]
    fn the_image_has_a_size_of_its_own() {
        // Mermaid sizes a diagram to its container (`width="100%"`); an attached image has none.
        for source in [
            SIMPLE,
            "sequenceDiagram\n  Alice->>Bob: hi",
            "gantt\n  Build :a1, 2024-01-01, 5d",
            "journey\n  title My day\n  section Go to work\n    Make tea: 5: Me",
            "pie title Pets\n  \"Dogs\" : 386\n  \"Cats\" : 85",
        ] {
            let svg = render(source);
            let root = &svg[..svg.find('>').unwrap()];
            assert!(!root.contains('%'), "{root}");
            assert!(number(root, "width") > 10.0, "{root}");
            assert!(number(root, "height") > 10.0, "{root}");
        }
    }

    #[test]
    fn diagram_kinds_render() {
        for (source, text) in [
            (
                "flowchart LR\n  A[Start] --> B{Decision}\n  B -->|Yes| C[OK]\n  B -->|No| D[Cancel]",
                "Decision",
            ),
            // A participant the diagram does not declare, and `->>+`.
            (
                "sequenceDiagram\n  participant Alice\n  Alice->>+Bob: Hello\n  Bob-->>-Alice: Hi",
                "Hello",
            ),
            (
                "classDiagram\n  Animal <|-- Duck\n  Animal : +int age\n  Duck : +swim()",
                "Duck",
            ),
            (
                "stateDiagram-v2\n  [*] --> Still\n  Still --> Moving\n  Moving --> [*]",
                "Moving",
            ),
            ("pie title Pets\n  \"Dogs\" : 386\n  \"Cats\" : 85", "Dogs"),
            ("erDiagram\n  CUSTOMER ||--o{ ORDER : places", "CUSTOMER"),
        ] {
            let svg = render(source);
            assert!(svg.starts_with("<svg "), "{source:?}");
            assert!(markup(&svg).contains(text), "{source:?}: {svg}");
            assert_eq!(mermaid::source_of(&svg).as_deref(), Some(source));
        }
    }

    #[test]
    fn renders_are_deterministic() {
        // Text is measured from built-in metrics, and neither the clock nor the fonts of the
        // machine are used.
        for source in [
            "flowchart TD\n  A[Start] --> B{Is it?}\n  B -->|Yes| C[OK]\n  C --> D[Rethink]\n  D --> B\n  B ---->|No| E[End]",
            "gantt\n  Build :a1, 2024-01-01, 5d\n  Test :after a1, 2d",
        ] {
            assert_eq!(render(source), render(source));
        }
    }

    #[test]
    fn labels_are_svg_text() {
        // Not HTML in a `foreignObject`, which only a browser draws.
        let svg = render("flowchart LR\n  A[\"Grüße <b>bold</b> & more\"] --> B{{日本語}}");
        let markup = markup(&svg);
        assert!(!markup.contains("foreignObject"), "{markup}");
        assert!(
            !markup.contains("<div") && !markup.contains("<span"),
            "{markup}"
        );
        for text in ["Grüße", "bold", "more", "日本語"] {
            assert!(markup.contains(text), "{text}: {markup}");
        }
    }

    #[test]
    fn gantt_charts_follow_their_dates() {
        // Issue 3: starts and ends as numbers (`dateFormat x`). The renderer before placed
        // every task after the one before, three days long, without an error.
        let svg = render(
            "gantt\n    title Two rows that overlap\n    dateFormat x\n    axisFormat %s\n    \
             todayMarker off\n    section A\n    Init     :a1, 0, 3000\n    Execute  :crit, a2, \
             3000, 13000\n    section B\n    Init     :b1, 0, 3000\n    Execute  :crit, b2, \
             13000, 23000",
        );
        let chart = markup(&svg);
        let bar = |id: &str| {
            let bar = element(&chart, &format!("merman-{id}"));
            (number(bar, "x"), number(bar, "width"))
        };
        let (a1, a2, b1, b2) = (bar("a1"), bar("a2"), bar("b1"), bar("b2"));
        // `a1` and `b1` are the same three units from the start.
        assert_eq!(a1, b1);
        // `a2` follows `a1` and is ten units long: ten thirds of `a1`.
        assert!((a2.0 - (a1.0 + a1.1)).abs() < 1.5, "{a1:?} {a2:?}");
        assert!((a2.1 / a1.1 - 10.0 / 3.0).abs() < 0.1, "{a1:?} {a2:?}");
        // `b2` starts where `a2` ends and is as long.
        assert!((b2.0 - (a2.0 + a2.1)).abs() < 1.5, "{a2:?} {b2:?}");
        assert!((b2.1 - a2.1).abs() < 1.5, "{a2:?} {b2:?}");

        // Dates with a format of their own, an end date instead of a duration, `after`.
        let svg = render(
            "gantt\n  dateFormat DD.MM.YYYY\n  Build :a1, 01.01.2024, 10d\n  Test :a2, \
             11.01.2024, 21.01.2024\n  Ship :a3, after a2, 5d",
        );
        let chart = markup(&svg);
        let bar = |id: &str| {
            let bar = element(&chart, &format!("merman-{id}"));
            (number(bar, "x"), number(bar, "width"))
        };
        let (a1, a2, a3) = (bar("a1"), bar("a2"), bar("a3"));
        assert!((a2.0 - (a1.0 + a1.1)).abs() < 1.5, "{a1:?} {a2:?}");
        assert!((a2.1 - a1.1).abs() < 1.5, "{a1:?} {a2:?}");
        assert!((a3.0 - (a2.0 + a2.1)).abs() < 1.5, "{a2:?} {a3:?}");
        assert!((a3.1 / a1.1 - 0.5).abs() < 0.05, "{a1:?} {a3:?}");
    }

    #[test]
    fn invalid_sources() {
        for (source, message) in [
            ("this is not a diagram", "No Mermaid diagram type detected"),
            ("", "No Mermaid diagram type detected"),
            (
                "sequenceDiagramm\n  participant Alice\n  Alice->>Bob: hi",
                "Diagram parse error (sequence)",
            ),
            (
                "flowchart LR\n  A --> ",
                "Diagram parse error (flowchart-v2)",
            ),
            // The former renderer panicked on these.
            ("flowchart LR\n  A[/]", "Diagram parse error (flowchart-v2)"),
            (
                "flowchart LR\n  A[\\]",
                "Diagram parse error (flowchart-v2)",
            ),
        ] {
            match mermaid::render_svg(source) {
                Err(MermaidError::Invalid(found)) => {
                    assert!(found.contains(message), "{source:?}: {found}");
                }
                other => panic!("{source:?}: {other:?}"),
            }
        }
        render("sequenceDiagram\n  participant Alice\n  Alice->>Bob: hi");
    }

    #[test]
    fn what_the_build_leaves_out_is_refused() {
        // Formulas (`math`) are not built in: an error, not a diagram without them.
        match mermaid::render_svg("graph TD\n A[\"$$\\sqrt{x}$$\"] --> B") {
            Err(MermaidError::Invalid(message)) => {
                assert!(message.contains("lacks capability"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn characters_xml_does_not_allow_never_reach_the_svg() {
        // A vertical tab is the soft line break of office documents, so a pasted label can hold
        // one. The embedded source has U+FFFD for each, as its normalized form has.
        for source in [
            "graph TD\n  A[first\u{b}second] --> B[page\u{c}break]",
            "sequenceDiagram\n  Alice->>Bob: form\u{c}feed and \u{1}control",
            "pie title Pets\u{c}and\u{FFFF}more\n  \"Dogs\" : 386",
        ] {
            let svg = render(source);
            assert!(svg.chars().all(is_xml_char), "{source:?}: {svg}");
            let replaced: String = source
                .chars()
                .map(|c| if is_xml_char(c) { c } else { '\u{FFFD}' })
                .collect();
            assert_eq!(mermaid::source_of(&svg), Some(replaced));
        }
    }

    #[test]
    fn deep_nesting_is_refused() {
        // Nesting a parser could follow by recursion until the stack overflows, which aborts
        // the process: the renderer refuses it.
        let n = 8_000;
        let (open, close) = ("[".repeat(n), "]".repeat(n));
        for source in [
            format!("%%{{init:{{\"x\":{open}{close}}}}}%%\ngraph TD\n A-->B"),
            // Not closed: a parser fails only at the deepest level.
            format!("%%{{init:{{\"x\":{open}}}}}%%\ngraph TD\n A-->B"),
            format!("%%{{init: {open}{close}}}%%\ngraph TD\n A-->B"),
        ] {
            let message = too_large(&source);
            assert!(message.contains("nesting exceeds"), "{message}");
        }
        // Within its limit.
        let brackets = format!(
            "%%{{init: {{\"theme\": \"dark\", \"x\": {}{}}}}}%%\ngraph TD\n A-->B",
            "[".repeat(62),
            "]".repeat(62)
        );
        assert_eq!(
            markup(&render(&brackets)),
            markup(&render(
                "%%{init: {\"theme\": \"dark\"}}%%\ngraph TD\n A-->B"
            ))
        );
    }

    #[test]
    fn the_renderer_does_not_run_on_the_callers_stack() {
        // Nested C4 boundaries, on a thread with a small stack (a Windows main thread has
        // 1 MiB, workers of a runtime often less): the render thread has its own.
        let mut source = String::from("C4Context\n");
        for i in 0..499 {
            let _ = writeln!(source, "Boundary(b{i}, \"B{i}\") {{");
        }
        source.push_str("Person(p, \"P\")\n");
        source.push_str(&"}\n".repeat(499));
        let svg = std::thread::Builder::new()
            .stack_size(256 << 10)
            .spawn(move || mermaid::render_svg(&source))
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
        assert!(svg.contains("B498"));
    }

    #[test]
    fn diagrams_that_would_take_long_are_refused() {
        // The renderer's own limits answer in milliseconds; none of these gets to the deadline.
        let started = std::time::Instant::now();
        let chain = |edges: usize| {
            let mut source = String::from("flowchart TD\n");
            for i in 0..edges {
                let _ = writeln!(source, "  n{i} --> n{}", i + 1);
            }
            source
        };
        // 500 edges in a flowchart.
        assert!(too_large(&chain(501)).contains("edge limit exceeded"));
        render(&chain(300));
        // `&` lists make an edge for every pair: these 14 KB would make a million.
        let list = |prefix: &str, len: usize| {
            (0..len)
                .map(|i| format!("{prefix}{i}"))
                .collect::<Vec<_>>()
                .join(" & ")
        };
        let lists = format!(
            "flowchart TD\n  {} --> {}",
            list("a", 1000),
            list("b", 1000)
        );
        assert!(
            too_large(&lists).contains("1000000 edges"),
            "{}",
            too_large(&lists)
        );
        render(&format!(
            "flowchart TD\n  {} --> {}",
            list("a", 5),
            list("b", 5)
        ));
        // Thousands of nodes in hundreds of nested groups: the former renderer took minutes.
        let nodes = |count: usize| {
            (0..count)
                .map(|i| format!("a{i}"))
                .collect::<Vec<_>>()
                .join("-->")
        };
        let nested = format!(
            "flowchart TD\n{}{}\n{}",
            "subgraph s\n".repeat(450),
            nodes(5_000),
            "end\n".repeat(450)
        );
        let joined = format!(
            "flowchart TD\n{}\n{}\n{}",
            vec!["subgraph s"; 2_000].join("; "),
            nodes(1_150),
            vec!["end"; 2_000].join("; ")
        );
        let transitions = (0..2_400)
            .map(|i| format!("a{i} --> a{}", i + 1))
            .collect::<Vec<_>>()
            .join("; ");
        let states = format!(
            "stateDiagram-v2\n{}{transitions}\n{}",
            "state S {\n".repeat(450),
            "}\n".repeat(450)
        );
        for source in [nested, joined, states] {
            assert!(source.len() <= 50_000 && source.lines().count() <= 1_000);
            let message = too_large(&source);
            assert!(
                message.contains("limit exceeded") || message.contains("resource limit"),
                "{message}"
            );
        }
        // Diagrams of the sizes people write.
        let mut messages = String::from("sequenceDiagram\n");
        let mut tasks =
            String::from("gantt\n  dateFormat YYYY-MM-DD\n  section S\n  t0 :a0, 2024-01-01, 1d\n");
        for i in 1..=400 {
            let _ = writeln!(messages, "  Alice->>Bob: m{i}");
            let _ = writeln!(tasks, "  t{i} :a{i}, after a{}, 1d", i - 1);
        }
        render(&messages);
        render(&tasks);
        let mut source = String::from("flowchart TD\n");
        for i in 0..12 {
            let _ = writeln!(source, "subgraph s{i}\n  n{i} --> m{i} & k{i}");
        }
        source.push_str(&"end\n".repeat(12));
        for i in 0..150 {
            let _ = writeln!(source, "n{} --> x{i}", i % 12);
        }
        render(&source);
        let mut states = String::from("stateDiagram-v2\n");
        for i in 0..8 {
            let _ = writeln!(states, "state C{i} {{\n  [*] --> s{i}\n  s{i} --> t{i}");
        }
        states.push_str(&"}\n".repeat(8));
        render(&states);
        // Far from the 20 s deadline, also in a debug build.
        assert!(started.elapsed().as_secs() < 120, "{:?}", started.elapsed());
    }

    fn too_large(source: &str) -> String {
        match mermaid::render_svg(source) {
            Err(MermaidError::TooLarge(message)) => message,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn size_limits() {
        let bytes = |len: usize| format!("{SIMPLE}\n%%{}", "x".repeat(len - SIMPLE.len() - 3));
        assert_eq!(bytes(50_000).len(), 50_000);
        assert_eq!(
            mermaid::source_of(&render(&bytes(50_000))).as_deref(),
            Some(bytes(50_000).as_str())
        );
        assert!(too_large(&bytes(50_001)).contains("limit of 50000"));
        let lines = |count: usize| format!("{SIMPLE}{}", "\n%% comment".repeat(count - 2));
        assert_eq!(lines(1_000).lines().count(), 1_000);
        render(&lines(1_000));
        assert!(too_large(&lines(1_001)).contains("limit of 1000"));
        assert!(too_large(&"\n".repeat(1_001)).contains("limit of 1000"));
    }

    #[test]
    fn dangerous_click_links_are_dropped() {
        let source = "flowchart LR\n  A-->B\n  A-->C\n  click A \"javascript:alert(1)\" \"tip\"\n  click B href \"https://example.com/?a=1&b=2\" _blank\n  click C \"java\tscript:alert(2)\"\n  click A call alert(3)";
        let svg = render(source);
        let markup = markup(&svg);
        for needle in ["script:", "alert", " on"] {
            assert!(!markup.contains(needle), "{needle}: {markup}");
        }
        assert!(
            markup.contains("href=\"https://example.com/?a=1&amp;b=2\""),
            "{markup}"
        );
        assert_eq!(mermaid::source_of(&svg).as_deref(), Some(source));
    }

    #[test]
    fn markup_in_the_source_stays_text() {
        // Labels, style values, class definitions, directives: wherever a source can put text,
        // markup in it is written as text or dropped, and nothing in the SVG runs or loads.
        for source in [
            "flowchart LR\n  A[\"<img src=x onerror=alert(1)><script>alert(2)</script><b>bold</b>\"] --> B",
            "flowchart LR\n  A-->B\n  style A fill:red\"><script>alert(1)</script><rect x=\"",
            "flowchart LR\n  A-->B\n  style A fill:red\" onmouseover=\"alert(1)\n  linkStyle 0 stroke:red\" onclick=\"alert(3)",
            "flowchart LR\n  A-->B\n  classDef c fill:red\" onload=\"alert(1)\n  class A c",
            "%%{init: {\"fontFamily\": \"x;} </style><script>alert(1)</script>\"}}%%\ngraph TD\n A-->B",
            "%%{init: {\"themeCSS\": \".node rect { fill: url(https://evil.example/c.png) } @import url(https://evil.example/i.css);\"}}%%\ngraph TD\n A-->B",
            "%%{init: {\"themeVariables\": {\"primaryColor\": \"url(https://evil.example/c.png)\"}}}%%\ngraph TD\n A-->B",
            "sequenceDiagram\n  Alice->>Bob: <script>alert(1)</script><img src=x onerror=alert(2)>",
            "flowchart LR\n  A@{ img: \"https://evil.example/t.png\", label: \"x\", w: 60, h: 60 } --> B",
        ] {
            let markup = match mermaid::render_svg(source) {
                Ok(svg) => {
                    assert_eq!(mermaid::source_of(&svg).as_deref(), Some(source));
                    markup(&svg)
                }
                // Refused is as good.
                Err(MermaidError::Invalid(_)) => continue,
                Err(other) => panic!("{source:?}: {other}"),
            };
            // No element that runs or loads, no event handler attribute, no address outside.
            for needle in [
                "<script",
                "<img",
                "<foreignObject",
                "<iframe",
                "evil.example",
            ] {
                assert!(!markup.contains(needle), "{source:?} {needle}: {markup}");
            }
            // Outside the style sheet (CSS text, where a `"` is a character), every `="` starts
            // an attribute value: inside values the quotes are escaped.
            let sheet = markup.find("<style>").map_or(0..0, |from| {
                let to = markup[from..]
                    .find("</style>")
                    .map_or(markup.len(), |to| from + to);
                from..to
            });
            let outside = format!("{}{}", &markup[..sheet.start], &markup[sheet.end..]);
            for (at, _) in outside.match_indices("=\"") {
                let name = outside[..at]
                    .rsplit(|c: char| c.is_whitespace())
                    .next()
                    .unwrap();
                assert!(
                    !name.to_ascii_lowercase().starts_with("on"),
                    "{source:?}: {name}"
                );
            }
        }
    }

    #[test]
    fn init_directive_selects_the_theme() {
        let diagram = "flowchart TD\n    A-->B";
        let with_theme = |theme: &str| {
            markup(&render(&format!(
                "%%{{init: {{\"theme\": \"{theme}\"}}}}%%\n{diagram}"
            )))
        };
        let plain = markup(&render(diagram));
        assert_ne!(with_theme("dark"), plain);
        assert_ne!(with_theme("forest"), plain);
        assert_ne!(with_theme("dark"), with_theme("forest"));
        // JSON5, as mermaid accepts it.
        assert_eq!(
            markup(&render(&format!(
                "%%{{init: {{'theme': 'dark'}}}}%%\n{diagram}"
            ))),
            with_theme("dark")
        );
    }
}

#[cfg(not(feature = "mermaid"))]
#[test]
fn rendering_needs_the_feature() {
    let large = "x".repeat(60_000);
    for source in [SIMPLE, "", "flowchart LR\n  A[/]", large.as_str()] {
        assert!(matches!(
            mermaid::render_svg(source),
            Err(MermaidError::Disabled)
        ));
    }
}

fn ink(payload: &str) -> String {
    format!("https://mermaid.ink/img/{payload}")
}

#[test]
fn mermaid_ink_urls_decode() {
    // The plain form: standard, padded base64 of the raw source.
    assert_eq!(
        mermaid::decode_mermaid_ink("https://mermaid.ink/img/Z3JhcGggVEQKICAgIEEtLT5C").as_deref(),
        Some(SIMPLE)
    );
    let source = "graph LR\n  A[\"Größe >= 1?\"] --> B";
    let standard = STANDARD.encode(source);
    assert!(
        standard.contains(['+', '/']) && standard.ends_with('='),
        "{standard}"
    );
    for payload in [
        standard.clone(),
        STANDARD_NO_PAD.encode(source),
        URL_SAFE.encode(source),
        URL_SAFE_NO_PAD.encode(source),
        format!("base64:{standard}"),
        standard
            .replace('+', "%2B")
            .replace('/', "%2f")
            .replace('=', "%3D"),
    ] {
        for url in [
            ink(&payload),
            format!("https://mermaid.ink/svg/{payload}"),
            format!("{}?type=png&bgColor=!white", ink(&payload)),
            format!("{}#fragment", ink(&payload)),
            format!("HTTPS://Mermaid.INK/img/{payload}"),
            format!("  {}\n", ink(&payload)),
        ] {
            assert_eq!(
                mermaid::decode_mermaid_ink(&url).as_deref(),
                Some(source),
                "{url}"
            );
        }
    }
    // The mermaid live editor's state: JSON with the source in `code`.
    let state = serde_json::json!({"code": source, "mermaid": {"theme": "default"}}).to_string();
    for payload in [STANDARD.encode(&state), URL_SAFE_NO_PAD.encode(&state)] {
        assert_eq!(
            mermaid::decode_mermaid_ink(&ink(&payload)).as_deref(),
            Some(source)
        );
    }
    // JSON without a string `code` is taken as the source itself.
    for text in [
        r#"{"mermaid": {"theme": "dark"}}"#,
        r#"{"code": 42}"#,
        "[1, 2]",
        "\"quoted\"",
    ] {
        assert_eq!(
            mermaid::decode_mermaid_ink(&ink(&STANDARD.encode(text))).as_deref(),
            Some(text)
        );
    }
}

#[test]
fn mermaid_ink_urls_that_do_not_decode() {
    for url in [
        // zlib-compressed
        "https://mermaid.ink/img/pako:eNpLL0osyFAIcbHmUlBwVNC1s1NwAgAzKgR8".to_owned(),
        // other hosts, schemes and paths
        "https://example.com/img/Z3JhcGggVEQKICAgIEEtLT5C".to_owned(),
        "https://mermaid.ink.example.com/img/Z3JhcGggVEQKICAgIEEtLT5C".to_owned(),
        "https://mermaid.ink/pdf/Z3JhcGggVEQKICAgIEEtLT5C".to_owned(),
        "https://mermaid.ink/Z3JhcGggVEQKICAgIEEtLT5C".to_owned(),
        "workitemimg:1-mermaid-744d43fa10d8c687.svg".to_owned(),
        // payloads that are not base64 of a source
        ink(""),
        ink("not*base64"),
        ink("Z3JhcGggVEQKICAgIEEtLT5"),
        ink("%zz"),
        ink("%+F"),
        ink(&STANDARD.encode([0xff, 0xfe, 0xfd])),
        ink(&STANDARD.encode(" \n\t")),
        String::new(),
    ] {
        assert_eq!(mermaid::decode_mermaid_ink(&url), None, "{url}");
    }
}
