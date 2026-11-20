//! Mermaid diagrams, attached to work items as SVG files named after a hash of their source.
//!
//! A diagram's file name is `mermaid-<hash>.svg`, the hash covering the normalized source (and
//! [`PROFILE`]), so an unchanged diagram maps to the same attachment and is neither rendered nor
//! uploaded again. The SVG carries the source in its `<metadata>` ([`embed_source`]), which is how
//! the conversion back to Markdown restores the ```` ```mermaid ```` block ([`source_of`]).

mod ink;
mod render;
mod svg;

use std::collections::BTreeSet;
use std::fmt::Write as _;

use comrak::Arena;
use sha2::{Digest, Sha256};

pub use ink::decode_mermaid_ink;
pub use svg::{embed_source, source_of};

/// Prefix of every hashed source. Change it only when the default theme or the embedding format
/// changes: every diagram then gets a new file name and is rendered again. The renderer version is
/// deliberately not part of it (an upgrade does not re-upload every diagram). `2`: the renderer
/// changed from mermaid-rs-renderer to merman, and with it every diagram's look.
pub const PROFILE: &str = "polarion-markdown/mermaid/2";

/// XML namespace of the `<mermaid>` element holding the source inside the SVG's `<metadata>`.
pub const NAMESPACE: &str = "urn:polarion-rs:mermaid";

/// A mermaid diagram of a Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagram {
    /// The source, normalized ([`normalize`]).
    pub source: String,
    /// 16 lowercase hex digits ([`hash`]).
    pub hash: String,
    /// `mermaid-<hash>.svg`.
    pub file_name: String,
}

/// The diagram for a mermaid source.
pub fn diagram(source: &str) -> Diagram {
    let source = normalize(source);
    let hash = hash_normalized(&source);
    Diagram {
        file_name: file_name(&hash),
        hash,
        source,
    }
}

/// The ```` ```mermaid ```` blocks of a Markdown document that
/// [`to_html_with_diagrams`](crate::to_html_with_diagrams) writes as images when their attachments
/// are known: in document order, each diagram once. Empty blocks are skipped, and so are blocks
/// nested so deep that the conversion writes them as text.
pub fn diagrams(markdown: &str) -> Vec<Diagram> {
    let arena = Arena::new();
    let root = crate::parse::parse(&arena, markdown);
    let mut seen = BTreeSet::new();
    crate::emit::reached_diagrams(root)
        .into_iter()
        .filter(|d| !d.source.is_empty() && seen.insert(d.hash.clone()))
        .collect()
}

/// Whether a code block info string names mermaid (its first word, ignoring case).
pub(crate) fn is_mermaid_info(info: &str) -> bool {
    info.split_whitespace()
        .next()
        .is_some_and(|lang| lang.eq_ignore_ascii_case("mermaid"))
}

/// Line endings to `\n`, characters XML does not allow replaced (the form feed by a space, the
/// others by U+FFFD, as the HTML gets them), trailing spaces and tabs removed from every line,
/// leading and trailing empty lines dropped (no final newline). So the source embedded in the SVG
/// ([`embed_source`]) is the normalized one exactly, and hashes the same when it comes back.
pub fn normalize(source: &str) -> String {
    let text = crate::html::xml_safe(source)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect();
    match (
        lines.iter().position(|l| !l.is_empty()),
        lines.iter().rposition(|l| !l.is_empty()),
    ) {
        (Some(first), Some(last)) => lines[first..=last].join("\n"),
        _ => String::new(),
    }
}

/// The hash of a source (normalized first): the first 8 bytes of
/// SHA-256(`PROFILE` + `"\n"` + source) as 16 lowercase hex digits.
pub fn hash(source: &str) -> String {
    hash_normalized(&normalize(source))
}

fn hash_normalized(normalized: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(PROFILE.as_bytes());
    hasher.update(b"\n");
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    let bytes: &[u8] = &digest;
    let mut hex = String::with_capacity(16);
    for b in &bytes[..8] {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// `mermaid-<hash>.svg`.
pub fn file_name(hash: &str) -> String {
    format!("mermaid-{hash}.svg")
}

/// Whether an attachment file name is one of ours: `mermaid-` + 16 lowercase hex digits + `.svg`.
pub fn is_diagram_file(name: &str) -> bool {
    name.strip_prefix("mermaid-")
        .and_then(|rest| rest.strip_suffix(".svg"))
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// Why a diagram could not be rendered.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MermaidError {
    /// The source is not a diagram the renderer understands, or needs something this build
    /// leaves out (formulas).
    #[error("{0}")]
    Invalid(String),
    /// The source exceeds a size limit: ours, checked before rendering, or one of the
    /// renderer's, or rendering took longer than the deadline.
    #[error("{0}")]
    TooLarge(String),
    /// The renderer panicked (or no thread could be started to render on).
    #[error("the renderer failed: {0}")]
    Panicked(String),
    /// Built without the `mermaid` feature.
    #[error("rendering mermaid diagrams needs the `mermaid` feature")]
    Disabled,
}

/// Render a mermaid source to SVG, with the source embedded ([`embed_source`]) and links to
/// dangerous URLs (`javascript:` ...) neutralized.
///
/// - Needs the `mermaid` feature, else [`MermaidError::Disabled`].
/// - The renderer is [merman](https://crates.io/crates/merman), which follows mermaid.js in
///   syntax, layout and SVG structure. Formulas (`$$…$$`) are not built in and are refused
///   ([`MermaidError::Invalid`]).
/// - Refused as too large ([`MermaidError::TooLarge`]): more than 50,000 bytes or 1,000 lines,
///   checked before anything is parsed; what the renderer's own limits refuse (more than 500
///   edges in a flowchart, also from `&` lists; more than 32,000 items in a diagram; a
///   directive nested more than 256 deep); and a render that takes longer than 20 s.
/// - Labels are SVG text, not HTML in a `foreignObject`, which only a browser draws.
/// - The image has a size: mermaid sizes a diagram to its container (`width="100%"`), so the
///   root gets the width and height of its `viewBox`.
/// - The theme is the one a `%%{init: {"theme": "…"}}%%` directive names, else mermaid's
///   default theme.
/// - Nothing in the SVG runs or loads. The renderer drops script, event handlers and dangerous
///   links and escapes what it writes; this function does not rely on it. Links to
///   `javascript:`, `vbscript:`, `file:` and `data:` URLs (other than PNG, GIF, JPEG and WebP
///   images) point to `#`. Event handler attributes, comments and every element the renderer
///   does not write itself (`script`, `foreignObject`, HTML, animation, …) are removed.
///   Characters XML does not allow (C0 controls other than tab and line breaks, U+FFFE, U+FFFF)
///   become U+FFFD. A style that would load a resource (`@import`, a `url(…)` that is not a
///   reference into the diagram) fails with [`MermaidError::Invalid`].
/// - The result is well-formed XML, else [`MermaidError::Invalid`]: browsers do not display an
///   SVG that is not.
/// - The renderer runs on a thread of its own with a 64 MiB stack (reserved, committed as used):
///   a stack overflow would abort the process. Its panic becomes [`MermaidError::Panicked`];
///   the panic hook still prints it.
/// - A source always gives the same SVG: text is measured from built-in metrics (the fonts of
///   the machine are not read), and neither the clock nor the time zone is used, so a gantt
///   chart gets no "today" line.
/// - Rendering is CPU-bound and synchronous: a typical diagram takes milliseconds, a flowchart
///   at the edge limit less than a second (optimized build).
pub fn render_svg(source: &str) -> Result<String, MermaidError> {
    render::render_svg(source)
}
