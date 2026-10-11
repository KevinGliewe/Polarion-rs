//! Rendering with mermaid-rs-renderer (feature `mermaid`): limits checked before rendering, the
//! renderer on a thread of its own, a sanitizer for what a diagram source can smuggle into the
//! SVG, and a check that the result is well-formed XML.

use super::MermaidError;

#[cfg(not(feature = "mermaid"))]
pub(super) fn render_svg(source: &str) -> Result<String, MermaidError> {
    let _ = source;
    Err(MermaidError::Disabled)
}

#[cfg(feature = "mermaid")]
pub(super) use enabled::render_svg;

#[cfg(feature = "mermaid")]
mod enabled {
    use std::any::Any;
    use std::thread;

    use mermaid_rs_renderer::{
        DiagramKind, Graph, LayoutConfig, ParseError, Theme, compute_layout, parse_mermaid,
        validator,
    };

    use super::MermaidError;
    use crate::html::{dangerous_url, xml_allows};
    use crate::mermaid::svg::{self, Item, Markup, StartTag};

    /// Sources longer than this many bytes are not rendered.
    const MAX_BYTES: usize = 50_000;
    /// Sources with more lines than this are not rendered.
    const MAX_LINES: usize = 1_000;
    /// A `%%{…}%%` directive line with more `{` and `[` than this is not rendered: the renderer
    /// reads the directive with a JSON5 parser that recurses once per nesting level, without a
    /// limit (1.6 KB of stack a level in a debug build).
    const MAX_DIRECTIVE_BRACKETS: usize = 64;
    /// A line with a `$$` formula in which `{…}` nest deeper than this is not rendered: the
    /// renderer recurses once per group and copies the rest of the formula at every level (a
    /// formula of 7,000 nested groups takes a gigabyte).
    const MAX_FORMULA_DEPTH: usize = 32;
    /// Sources whose `&` lists (`A & B --> C & D`: an edge for every pair) would make the parser
    /// copy more bytes than this are not parsed: every edge gets a copy of the names and the label
    /// (a 46 KB source makes nine million edges in 2.3 GB).
    const MAX_LIST_BYTES: usize = 16 << 20;
    /// Sources whose subgraphs or composite states the parser would spend more steps than this
    /// (about a second) on are not parsed: each time the source names a node, it looks the node
    /// up among the members of every open group and of all the others, so the work grows with
    /// node references × nesting depth × (groups + node references). 450 nested subgraphs around
    /// a chain of 5,000 nodes (49 KB) take 143 s.
    const MAX_GROUP_WORK: u64 = 200_000_000;
    /// Diagrams whose edges the renderer routes are not laid out with more edges than this:
    /// routing time grows with the square of the edge count (300 edges take up to about 15 s in
    /// an optimized build, 800 more than a minute).
    const MAX_EDGES: usize = 300;
    /// The stack of the thread the renderer runs on, reserved and committed as it is used. Parts
    /// of the renderer recurse once per nesting level of the source (C4 boundaries take 4 KB of
    /// stack a level), and a stack overflow aborts the process, which `catch_unwind` cannot
    /// prevent: the stack of the calling thread (1 MiB for the main thread on Windows) is not
    /// enough for every source within the limits.
    const STACK_SIZE: usize = 64 << 20;

    pub(in crate::mermaid) fn render_svg(source: &str) -> Result<String, MermaidError> {
        check_limits(source)?;
        let svg = render_on_own_thread(source)?;
        let svg = svg::embed_source(&sanitize(&xml_chars(svg)), source);
        // What the sanitizer keeps of markup injected through a style value can still break the
        // XML, and browsers do not display such an SVG at all.
        svg::check_well_formed(&svg).map_err(|why| {
            MermaidError::Invalid(format!(
                "the renderer wrote an SVG that is not well-formed XML ({why}); it writes \
                 `style`, `classDef` and `linkStyle` values as they are"
            ))
        })?;
        Ok(svg)
    }

    fn check_limits(source: &str) -> Result<(), MermaidError> {
        if source.len() > MAX_BYTES {
            return Err(MermaidError::TooLarge(format!(
                "the diagram has {} bytes, more than the limit of {MAX_BYTES}",
                source.len()
            )));
        }
        let lines = source.lines().count();
        if lines > MAX_LINES {
            return Err(MermaidError::TooLarge(format!(
                "the diagram has {lines} lines, more than the limit of {MAX_LINES}"
            )));
        }
        let mut list_bytes = 0usize;
        // The renderer splits lines the same way (`str::lines`), and its parsers read a directive
        // or a label from a single line.
        for (index, line) in source.lines().enumerate() {
            let number = index + 1;
            if line.trim_start().starts_with("%%{") {
                let brackets = line.bytes().filter(|b| matches!(b, b'{' | b'[')).count();
                if brackets > MAX_DIRECTIVE_BRACKETS {
                    return Err(MermaidError::TooLarge(format!(
                        "line {number}: the directive has {brackets} `{{` and `[`, more than the \
                         limit of {MAX_DIRECTIVE_BRACKETS}"
                    )));
                }
            }
            if line.contains("$$") {
                let depth = brace_depth(line);
                if depth > MAX_FORMULA_DEPTH {
                    return Err(MermaidError::TooLarge(format!(
                        "line {number}: `{{…}}` nest {depth} deep in a line with a `$$` formula, \
                         more than the limit of {MAX_FORMULA_DEPTH}"
                    )));
                }
            }
            list_bytes = list_bytes.saturating_add(list_copies(line));
        }
        if list_bytes > MAX_LIST_BYTES {
            return Err(MermaidError::TooLarge(format!(
                "the diagram's `&` lists would make the parser copy {} MiB, more than the limit \
                 of {} MiB",
                list_bytes >> 20,
                MAX_LIST_BYTES >> 20
            )));
        }
        let work = group_work(source);
        if work > MAX_GROUP_WORK {
            return Err(MermaidError::TooLarge(format!(
                "the diagram has too many nodes in nested groups (subgraphs, composite states): \
                 the parser would take {} million steps, more than the limit of {} million",
                work / 1_000_000,
                MAX_GROUP_WORK / 1_000_000
            )));
        }
        Ok(())
    }

    /// At least the steps of the parser's group bookkeeping ([`MAX_GROUP_WORK`]): node references
    /// × nesting depth × (groups + node references), for flowchart subgraphs (a statement
    /// `subgraph …` opens one) and, in a state diagram, for composite states (any `{` may open
    /// one). Every possible opener counts, also in front matter; only a line that is just `end`
    /// (or `}`) outside front matter, which the parser surely reads as one, closes a group.
    fn group_work(source: &str) -> u64 {
        let states = source.to_ascii_lowercase().contains("statediagram");
        let mut refs = 0u64;
        // For subgraphs and for composite states: depth, deepest depth, groups.
        let mut subgraphs = (0u64, 0u64, 0u64);
        let mut composites = (0u64, 0u64, 0u64);
        let open = |(depth, deepest, groups): &mut (u64, u64, u64), count: u64| {
            *depth += count;
            *deepest = (*deepest).max(*depth);
            *groups += count;
        };
        let mut front_matter = false;
        for line in source.lines() {
            let line = line.trim();
            refs += node_refs(line);
            if line == "---" {
                front_matter = !front_matter;
            } else if !front_matter && line == "end" {
                subgraphs.0 = subgraphs.0.saturating_sub(1);
            } else if !front_matter && line == "}" {
                composites.0 = composites.0.saturating_sub(1);
            }
            // The parser splits a line into statements at `;` (outside quotes and brackets).
            let openers = line
                .split(';')
                .filter(|statement| statement.trim_start().starts_with("subgraph"))
                .count();
            open(&mut subgraphs, openers as u64);
            if states {
                open(&mut composites, line.matches('{').count() as u64);
            }
        }
        [subgraphs, composites]
            .iter()
            .map(|&(_, deepest, groups)| {
                refs.saturating_mul(deepest)
                    .saturating_mul(groups.saturating_add(refs))
            })
            .max()
            .unwrap_or(0)
    }

    /// At least the node references of a line: per statement (separated by `;`, or by braces in
    /// a state diagram) at most two per arrow (a run of `-`, `=` and `.` with a `-` or `=`), two
    /// per `&` and two more.
    fn node_refs(line: &str) -> u64 {
        let mut arrows = 0u64;
        let mut run = None;
        for b in line.bytes().chain([b' ']) {
            match (b, run) {
                (b'-' | b'=', _) => run = Some(true),
                (b'.', None) => run = Some(false),
                (b'.', Some(_)) => {}
                (_, Some(arrow)) => {
                    arrows += u64::from(arrow);
                    run = None;
                }
                (_, None) => {}
            }
        }
        let separators = line
            .bytes()
            .filter(|b| matches!(b, b'&' | b';' | b'{' | b'}'))
            .count() as u64;
        2 * (arrows + separators + 1)
    }

    /// How deep `{…}` nest in a line at most, counted as the renderer matches the braces of a
    /// formula: every `{` opens a group, every `}` closes one if one is open. A formula is part
    /// of a line, so its groups nest at most this deep.
    fn brace_depth(line: &str) -> usize {
        let mut depth = 0usize;
        let mut deepest = 0;
        for b in line.bytes() {
            match b {
                b'{' => {
                    depth += 1;
                    deepest = deepest.max(depth);
                }
                b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        deepest
    }

    /// At least as many bytes as the parser copies for the edges a line's `&` lists add: a line
    /// with n `&` adds at most n²/4 + 2n edges to the one per arrow, and each holds copies of the
    /// names and the label (together shorter than the line) besides about 200 bytes of its own.
    fn list_copies(line: &str) -> usize {
        let n = line.bytes().filter(|&b| b == b'&').count();
        (n * n / 4 + 2 * n).saturating_mul(line.len() + 256)
    }

    /// [`render_raw`] on a thread with a [`STACK_SIZE`] stack; a panic becomes
    /// [`MermaidError::Panicked`]. Only the renderer's own state can suffer from one (`render_raw`
    /// holds none of ours): a panic while it measures text poisons its global text measurer, and
    /// later renders in the process fall back to estimated text widths (the known panics happen
    /// while parsing).
    fn render_on_own_thread(source: &str) -> Result<String, MermaidError> {
        thread::scope(|scope| {
            thread::Builder::new()
                .name("mermaid-render".to_owned())
                .stack_size(STACK_SIZE)
                .spawn_scoped(scope, || render_raw(source))
                .map_err(|err| {
                    MermaidError::Panicked(format!(
                        "no thread could be started to render on: {err}"
                    ))
                })?
                .join()
                .unwrap_or_else(|payload| Err(MermaidError::Panicked(panic_message(&*payload))))
        })
    }

    /// The renderer's SVG, before [`xml_chars`] and [`sanitize`].
    fn render_raw(source: &str) -> Result<String, MermaidError> {
        // Not `render_with_options`: it parses strictly, and the strict validator rejects valid
        // diagrams (participants a sequence diagram creates implicitly, `A->>+B`).
        let parsed = parse_mermaid(source).map_err(|err| invalid(source, err.to_string()))?;
        check_edges(&parsed.graph)?;
        let theme = parsed
            .init_config
            .as_ref()
            .and_then(|init| init.get("theme"))
            .and_then(|theme| theme.as_str())
            .and_then(Theme::from_name)
            .unwrap_or_else(Theme::mermaid_default);
        let config = LayoutConfig::default();
        let layout = compute_layout(&parsed.graph, &theme, &config);
        Ok(mermaid_rs_renderer::render_svg(&layout, &theme, &config))
    }

    /// That a diagram whose edges the renderer routes has at most [`MAX_EDGES`]: flowcharts and
    /// class, state, ER and requirement diagrams. Packet diagrams are laid out the same way, but
    /// their edges are the fields, one per line, so they stay at about 3 s within the line limit;
    /// the other kinds have layouts of their own, and their edges (gantt dependencies, journey
    /// steps, messages) cost about linear time.
    fn check_edges(graph: &Graph) -> Result<(), MermaidError> {
        let routed = matches!(
            graph.kind,
            DiagramKind::Flowchart
                | DiagramKind::Class
                | DiagramKind::State
                | DiagramKind::Er
                | DiagramKind::Requirement
        );
        if routed && graph.edges.len() > MAX_EDGES {
            return Err(MermaidError::TooLarge(format!(
                "the diagram has {} edges, more than the limit of {MAX_EDGES}",
                graph.edges.len()
            )));
        }
        Ok(())
    }

    /// The error for a source the (lenient) parser rejects: the parser's message, or for an `init`
    /// directive the strict validator's, which names the line and the JSON error. The validator's
    /// other checks reject valid diagrams (participants a sequence diagram creates implicitly,
    /// `%%{…}%%` without a `:`), so their message could blame a line that is fine.
    fn invalid(source: &str, lenient: String) -> MermaidError {
        if lenient.starts_with("invalid Mermaid init directive")
            && let Err(strict) = validator::validate(source)
            && let ParseError::InvalidDirective { directive, .. } = &strict
            && directive == "init"
        {
            return MermaidError::Invalid(strict.to_string());
        }
        MermaidError::Invalid(lenient)
    }

    fn panic_message(payload: &(dyn Any + Send)) -> String {
        payload
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic without a message".to_owned())
    }

    /// The SVG with the characters XML does not allow (C0 controls other than tab and line
    /// breaks, U+FFFE, U+FFFF) replaced by U+FFFD, as [`svg::embed_source`] replaces them in the
    /// source: the renderer writes text as the source has it (a label pasted from an office
    /// document can hold a vertical tab, its soft line break).
    fn xml_chars(svg: String) -> String {
        if svg.chars().all(xml_allows) {
            return svg;
        }
        svg.chars()
            .map(|c| if xml_allows(c) { c } else { '\u{FFFD}' })
            .collect()
    }

    /// The elements mermaid-rs-renderer writes (as of 0.3.1, exact names); [`sanitize`] removes
    /// all others. None of these runs script or loads a document. A renderer upgrade that writes a
    /// new element needs it here; the test `sanitize_keeps_what_the_renderer_writes` notices.
    const ELEMENTS: &[&str] = &[
        "a",
        "circle",
        "defs",
        "ellipse",
        "g",
        "image",
        "line",
        "linearGradient",
        "marker",
        "path",
        "polygon",
        "polyline",
        "rect",
        "stop",
        "style",
        "svg",
        "symbol",
        "text",
        "title",
        "tspan",
    ];

    /// Elements of [`ELEMENTS`] that hold only text: an HTML parser reads the content of an SVG
    /// `title` as HTML, where a `style` holds raw text, so markup inside them would mean something
    /// else to it than to an XML parser.
    const TEXT_ONLY: &[&str] = &["style", "title"];

    /// Attributes (local names, any case) holding a URL that is followed or loaded: `href` (also
    /// `xlink:href`), `xml:base`, and HTML's `src`, `action` and `formaction`.
    const URL_ATTRIBUTES: &[&str] = &["href", "base", "src", "action", "formaction"];

    /// The SVG with what a diagram source can smuggle into it removed, everything else kept byte
    /// for byte. mermaid-rs-renderer escapes text and link URLs, but writes `style`, `classDef` and
    /// `linkStyle` values into attributes unescaped: a `"` in one ends the attribute and starts
    /// markup of the author's choosing. So:
    /// - a link to a dangerous URL ([`dangerous_url`]; `click` and `link` directives) points to
    ///   `#`;
    /// - event handler attributes (`on…`) are removed;
    /// - elements the renderer does not write ([`ELEMENTS`]: no `script`, `foreignObject`,
    ///   animation or HTML) are removed with their content, as are elements inside `title` and
    ///   `style` ([`TEXT_ONLY`]) and stray end tags;
    /// - comments, CDATA sections, processing instructions and declarations are removed: the
    ///   renderer writes none, and an HTML parser ends some of them earlier than an XML parser.
    fn sanitize(svg: &str) -> String {
        let mut out = String::with_capacity(svg.len());
        let mut copied = 0;
        // Inside a removed element: its name and how many elements of that name are open.
        let mut removing: Option<(&str, usize)> = None;
        // Inside a `title` or `style` element: its name.
        let mut text_only: Option<&str> = None;
        for item in Markup::new(svg) {
            if let Some((name, depth)) = &mut removing {
                match item {
                    Item::Start(tag)
                        if !tag.self_closing && tag.name.eq_ignore_ascii_case(name) =>
                    {
                        *depth += 1;
                    }
                    Item::End {
                        end, name: closed, ..
                    } if closed.eq_ignore_ascii_case(name) => {
                        *depth -= 1;
                        if *depth == 0 {
                            removing = None;
                            copied = end;
                        }
                    }
                    _ => {}
                }
                continue;
            }
            match item {
                Item::Text => {}
                Item::Special { start, end } => {
                    out.push_str(&svg[copied..start]);
                    copied = end;
                }
                Item::End { start, end, name } => {
                    if text_only.is_some_and(|open| open == name) {
                        text_only = None;
                    } else if text_only.is_some() || !ELEMENTS.contains(&name) {
                        out.push_str(&svg[copied..start]);
                        copied = end;
                    }
                }
                Item::Start(tag) => {
                    if text_only.is_some() || !ELEMENTS.contains(&tag.name) {
                        out.push_str(&svg[copied..tag.start]);
                        copied = tag.end;
                        if tag.closed && !tag.self_closing {
                            removing = Some((tag.name, 1));
                        }
                        continue;
                    }
                    if let Some(clean) = clean_tag(svg, &tag) {
                        out.push_str(&svg[copied..tag.start]);
                        out.push_str(&clean);
                        copied = tag.end;
                    }
                    if !tag.self_closing && TEXT_ONLY.contains(&tag.name) {
                        text_only = Some(tag.name);
                    }
                }
            }
        }
        // An unterminated removed element extends to the end.
        if removing.is_none() {
            out.push_str(&svg[copied..]);
        }
        out
    }

    /// The tag without event handler attributes and with dangerous URLs replaced by `#`; `None`
    /// when it needs neither.
    fn clean_tag(svg: &str, tag: &StartTag<'_>) -> Option<String> {
        let mut clean = String::new();
        let mut at = tag.start;
        for attr in &tag.attrs {
            let name = local_name(attr.name);
            if name
                .get(..2)
                .is_some_and(|on| on.eq_ignore_ascii_case("on"))
            {
                clean.push_str(&svg[at..attr.start]);
                at = attr.end;
            } else if let Some((from, to)) = attr.value
                && URL_ATTRIBUTES
                    .iter()
                    .any(|url| name.eq_ignore_ascii_case(url))
                && dangerous_value(&svg[from..to])
            {
                clean.push_str(&svg[at..from]);
                clean.push('#');
                at = to;
            }
        }
        if at == tag.start {
            return None;
        }
        clean.push_str(&svg[at..tag.end]);
        Some(clean)
    }

    /// `xlink:href` → `href`.
    fn local_name(name: &str) -> &str {
        name.rsplit(':').next().unwrap_or(name)
    }

    /// Whether an attribute value (still escaped) is a dangerous URL, also to an HTML parser: a
    /// value with a reference XML does not define (`&colon;`, `&#106` without its `;`, a bare `&`)
    /// counts as dangerous, since the renderer escapes every `&` it writes.
    fn dangerous_value(raw: &str) -> bool {
        raw.match_indices('&')
            .any(|(at, _)| svg::reference(&raw[at..]).is_none())
            || dangerous_url(&svg::unescape(raw))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn dangerous_values() {
            assert!(dangerous_value("&#106;avascript:x"));
            assert!(dangerous_value("&#x6A;avascript:x"));
            assert!(dangerous_value("java&#9;script:x"));
            assert!(dangerous_value("javascript&#58;x"));
            assert!(dangerous_value("javascript&colon;x"));
            assert!(dangerous_value("&#106avascript:x"));
            assert!(dangerous_value("a & b"));
            assert!(!dangerous_value(
                "https://example.com/?a=1&amp;b=2&#38;c=&lt;3&gt;"
            ));
            assert!(!dangerous_value("javascript&amp;#58;x"));
        }

        #[test]
        fn sanitize_keeps_clean_markup() {
            let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\"><a href=\"https://x/?a&amp;b\" xlink:href=\"#n\" target=\"_blank\"><title>tip &lt;b&gt;</title><rect fill=\"red\" orient=\"auto\"/></a><image xlink:href=\"data:image/png;base64,AA==\"/><style>.a>.b{fill:red}</style><text>on=\"x\" &lt;script&gt;</text></svg>";
            assert_eq!(sanitize(svg), svg);
        }

        /// Every element and attribute of these renders is the renderer's own: the sanitizer
        /// changes nothing, and the result is well-formed XML.
        #[test]
        fn sanitize_keeps_what_the_renderer_writes() {
            for source in [
                "flowchart TD\n  A[Start] --> B{Is it?}\n  B -->|Yes| C[OK]\n  C --> D[Rethink]\n  D --> B\n  B ---->|No| E[End]",
                "flowchart LR\n  subgraph one\n    a1-->a2\n  end\n  one --> b1\n  click a1 \"https://example.com/?a=1&b=2\" \"tip <x>\" _blank\n  click b1 href \"#anchor\"",
                "flowchart LR\n  A[rect] --> B(round) --> C([stadium]) --> D[[sub]] --> E[(db)] --> F((circle)) --> G>flag] --> H{rhombus} --> I{{hex}} --> J[/para/] --> K[\\alt\\]",
                "flowchart LR\n  A-->B\n  style A fill:#f9f,stroke:#333,stroke-width:4px\n  classDef c fill:#bbf\n  class B c\n  linkStyle 0 stroke:#ff3",
                "sequenceDiagram\n  participant Alice\n  Alice->>+Bob: Hello\n  Bob-->>-Alice: Hi\n  Note right of Bob: thinking\n  loop Every minute\n    Bob->>Alice: ping\n  end",
                "classDiagram\n  Animal <|-- Duck\n  Animal : +int age\n  Duck : +swim()",
                "stateDiagram-v2\n  [*] --> Still\n  Still --> Moving\n  Moving --> [*]",
                "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  ORDER ||--|{ LINE-ITEM : contains",
                "pie title Pets\n  \"Dogs\" : 386\n  \"Cats\" : 85",
                "gantt\n  title A Gantt\n  dateFormat YYYY-MM-DD\n  section S\n  Task :a1, 2024-01-01, 30d\n  Other :after a1, 20d",
                "journey\n  title My day\n  section Go to work\n    Make tea: 5: Me",
                "timeline\n  title History\n  2002 : LinkedIn\n  2004 : Facebook : Google",
                "mindmap\n  root((mindmap))\n    Origins\n      Long history\n    Research",
                "gitGraph\n  commit\n  branch develop\n  checkout develop\n  commit\n  checkout main\n  merge develop",
                "quadrantChart\n  title Reach\n  x-axis Low --> High\n  y-axis Low --> High\n  Campaign A: [0.3, 0.6]",
                "xychart-beta\n  title Sales\n  x-axis [jan, feb, mar]\n  y-axis Revenue 0 --> 100\n  bar [10, 50, 30]\n  line [10, 50, 30]",
                "sankey-beta\nA,B,10\nA,C,5",
                "C4Context\n  title System Context\n  Person(customer, \"Customer\", \"A customer\")\n  System(sys, \"System\", \"The system\")\n  Rel(customer, sys, \"Uses\")",
                "requirementDiagram\n  requirement r {\n    id: 1\n    text: the text.\n    risk: high\n    verifymethod: test\n  }\n  element e {\n    type: simulation\n  }\n  e - satisfies -> r",
                "block-beta\n  columns 3\n  a b c",
                "packet-beta\n  0-15: \"Source Port\"\n  16-31: \"Destination Port\"",
                "kanban\n  Todo\n    [Create Documentation]\n  Done\n    [Write tests]",
                "architecture-beta\n  group api(cloud)[API]\n  service db(database)[Database] in api\n  service server(server)[Server] in api\n  db:L -- R:server",
                "radar-beta\n  axis A, B, C\n  curve c1{1, 2, 3}",
                "treemap-beta\n\"Root\"\n    \"A\": 10\n    \"B\": 20",
                "zenuml\n  Alice->Bob: hi",
                "graph TD\n  A[\"x < y & z\"] --> B[\"line<br>break\"]\n  B --> C[Grüße 日本語 🎉]",
            ] {
                let svg = render_raw(source).unwrap_or_else(|e| panic!("{source:?}: {e}"));
                assert_eq!(sanitize(&svg), svg, "{source:?}");
                assert_eq!(xml_chars(svg.clone()), svg, "{source:?}");
                let embedded = svg::embed_source(&svg, source);
                assert_eq!(svg::check_well_formed(&embedded), Ok(()), "{source:?}");
            }
        }

        /// Random flowcharts of ids, shapes, labels with markup characters and edge kinds come
        /// out well-formed: the check does not refuse what the renderer writes on its own.
        #[test]
        fn random_flowcharts_are_well_formed() {
            let shapes = [
                ("[", "]"),
                ("(", ")"),
                ("{", "}"),
                ("([", "])"),
                ("[[", "]]"),
                ("((", "))"),
                ("{{", "}}"),
                ("[(", ")]"),
            ];
            let words = [
                "a & b",
                "x < y",
                "\\\"q\\\"",
                "it's",
                "Grüße",
                "#quot;",
                "a<br>b",
            ];
            let arrows = ["-->", "---", "-.->", "==>", "--o", "--x", "<-->"];
            let mut state = 0x9E37_79B9_7F4A_7C15_u64;
            let mut next = |bound: usize| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state % bound as u64) as usize
            };
            for _ in 0..12 {
                let mut source = String::from(["flowchart TD", "flowchart LR"][next(2)]);
                for _ in 0..1 + next(8) {
                    let (open, close) = shapes[next(shapes.len())];
                    let word = words[next(words.len())];
                    let arrow = arrows[next(arrows.len())];
                    let from = next(6);
                    let to = next(6);
                    source.push_str(&format!("\n  n{from}{open}\"{word}\"{close} {arrow} n{to}"));
                }
                let svg = render_raw(&source).unwrap_or_else(|e| panic!("{source:?}: {e}"));
                let embedded = svg::embed_source(&sanitize(&xml_chars(svg)), &source);
                assert_eq!(svg::check_well_formed(&embedded), Ok(()), "{source:?}");
            }
        }

        #[test]
        fn brace_depths() {
            assert_eq!(brace_depth(""), 0);
            assert_eq!(brace_depth("A{x} --> B{{y}}"), 2);
            assert_eq!(brace_depth("$$\\frac{\\sqrt{x}}{y}$$"), 2);
            // A `}` closes only an open group, so leading ones cannot hide the nesting after them.
            assert_eq!(brace_depth("}}}}{{{"), 3);
            assert_eq!(brace_depth("{a}{b}{c}"), 1);
        }

        /// Edges of kinds the renderer does not route are not limited, however many there are.
        #[test]
        fn only_routed_edges_are_limited() {
            let mut tasks = String::from(
                "gantt\n  dateFormat YYYY-MM-DD\n  section S\n  t0 :a0, 2024-01-01, 1d\n",
            );
            let mut messages = String::from("sequenceDiagram\n");
            let mut flows = String::from("sankey-beta\n");
            for i in 1..=400 {
                tasks.push_str(&format!("  t{i} :a{i}, after a{}, 1d\n", i - 1));
                messages.push_str(&format!("  A{}->>A{}: m{i}\n", i % 7, i % 5));
                flows.push_str(&format!("A{},B{},{i}\n", i % 30, i % 40));
            }
            for source in [tasks, messages, flows] {
                let graph = parse_mermaid(&source).unwrap().graph;
                assert!(graph.edges.len() > MAX_EDGES, "{:?}", graph.kind);
                assert!(check_edges(&graph).is_ok(), "{:?}", graph.kind);
            }
            for header in ["flowchart TD", "classDiagram", "stateDiagram-v2"] {
                let mut source = format!("{header}\n");
                for i in 0..=MAX_EDGES {
                    source.push_str(&format!("  N{i} --> N{}\n", i + 1));
                }
                let graph = parse_mermaid(&source).unwrap().graph;
                assert_eq!(graph.edges.len(), MAX_EDGES + 1, "{header}");
                assert!(check_edges(&graph).is_err(), "{header}");
            }
        }

        /// The bound of [`list_copies`] holds for what the renderer's parser makes of `&` lists:
        /// at most n²/4 + 2n edges beyond one per arrow for n `&` in a line.
        #[test]
        fn list_edges_are_bounded() {
            let mut state = 0x2545_F491_4F6C_DD1D_u64;
            let mut next = |bound: usize| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state % bound as u64) as usize
            };
            let mut lines = vec![
                "a & b --> c & d".to_owned(),
                "a[\"x & y\"] & b --> c".to_owned(),
                "a & b & c --> d --> e & f & g & h".to_owned(),
            ];
            for _ in 0..40 {
                let groups: Vec<String> = (0..1 + next(4))
                    .map(|g| {
                        (0..1 + next(7))
                            .map(|i| format!("n{g}x{i}"))
                            .collect::<Vec<_>>()
                            .join(" & ")
                    })
                    .collect();
                lines.push(groups.join(" --> "));
            }
            for line in lines {
                let parsed = parse_mermaid(&format!("flowchart TD\n{line}")).unwrap();
                let n = line.matches('&').count();
                let arrows = line.matches("-->").count();
                assert!(
                    parsed.graph.edges.len() <= n * n / 4 + 2 * n + arrows,
                    "{line}: {} edges",
                    parsed.graph.edges.len()
                );
                assert!(list_copies(&line) >= (n * n / 4 + 2 * n) * line.len());
            }
        }

        #[test]
        fn sanitize_neutralizes_links() {
            assert_eq!(
                sanitize(
                    "<a href=\"javascript:alert(1)\" xlink:href='JAVASCRIPT:x' x:href=java&#9;script:y>t</a>"
                ),
                "<a href=\"#\" xlink:href='#' x:href=#>t</a>"
            );
            assert_eq!(
                sanitize("<g xml:base=\"javascript:x//\"><a href=\"y\">t</a></g>"),
                "<g xml:base=\"#\"><a href=\"y\">t</a></g>"
            );
        }

        #[test]
        fn sanitize_removes_event_handlers() {
            assert_eq!(
                sanitize(
                    "<rect fill=\"red\" onmouseover=\"alert(1)\" stroke=\"b\" ONload='x' svg:onclick=\"y\"/>"
                ),
                "<rect fill=\"red\" stroke=\"b\"/>"
            );
            // `/` separates attributes; an unquoted value runs to whitespace or `>`.
            assert_eq!(sanitize("<rect/onclick=\"x\"/><g onload=y>"), "<rect/><g>");
        }

        #[test]
        fn sanitize_removes_elements_the_renderer_does_not_write() {
            assert_eq!(
                sanitize(
                    "<g><script>alert(1)<script>x</script></script><svg:Script/>a<foreignObject><div onclick=\"x\">h</div></foreignObject>b<set attributeName=\"href\" to=\"javascript:x\"/>c<svg:a href=\"x\">d</svg:a>e<Rect/></g>"
                ),
                "<g>abce</g>"
            );
            assert_eq!(sanitize("<g><script>never closed<rect/>"), "<g>");
            // HTML: a `p` would end the SVG for an HTML parser; `_x` is no tag to one, so the
            // `img` in its attribute would be markup.
            assert_eq!(
                sanitize(
                    "<g><p/><_x a=\"<img src=x onerror=alert(1)>\"/><b>bold<i>i</i></b></p></br><P>x</P><rect/></g>"
                ),
                "<g><rect/></g>"
            );
        }

        #[test]
        fn sanitize_keeps_title_and_style_text_only() {
            // An HTML parser reads a `title`'s content as HTML (and a `style`'s as raw text there),
            // which would end the `style` inside the attribute and make the `img` markup.
            assert_eq!(
                sanitize(
                    "<a><title>tip<style><a title=\"</style><img src=x onerror=alert(1)>\"></a></style></rect></title><rect/></a>"
                ),
                "<a><title>tip</title><rect/></a>"
            );
            assert_eq!(
                sanitize(
                    "<style>.a{}<title><b>x</b></title>.b{}</style><title><title>t</title><rect/></title><g/>"
                ),
                "<style>.a{}.b{}</style><title></title><g/>"
            );
        }

        #[test]
        fn sanitize_removes_comments_and_declarations() {
            assert_eq!(
                sanitize(
                    "<?xml version=\"1.0\"?><!DOCTYPE svg><svg><!--><rect onclick=x>--><![CDATA[><img onerror=x>]]><?pi x?>t</svg>"
                ),
                "<svg>t</svg>"
            );
        }
    }
}
