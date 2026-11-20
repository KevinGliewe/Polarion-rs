//! Rendering with merman (feature `mermaid`): our size limits before rendering, merman's own
//! limits and a deadline while it runs, the renderer on a thread of its own, a sanitizer for what
//! a diagram source could smuggle into the SVG, a fixed size for the image, and a check that the
//! result is well-formed XML.

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
    use std::time::Duration;

    use merman::svg::SvgPipeline;
    use merman::{
        OperationControl, RenderError, RenderOutput, RenderRequest, Renderer, SvgRequest,
    };

    use super::MermaidError;
    use crate::html::{dangerous_url, xml_allows};
    use crate::mermaid::svg::{self, Item, Markup, StartTag};

    /// Sources longer than this many bytes are not rendered.
    const MAX_BYTES: usize = 50_000;
    /// Sources with more lines than this are not rendered.
    const MAX_LINES: usize = 1_000;
    /// A render that takes longer is given up. The renderer has limits of its own (500 flowchart
    /// edges, 32,000 items in a diagram, 256 levels in a directive), which refuse what would take
    /// long in milliseconds; this is the backstop for what they miss. A typical diagram takes
    /// milliseconds, a flowchart at the edge limit less than a second (optimized build).
    const DEADLINE: Duration = Duration::from_secs(20);
    /// The stack of the thread the renderer runs on, reserved and committed as it is used. A
    /// stack overflow aborts the process, which `catch_unwind` cannot prevent, and the stack of
    /// the calling thread may be small (1 MiB for the main thread on Windows, less for a worker
    /// of an async runtime). 499 nested C4 boundaries render in 1 MiB; this is the reserve for
    /// nesting nobody measured.
    const STACK_SIZE: usize = 64 << 20;

    pub(in crate::mermaid) fn render_svg(source: &str) -> Result<String, MermaidError> {
        check_limits(source)?;
        let svg = render_on_own_thread(source)?;
        let svg = fixed_size(&sanitize(&xml_chars(svg)));
        if let Some(what) = css_load(&svg) {
            return Err(MermaidError::Invalid(format!(
                "the diagram's styles would load a resource ({what}); a style can only refer to \
                 the diagram itself"
            )));
        }
        let svg = svg::embed_source(&svg, source);
        // The renderer escapes what it writes; this is the check that it did, since browsers do
        // not display an SVG that is not well-formed at all.
        svg::check_well_formed(&svg).map_err(|why| {
            MermaidError::Invalid(format!(
                "the renderer wrote an SVG that is not well-formed XML ({why})"
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
        Ok(())
    }

    /// [`render_raw`] on a thread with a [`STACK_SIZE`] stack; a panic becomes
    /// [`MermaidError::Panicked`]. `render_raw` holds no state of ours, and the renderer keeps
    /// none between renders (text is measured from built-in font metrics, without a cache).
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

    /// The renderer's SVG, before [`xml_chars`], [`sanitize`] and [`fixed_size`].
    ///
    /// - `SvgRequest::default()` is merman's deterministic environment: text is measured from
    ///   built-in metrics (no system fonts are read), and neither the clock nor the time zone is
    ///   used (a gantt chart gets no "today" line), so a source always gives the same SVG.
    /// - `SvgPipeline::resvg_safe()` writes labels as SVG text. Mermaid's default, HTML in a
    ///   `foreignObject`, is not drawn where an SVG is shown as an image by something other
    ///   than a browser (Polarion's PDF and Word export), and is HTML we would have to vet.
    fn render_raw(source: &str) -> Result<String, MermaidError> {
        // The deadline runs from here, on the render thread.
        let control = OperationControl::new().with_deadline(DEADLINE);
        let request = SvgRequest {
            pipeline: Some(SvgPipeline::resvg_safe()),
            ..SvgRequest::default()
        };
        match Renderer::new().render(RenderRequest::svg(source, control, request)) {
            Ok(RenderOutput::Svg(Some(svg))) => Ok(svg.into_parts().0.trim_end().to_owned()),
            Ok(_) | Err(RenderError::NoDiagram) => Err(MermaidError::Invalid(
                "no mermaid diagram: the first line must name the kind (flowchart, \
                 sequenceDiagram, gantt, ...)"
                    .to_owned(),
            )),
            Err(RenderError::Cancelled(_)) => Err(MermaidError::TooLarge(format!(
                "rendering the diagram took longer than {} s",
                DEADLINE.as_secs()
            ))),
            Err(limit @ RenderError::ResourceLimitExceeded(_)) => {
                Err(MermaidError::TooLarge(limit.to_string()))
            }
            Err(other) => Err(classify(other.to_string())),
        }
    }

    /// The error for what the renderer refuses with a message: its limits on the size of a
    /// diagram, which it reports like syntax errors, are [`MermaidError::TooLarge`].
    fn classify(message: String) -> MermaidError {
        if message.contains("limit exceeded") || message.contains("exceeds") {
            MermaidError::TooLarge(message)
        } else {
            MermaidError::Invalid(message)
        }
    }

    fn panic_message(payload: &(dyn Any + Send)) -> String {
        payload
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "the renderer panicked".to_owned())
    }

    /// The SVG with a size of its own: the root's `width="100%"` (mermaid sizes a diagram to its
    /// container, up to the `max-width` in its `style`) becomes the width of the `viewBox`, and
    /// the height is added. An image without a size is drawn at the viewer's default (300 × 150
    /// in a browser) or as wide as the page; with one, it is drawn as large as it was laid out.
    /// Anything unexpected in the root tag leaves it as it is.
    fn fixed_size(svg: &str) -> String {
        let Some(Item::Start(root)) = Markup::new(svg).next() else {
            return svg.to_owned();
        };
        let size = root.attr(svg, "viewBox").and_then(|view_box| {
            let numbers: Vec<f64> = view_box
                .split([' ', ','])
                .filter(|part| !part.is_empty())
                .map(|part| part.parse().ok())
                .collect::<Option<_>>()?;
            match numbers[..] {
                [_, _, width, height] if width > 0.0 && height > 0.0 => Some((width, height)),
                _ => None,
            }
        });
        let width = root
            .attrs
            .iter()
            .find(|attr| attr.name == "width")
            .and_then(|attr| attr.value);
        let has_height = root.attrs.iter().any(|attr| attr.name == "height");
        let (Some((width, height)), Some((from, to))) = (size, width) else {
            return svg.to_owned();
        };
        if &svg[from..to] != "100%" {
            return svg.to_owned();
        }
        // After the value's closing quote. Some kinds (journey) come with a height of their own.
        let after = to + 1;
        let height = if has_height {
            String::new()
        } else {
            format!(" height=\"{height}\"")
        };
        format!(
            "{}{width}{}{height}{}",
            &svg[..from],
            &svg[to..after],
            &svg[after..]
        )
    }

    /// What in the styles of the SVG would make a viewer load something: `@import`, or a
    /// `url(…)` that is not a reference into the document (`url(#id)`). Looked for in `style`
    /// elements and `style` attributes, where the renderer writes values from the source (the
    /// theme variables and CSS of a directive, `classDef`, `style`). It keeps these out as far
    /// as known; this is the check that it did.
    fn css_load(svg: &str) -> Option<String> {
        let mut style_from = None;
        for item in Markup::new(svg) {
            match item {
                Item::Start(tag) => {
                    for attr in &tag.attrs {
                        if let Some((from, to)) = attr.value
                            && attr.name.eq_ignore_ascii_case("style")
                            && let Some(found) = css_load_in(&svg::unescape(&svg[from..to]))
                        {
                            return Some(found);
                        }
                    }
                    if tag.name == "style" && !tag.self_closing {
                        style_from = Some(tag.end);
                    }
                }
                Item::End {
                    start,
                    name: "style",
                    ..
                } => {
                    if let Some(from) = style_from.take()
                        && let Some(found) = css_load_in(&svg::unescape(&svg[from..start]))
                    {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// [`css_load`] for one piece of CSS: the `@import` or `url(` it finds, shortened.
    fn css_load_in(css: &str) -> Option<String> {
        // CSS escapes (`u\72l(`) and comments (`@im/**/port`) could hide either from the search
        // below; the renderer writes neither.
        if css.contains('\\') || css.contains("/*") {
            return Some("a CSS escape or comment".to_owned());
        }
        let lower = css.to_ascii_lowercase();
        if lower.contains("@import") {
            return Some("`@import`".to_owned());
        }
        for (at, _) in lower.match_indices("url(") {
            let target = lower[at + 4..].trim_start_matches([' ', '\t', '\n', '\r', '"', '\'']);
            if !target.starts_with('#') {
                let shown: String = css[at..].chars().take(40).collect();
                return Some(format!("`{shown}`"));
            }
        }
        None
    }

    /// The SVG with the characters XML does not allow (C0 controls other than tab and line
    /// breaks, U+FFFE, U+FFFF) replaced by U+FFFD, as [`svg::embed_source`] replaces them in the
    /// source. The renderer drops them from labels as far as known (a label pasted from an office
    /// document can hold a vertical tab, its soft line break); this makes sure of it.
    fn xml_chars(svg: String) -> String {
        if svg.chars().all(xml_allows) {
            return svg;
        }
        svg.chars()
            .map(|c| if xml_allows(c) { c } else { '\u{FFFD}' })
            .collect()
    }

    /// The elements merman writes with [`SvgPipeline::resvg_safe`] (as of 0.10.0, exact names);
    /// [`sanitize`] removes all others. None of these runs script or loads a document: no
    /// `script`, no `foreignObject` (HTML), no animation. A renderer upgrade that writes a new
    /// element needs it here; the test `sanitize_keeps_what_the_renderer_writes` notices.
    const ELEMENTS: &[&str] = &[
        "a",
        "circle",
        "clipPath",
        "defs",
        "ellipse",
        "feDropShadow",
        "filter",
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
        "use",
    ];

    /// Elements of [`ELEMENTS`] that hold only text: an HTML parser reads the content of an SVG
    /// `title` as HTML, where a `style` holds raw text, so markup inside them would mean something
    /// else to it than to an XML parser.
    const TEXT_ONLY: &[&str] = &["style", "title"];

    /// Attributes (local names, any case) holding a URL that is followed or loaded: `href` (also
    /// `xlink:href`), `xml:base`, and HTML's `src`, `action` and `formaction`.
    const URL_ATTRIBUTES: &[&str] = &["href", "base", "src", "action", "formaction"];

    /// The SVG with what a diagram source could smuggle into it removed, everything else kept
    /// byte for byte. merman escapes what it writes and drops scripts, event handlers and
    /// dangerous links itself, as far as known; this does not rely on it. So:
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
        fn sizes_and_style_loads() {
            assert_eq!(
                fixed_size(
                    "<svg id=\"m\" width=\"100%\" viewBox=\"4 4 168 152.5\" style=\"max-width:168px\"><g/></svg>"
                ),
                "<svg id=\"m\" width=\"168\" height=\"152.5\" viewBox=\"4 4 168 152.5\" style=\"max-width:168px\"><g/></svg>"
            );
            assert_eq!(
                fixed_size("<svg width=\"100%\" height=\"5\" viewBox=\"0 0 2 1\"/>"),
                "<svg width=\"2\" height=\"5\" viewBox=\"0 0 2 1\"/>"
            );
            // Left alone: a size of its own, no view box, one that is no size.
            for svg in [
                "<svg width=\"10\" viewBox=\"0 0 1 1\"/>",
                "<svg width=\"100%\"/>",
                "<svg width=\"100%\" viewBox=\"0 0 0 1\"/>",
                "<svg width=\"100%\" viewBox=\"a b c d\"/>",
                "no markup",
            ] {
                assert_eq!(fixed_size(svg), svg);
            }

            for clean in [
                "<svg><style>#m .a{fill:url(#grad);marker-end:url( '#arrow' )}</style><g style=\"filter:url(&quot;#shadow&quot;)\"/><text>see url(https://x) and @import</text></svg>",
                "<svg><style/></svg>",
            ] {
                assert_eq!(css_load(clean), None, "{clean}");
            }
            for (loading, what) in [
                (
                    "<svg><style>@import url(https://x/a.css);</style></svg>",
                    "`@import`",
                ),
                (
                    "<svg><style>.a{fill:URL(https://x/a.png)}</style></svg>",
                    "`URL(https://x/a.png)}`",
                ),
                (
                    "<svg><g style=\"background:url(&#34;//x/a.png&#34;)\"/></svg>",
                    "`url(\"//x/a.png\")`",
                ),
                (
                    "<svg><style>.a{fill:u\\72l(https://x)}</style></svg>",
                    "a CSS escape or comment",
                ),
                (
                    "<svg><style>@im/**/port 'x';</style></svg>",
                    "a CSS escape or comment",
                ),
            ] {
                assert_eq!(css_load(loading).as_deref(), Some(what), "{loading}");
            }
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
                assert_eq!(css_load(&svg), None, "{source:?}");
                assert_ne!(fixed_size(&svg), svg, "{source:?}");
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
            let words = ["a & b", "x < y", "it's", "Grüße", "#quot;", "a<br>b"];
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
