//! Markdown ↔ Polarion rich text (the HTML of work item descriptions and other rich text fields).
//!
//! [`to_html`] writes the inline-styled HTML Polarion's rich text editor produces: headings as
//! bold sized spans, tables with Polarion's table styles, `PREFIX-123` ids as work item links,
//! `$…$` as formulas, highlighted code, mermaid diagrams as images of SVG attachments.
//! [`to_markdown`] reads such HTML back as canonical Markdown. Formatting Markdown cannot express
//! stays HTML ([`RawHtml::Keep`], the default), so converting back restores it, or is left out
//! ([`RawHtml::Drop`]). A self-check converts each block back and writes it conservatively, or as
//! raw HTML, where it would change.
//!
//! Features: `mermaid` (default) renders diagrams to SVG ([`mermaid::render_svg`]); `highlight`
//! (default) highlights fenced code that names its language.
//!
//! Lossy: footnotes, definition lists, loose lists of single paragraphs, `:--` alignment,
//! directives, HTML comments and other spellings of a construct come back in canonical form (see
//! `docs/markdown.md`).

mod emit;
mod highlight;
mod html;
mod ids;
pub mod mermaid;
mod options;
mod parse;
mod refs;
mod reverse;
mod styles;

use std::collections::BTreeMap;

pub use options::{RawHtml, ToHtmlOptions, ToMarkdownOptions, WorkItemIds};
pub use refs::{attachment_file_name, attachment_refs, diagram_refs};
#[doc(hidden)]
pub use reverse::Report;

/// Attachment id by diagram file name (`mermaid-<hash>.svg`, see [`mermaid::Diagram`]).
pub type DiagramAttachments = BTreeMap<String, String>;

/// Mermaid source by attachment id (see [`mermaid::source_of`]).
pub type DiagramSources = BTreeMap<String, String>;

/// Markdown to Polarion HTML. Mermaid blocks stay code blocks (see [`to_html_with_diagrams`]).
pub fn to_html(markdown: &str, options: &ToHtmlOptions) -> String {
    to_html_with_diagrams(markdown, options, &DiagramAttachments::new())
}

/// Markdown to Polarion HTML; a mermaid block whose file name is in `diagrams` becomes an image
/// of that attachment (`<img src="workitemimg:ID">`), the others stay code blocks.
pub fn to_html_with_diagrams(
    markdown: &str,
    options: &ToHtmlOptions,
    diagrams: &DiagramAttachments,
) -> String {
    emit::to_html(markdown, options, diagrams)
}

/// Polarion HTML to Markdown. Diagram images stay images (see [`to_markdown_with_diagrams`]).
pub fn to_markdown(html: &str, options: &ToMarkdownOptions) -> String {
    to_markdown_with_diagrams(html, options, &DiagramSources::new())
}

/// Polarion HTML to Markdown; an image of an attachment listed in `sources` becomes a
/// ```` ```mermaid ```` block with that source.
pub fn to_markdown_with_diagrams(
    html: &str,
    options: &ToMarkdownOptions,
    sources: &DiagramSources,
) -> String {
    reverse::to_markdown(html, options, sources).markdown
}

/// [`to_markdown_with_diagrams`] plus how many blocks needed a fallback (tests).
#[doc(hidden)]
pub fn to_markdown_report(
    html: &str,
    options: &ToMarkdownOptions,
    sources: &DiagramSources,
) -> Report {
    reverse::to_markdown(html, options, sources)
}

/// Append HTML to existing rich text as a new paragraph.
pub fn append_html(existing: &str, addition: &str) -> String {
    html::append_html(existing, addition)
}

/// Plain text (a `text/plain` description) as HTML: escaped, line breaks as `<br/>`.
pub fn text_to_html(text: &str) -> String {
    html::text_to_html(text)
}
