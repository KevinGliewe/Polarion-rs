//! Polarion HTML to Markdown.
//!
//! The pipeline (synchronous; the DOM and the comrak arena never leave the call):
//! 1. `walk`: the HTML parsed as a fragment (`scraper`), walked into a flow of blocks and styled
//!    inline atoms ([`css`] maps styles, [`raw`] writes what stays HTML);
//! 2. `lines`: paragraphs, line breaks, headings, and the breaks between blocks absorbed as the
//!    forward direction writes them;
//! 3. `build`: a valid comrak tree, with the escapes of `escape`;
//! 4. `check`: each block's Markdown parsed again and converted back; a block whose HTML differs
//!    is built conservatively, else kept as the intended HTML (keep mode). The document is written
//!    with comrak's CommonMark writer (with `parse::options()`), without a trailing newline.

mod build;
mod check;
mod css;
mod escape;
mod lines;
mod raw;
mod walk;

use crate::{DiagramSources, RawHtml, ToMarkdownOptions};

/// The result of a conversion to Markdown (tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The Markdown, without a trailing newline.
    pub markdown: String,
    /// Blocks the self-check had to rebuild conservatively or keep as raw HTML (one more when the
    /// check of the whole document differs).
    pub fallbacks: usize,
    /// Built comrak trees (per block and the whole document) that failed comrak's `validate()`.
    pub invalid: usize,
}

pub(crate) fn to_markdown(
    html: &str,
    options: &ToMarkdownOptions,
    sources: &DiagramSources,
) -> Report {
    if html
        .trim_matches(|c: char| c.is_ascii_whitespace())
        .is_empty()
    {
        return Report {
            markdown: String::new(),
            fallbacks: 0,
            invalid: 0,
        };
    }
    let walk::Walked {
        body,
        links,
        wrappers,
    } = walk::walk(html, options, sources);
    let blocks = lines::blocks(body, options.raw_html == RawHtml::Keep);
    check::render(blocks, &links, &wrappers, options)
}

#[cfg(test)]
mod tests;
