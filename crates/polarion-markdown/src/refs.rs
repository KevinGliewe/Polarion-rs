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
    fn file_names() {
        assert_eq!(attachment_file_name("12-mermaid-x.svg"), "mermaid-x.svg");
        assert_eq!(attachment_file_name("mermaid-x.svg"), "mermaid-x.svg");
        assert_eq!(attachment_file_name("-x"), "-x");
        assert_eq!(attachment_file_name("x"), "x");
    }
}
