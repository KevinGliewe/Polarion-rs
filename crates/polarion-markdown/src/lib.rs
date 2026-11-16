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
pub use refs::{attachment_file_name, attachment_refs, diagram_refs, image_refs, image_urls};
#[doc(hidden)]
pub use reverse::Report;

/// Attachment id by diagram file name (`mermaid-<hash>.svg`, see [`mermaid::Diagram`]).
pub type DiagramAttachments = BTreeMap<String, String>;

/// Mermaid source by attachment id (see [`mermaid::source_of`]).
pub type DiagramSources = BTreeMap<String, String>;

/// Attachment id by image source, as written in the Markdown ([`image_urls`]): the images shown
/// from attachments instead.
pub type ImageAttachments = BTreeMap<String, String>;

/// The source to write for an image, by its source in the HTML ([`image_refs`]).
pub type ImagePaths = BTreeMap<String, String>;

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

/// [`to_html_with_diagrams`]; an image whose source is in `images` (a Markdown image, or an
/// `<img>` in raw HTML) is shown from that attachment (`src="workitemimg:ID"`).
pub fn to_html_with_images(
    markdown: &str,
    options: &ToHtmlOptions,
    diagrams: &DiagramAttachments,
    images: &ImageAttachments,
) -> String {
    let html = emit::to_html(markdown, options, diagrams);
    if images.is_empty() {
        return html;
    }
    let attachment = |src: &str| {
        images
            .get(src)
            .map(|id| format!("{}{}", styles::ATTACHMENT_SCHEME, html::percent_encode(id)))
    };
    html::rewrite_img_src(&html, &attachment).into_owned()
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

/// [`to_markdown_with_diagrams`]; an image whose source is in `paths` is written with that source
/// instead (a file the attachment was saved to). [`to_html_with_images`] with the inverse map
/// shows the attachments again.
pub fn to_markdown_with_images(
    html: &str,
    options: &ToMarkdownOptions,
    sources: &DiagramSources,
    paths: &ImagePaths,
) -> String {
    if paths.is_empty() {
        return to_markdown_with_diagrams(html, options, sources);
    }
    let html = html::rewrite_img_src(html, &|src: &str| paths.get(src).cloned());
    to_markdown_with_diagrams(&html, options, sources)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_shown_from_attachments_and_read_back_as_files() {
        let markdown = concat!(
            "![shot](img/a.png \"A\") and ![](https://example.com/x.png)\n\n",
            "<img src=\"img/b c.png\" style=\"width: 100px;\">",
        );
        let images: ImageAttachments = [("img/a.png", "1-a.png"), ("img/b c.png", "2-b c.png")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let options = ToHtmlOptions::default();
        let html = to_html_with_images(markdown, &options, &DiagramAttachments::new(), &images);
        assert_eq!(
            html,
            concat!(
                r#"<img src="workitemimg:1-a.png" title="A" alt="shot" style="max-width: 650px;"/>"#,
                r#" and <img src="https://example.com/x.png" style="max-width: 650px;"/><br/><br/>"#,
                r#"<img src="workitemimg:2-b%20c.png" style="width: 100px;">"#,
            )
        );
        // Without the map nothing changes.
        assert_eq!(
            to_html_with_images(
                markdown,
                &options,
                &DiagramAttachments::new(),
                &ImageAttachments::new()
            ),
            to_html(markdown, &options)
        );

        let paths: ImagePaths = image_refs(&html)
            .into_iter()
            .map(|(src, id)| (src, format!("P-1/{}", id.replace(' ', "_"))))
            .collect();
        let back = to_markdown_with_images(
            &html,
            &ToMarkdownOptions::default(),
            &DiagramSources::new(),
            &paths,
        );
        assert_eq!(
            back,
            concat!(
                "![shot](P-1/1-a.png \"A\") and ![](https://example.com/x.png)\n\n",
                "<img src=\"P-1/2-b_c.png\" style=\"width: 100px;\"/>",
            )
        );
        // Written back with the inverse map, the same attachments are shown (raw HTML comes back
        // self-closed).
        let inverse: ImageAttachments = paths
            .iter()
            .map(|(src, path)| (path.clone(), refs::attachment_id(src).unwrap()))
            .collect();
        assert_eq!(
            to_html_with_images(&back, &options, &DiagramAttachments::new(), &inverse),
            html.replace("100px;\">", "100px;\"/>")
        );
        assert_eq!(
            image_urls(&back),
            ["P-1/1-a.png", "https://example.com/x.png", "P-1/2-b_c.png"]
        );
    }
}
