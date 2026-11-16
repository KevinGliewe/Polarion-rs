//! Attachment references in Polarion HTML.

use std::collections::BTreeSet;

use crate::html::percent_decode;
use crate::styles::ATTACHMENT_SCHEME;

/// Ids of the attachments the HTML references: `workitemimg:ID` and the last segment of
/// `/wi-attachment/…/ID` URLs in `src` and `href`, percent-decoded.
pub fn attachment_refs(html: &str) -> BTreeSet<String> {
    let document = scraper::Html::parse_fragment(html);
    document
        .tree
        .nodes()
        .filter_map(|node| node.value().as_element())
        .flat_map(|element| [element.attr("src"), element.attr("href")])
        .flatten()
        .filter_map(attachment_id)
        .collect()
}

/// The attachment id a URL references, if it references one: `workitemimg:ID` or the last segment
/// of a `/wi-attachment/…/ID` URL, percent-decoded.
pub(crate) fn attachment_id(url: &str) -> Option<String> {
    let url = url.trim();
    let scheme = ATTACHMENT_SCHEME.len();
    let id = if url.len() >= scheme
        && url.is_char_boundary(scheme)
        && url[..scheme].eq_ignore_ascii_case(ATTACHMENT_SCHEME)
    {
        url[scheme..].split(['?', '#']).next()?
    } else {
        let path = &url[url.find("/wi-attachment/")? + "/wi-attachment/".len()..];
        path.split(['?', '#'])
            .next()?
            .rsplit('/')
            .find(|segment| !segment.is_empty())?
    };
    let id = percent_decode(id);
    (!id.is_empty()).then(|| id.into_owned())
}

/// The [`attachment_refs`] whose file name is a diagram's ([`crate::mermaid::is_diagram_file`]).
pub fn diagram_refs(html: &str) -> BTreeSet<String> {
    attachment_refs(html)
        .into_iter()
        .filter(|id| crate::mermaid::is_diagram_file(attachment_file_name(id)))
        .collect()
}

/// The sources of the images Markdown shows, in document order, each once: Markdown images and
/// `<img>` tags in raw HTML, as [`crate::ImageAttachments`] names them. Images the conversion
/// does not write (a dangerous source, nesting too deep) are not among them.
pub fn image_urls(markdown: &str) -> Vec<String> {
    let options = crate::ToHtmlOptions {
        work_items: crate::WorkItemIds::Off,
        highlight: false,
    };
    let html = crate::to_html(markdown, &options);
    let mut urls: Vec<String> = Vec::new();
    for url in crate::html::img_srcs(&html) {
        if !url.is_empty() && !urls.contains(&url) {
            urls.push(url);
        }
    }
    urls
}

/// The images of HTML that show an attachment, in order, each source once: the source (as
/// [`crate::ImagePaths`] names it) and the attachment's id ([`attachment_refs`]).
pub fn image_refs(html: &str) -> Vec<(String, String)> {
    let mut refs: Vec<(String, String)> = Vec::new();
    for src in crate::html::img_srcs(html) {
        if let Some(id) = attachment_id(&src)
            && !refs.iter().any(|(s, _)| *s == src)
        {
            refs.push((src, id));
        }
    }
    refs
}

/// The file name of an attachment id: Polarion's ids are `<counter>-<file name>`.
pub fn attachment_file_name(id: &str) -> &str {
    match id.split_once('-') {
        Some((counter, name))
            if !counter.is_empty() && counter.bytes().all(|b| b.is_ascii_digit()) =>
        {
            name
        }
        _ => id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(html: &str) -> Vec<String> {
        attachment_refs(html).into_iter().collect()
    }

    #[test]
    fn workitemimg_and_wi_attachment_urls() {
        let html = concat!(
            r#"<img src="workitemimg:1-mermaid-0123456789abcdef.svg" title="x" style="max-width: 650px;"/>"#,
            r#"<p><img src="/polarion/wi-attachment/PROJ/PROJ-1/2-spec.png?revision=12" width="10"></p>"#,
            r#"<a href="https://polarion.example.com/polarion/wi-attachment/PROJ/PROJ-1/3-doc%20v2.pdf#page=2">doc</a>"#,
            r#"<img src="WorkItemImg:4-%C3%A4.png">"#,
            r#"<img src="https://example.com/x.png"><a href="workitemimg:">empty</a>"#,
            r#"<a href="/wi-attachment/">none</a><span data-src="workitemimg:5-no.png"></span>"#,
            "<img src=\"workitemimg:6-a&amp;b.png\">",
        );
        assert_eq!(
            refs(html),
            [
                "1-mermaid-0123456789abcdef.svg",
                "2-spec.png",
                "3-doc v2.pdf",
                "4-ä.png",
                "6-a&b.png"
            ]
        );
        assert!(refs("").is_empty());
        assert!(refs("no <b>refs</b>").is_empty());
    }

    #[test]
    fn diagram_refs_are_the_diagram_files() {
        let html = concat!(
            r#"<img src="workitemimg:1-mermaid-0123456789abcdef.svg"><img src="workitemimg:2-spec.png">"#,
            r#"<img src="workitemimg:3-mermaid-0123.svg"><img src="workitemimg:mermaid-fedcba9876543210.svg">"#,
        );
        assert_eq!(
            diagram_refs(html).into_iter().collect::<Vec<_>>(),
            [
                "1-mermaid-0123456789abcdef.svg",
                "mermaid-fedcba9876543210.svg"
            ]
        );
    }

    #[test]
    fn images_of_markdown_and_html() {
        let markdown = concat!(
            "![a](img/a.png) and ![](<b c.png> \"t\") ![again](img/a.png)\n\n",
            "<img src=\"d&amp;e.png\" style=\"width: 10px;\">\n\n",
            "```\n![no](code.png)\n```\n\n![x](javascript:alert(1))\n",
        );
        assert_eq!(image_urls(markdown), ["img/a.png", "b c.png", "d&e.png"]);
        let html = concat!(
            r#"<img src="workitemimg:1-a.png"/><img src=" workitemimg:1-a.png "/>"#,
            r#"<img src="/polarion/wi-attachment/P/P-1/2-b%20c.png"><img src="https://x/y.png">"#,
            r#"<a href="workitemimg:3-doc.pdf">doc</a>"#,
        );
        assert_eq!(
            image_refs(html),
            [
                ("workitemimg:1-a.png".to_owned(), "1-a.png".to_owned()),
                (
                    "/polarion/wi-attachment/P/P-1/2-b%20c.png".to_owned(),
                    "2-b c.png".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn file_names() {
        assert_eq!(attachment_file_name("12-mermaid-x.svg"), "mermaid-x.svg");
        assert_eq!(attachment_file_name("mermaid-x.svg"), "mermaid-x.svg");
        assert_eq!(attachment_file_name("-x"), "-x");
        assert_eq!(attachment_file_name("x"), "x");
    }
}
