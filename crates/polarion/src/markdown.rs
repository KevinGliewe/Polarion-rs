//! Work item descriptions as Markdown (feature `markdown`), converted by [`polarion_markdown`].
//!
//! Mermaid diagrams are attached to the work item as SVG files named after a hash of their
//! source (`mermaid-<hash>.svg`, see [`polarion_markdown::mermaid`]), and the description shows
//! them as images (`workitemimg:<attachment id>`). Writing renders and uploads only the diagrams
//! that have no attachment yet, renders all of them before it changes anything, and (when it
//! replaces the description) deletes the diagram attachments only the old description showed.
//! Reading fetches the SVGs the description shows and turns them back into
//! ```` ```mermaid ```` blocks: each SVG carries its source.

use std::collections::BTreeSet;

use polarion_markdown::mermaid::{self, Diagram, MermaidError};
use polarion_markdown::{
    self as md, DiagramAttachments, DiagramSources, RawHtml, ToHtmlOptions, ToMarkdownOptions,
    WorkItemIds,
};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::items::WorkItems;
use crate::model::{Attachment, Text, WorkItem};
use crate::query::Fields;
use crate::uri::ItemRef;
use crate::write::WorkItemUpdate;

/// The title of the diagram attachments.
const DIAGRAM_TITLE: &str = "Mermaid diagram";

/// Longest first line of a diagram quoted in messages, in characters.
const QUOTED_LINE: usize = 60;

/// Options of [`WorkItems::description_markdown`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkdownReadOptions {
    /// Formatting Markdown cannot express (colors, fonts, alignment, merged cells, ...): kept as
    /// inline HTML (default), so writing the Markdown back restores it, or dropped.
    pub raw_html: RawHtml,
    /// The ids that writing the Markdown back links (use the same value for both): a link to
    /// such an id is read as the plain id; another one stays its span (keep mode) or becomes its
    /// id as text (drop mode). `None`: the ids with the work item's own prefix (`MP` for `MP-42`).
    pub work_items: Option<WorkItemIds>,
}

/// Options of [`WorkItems::set_description_markdown`] and
/// [`WorkItems::append_description_markdown`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownWriteOptions {
    /// Which `PREFIX-123` ids in the text become work item links. `None` (default): the ids with
    /// the work item's own prefix (`MP` for `MP-42`), so `UTF-8` and the ids of other projects
    /// stay text.
    pub work_items: Option<WorkItemIds>,
    /// Highlight fenced code that names its language (needs the feature `highlight`; default:
    /// on).
    pub highlight: bool,
    /// Keep the diagram attachments the old description showed and the new one does not
    /// (default: they are deleted). Appending never deletes.
    pub keep_orphans: bool,
    /// Write a diagram that cannot be rendered as a code block, with a warning, instead of
    /// failing (default: off).
    pub allow_unrendered: bool,
}

impl Default for MarkdownWriteOptions {
    fn default() -> Self {
        Self {
            work_items: None,
            highlight: true,
            keep_orphans: false,
            allow_unrendered: false,
        }
    }
}

/// The result of writing a work item's description as Markdown (set or append).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct MarkdownUpdate {
    /// Whether the description was written; `false` when it already was this HTML, or there was
    /// nothing to append.
    pub updated: bool,
    /// The mermaid diagrams of the Markdown, in document order, each once.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagrams: Vec<DiagramAttachment>,
    /// Ids of the diagram attachments deleted because the description no longer shows them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deleted: Vec<String>,
    /// Diagrams written as code blocks, attachments that could not be deleted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// A mermaid diagram of written Markdown, and the attachment that shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct DiagramAttachment {
    /// `mermaid-<hash>.svg`, the hash covering the diagram's source.
    pub file_name: String,
    /// The id of the attachment the description shows; none when the diagram was written as a
    /// code block (see the warnings).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<String>,
    /// Uploaded by this call; `false`: an existing attachment was reused (or none was written).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub uploaded: bool,
}

impl<'a> WorkItems<'a> {
    /// The description as Markdown.
    ///
    /// Images of the work item's diagram attachments (`mermaid-<hash>.svg`, as
    /// [`WorkItems::set_description_markdown`] writes them) become ```` ```mermaid ```` blocks
    /// again, from the source their SVG carries. Each is fetched with `getAttachment`, which
    /// needs Polarion 3.20.1 or later: when the server answers with a SOAP fault, that diagram
    /// stays an image, and when the fault says the server does not know the operation, so do
    /// the others (they are not asked for). Only the work item's own attachments count:
    /// `workitemimg:` images and `/wi-attachment/` URLs of this work item. A `text/plain`
    /// description is read as text (without line breaks at its start and white space at its end).
    pub async fn description_markdown(
        &self,
        item: &ItemRef,
        options: &MarkdownReadOptions,
    ) -> Result<String> {
        let fetched = self.get_fields(item, &description_fields()).await?;
        let html = description_html(&fetched);
        let listed = attachment_ids(&fetched.attachments);
        let shown: Vec<String> = own_diagram_refs(&html, item, &fetched)
            .into_iter()
            .filter(|id| listed.contains(id))
            .collect();
        let mut sources = DiagramSources::new();
        for id in shown {
            match self.attachment(item, &id).await {
                Ok(svg) => {
                    let source = String::from_utf8(svg)
                        .ok()
                        .and_then(|svg| mermaid::source_of(&svg));
                    if let Some(source) = source {
                        sources.insert(id, source);
                    }
                }
                // The diagram stays an image. Servers before 3.20.1 do not know `getAttachment`:
                // asking for the other diagrams would only fail again.
                Err(Error::Soap(polarion_soap::Error::Fault { message, .. })) => {
                    if unknown_operation(&message) {
                        break;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        let options = ToMarkdownOptions {
            raw_html: options.raw_html,
            work_items: links(options.work_items.as_ref(), item, &fetched),
        };
        Ok(md::to_markdown_with_diagrams(&html, &options, &sources))
    }

    /// Replace the description with Markdown, converted to Polarion's rich text (HTML).
    ///
    /// `PREFIX-123` ids in the text become work item links as
    /// [`MarkdownWriteOptions::work_items`] says; by default the ids with the work item's own
    /// prefix.
    ///
    /// Mermaid blocks are shown as images of SVG attachments named after a hash of their source
    /// (`mermaid-<hash>.svg`, titled `Mermaid diagram`):
    /// - A diagram that has an attachment of that name is shown from it (the one the old
    ///   description shows, else the newest); it is neither rendered nor uploaded again.
    /// - The others are rendered (feature `mermaid`) and uploaded. All of them are rendered
    ///   before anything is changed: a diagram that cannot be rendered fails the call with
    ///   [`Error::Invalid`] and nothing is written, unless
    ///   [`MarkdownWriteOptions::allow_unrendered`] is set, which writes it as a code block with
    ///   a warning. Without the feature `mermaid` every new diagram is written as a code block,
    ///   with a warning.
    /// - Afterwards, the work item's diagram attachments the old description showed and the new
    ///   one does not are deleted, unless [`MarkdownWriteOptions::keep_orphans`] is set. A failed
    ///   delete is a warning: the description is written by then. Only the old description
    ///   counts, so later calls do not delete such an attachment either (nor one left over when
    ///   the reply to the update was lost): it has to be deleted by hand. Shown means a
    ///   `workitemimg:` image or a `/wi-attachment/` URL of this work item, not an attachment of
    ///   another work item that has the same id.
    ///
    /// A description that already is this HTML is not written again (`updated: false`).
    ///
    /// The requests are not atomic (read, upload, read the attachments again, update, delete).
    /// A diagram uploaded by a call that failed later is not deleted, but the next call that
    /// writes the same diagram shows it.
    ///
    /// Rendering is CPU-bound and blocks the calling thread until it is done: milliseconds for a
    /// typical diagram, seconds for a large one, and the first render of a process also loads
    /// the system fonts (see [`polarion_markdown::mermaid::render_svg`]).
    pub async fn set_description_markdown(
        &self,
        item: &ItemRef,
        markdown: &str,
        options: &MarkdownWriteOptions,
    ) -> Result<MarkdownUpdate> {
        self.write_markdown(item, markdown, options, false).await
    }

    /// Append Markdown to the description as a new paragraph. As
    /// [`WorkItems::set_description_markdown`], except that no attachment is deleted.
    ///
    /// A `text/plain` description is converted to HTML first. Blank Markdown changes nothing
    /// (`updated: false`) and sends nothing.
    ///
    /// The description is read, extended and written back: a change another user makes in
    /// between is lost.
    pub async fn append_description_markdown(
        &self,
        item: &ItemRef,
        markdown: &str,
        options: &MarkdownWriteOptions,
    ) -> Result<MarkdownUpdate> {
        self.write_markdown(item, markdown, options, true).await
    }

    /// Set (`append` false) or append the description: read the description and the
    /// attachments, render the new diagrams, upload them, read their ids, write the HTML, and
    /// (when setting) delete the diagram attachments no longer shown.
    async fn write_markdown(
        &self,
        item: &ItemRef,
        markdown: &str,
        options: &MarkdownWriteOptions,
        append: bool,
    ) -> Result<MarkdownUpdate> {
        // Before anything else, the first read included: a refused write sends nothing.
        self.writable("updateWorkItem", item)?;
        if append && markdown.trim().is_empty() {
            return Ok(MarkdownUpdate::default());
        }
        let diagrams = mermaid::diagrams(markdown);
        let fetched = self.get_fields(item, &description_fields()).await?;
        let old = description_html(&fetched);
        let shown_before = own_diagram_refs(&old, item, &fetched);
        let mut attachments = existing(&fetched.attachments, &diagrams, &shown_before);

        // Every diagram without an attachment is rendered before the first write.
        let mut warnings = Vec::new();
        let mut uploads: Vec<(String, String)> = Vec::new();
        for diagram in &diagrams {
            if attachments.contains_key(&diagram.file_name) {
                continue;
            }
            match mermaid::render_svg(&diagram.source) {
                Ok(svg) => uploads.push((diagram.file_name.clone(), svg)),
                Err(e) => warnings.push(unrendered(&diagram.source, &e, options.allow_unrendered)?),
            }
        }
        for (file_name, svg) in &uploads {
            self.add_attachment(item, file_name, DIAGRAM_TITLE, svg.as_bytes())
                .await?;
        }
        if !uploads.is_empty() {
            // `createAttachment` returns nothing; the ids are `<counter>-<file name>`.
            let listed = self
                .get_fields(item, &Fields::new(["attachments"]))
                .await?
                .attachments;
            for (file_name, _) in &uploads {
                let id = newest(listed.iter().filter(|a| named(a, file_name))).ok_or_else(|| {
                    Error::Invalid(format!(
                        "the diagram {file_name} was uploaded to {item} but is not among its attachments"
                    ))
                })?;
                attachments.insert(file_name.clone(), id);
            }
        }

        let html_options = ToHtmlOptions {
            work_items: links(options.work_items.as_ref(), item, &fetched),
            highlight: options.highlight,
        };
        let html = md::to_html_with_diagrams(markdown, &html_options, &attachments);
        let content = if append {
            md::append_html(&old, &html)
        } else {
            html
        };
        let updated = content != old;
        if updated {
            let update = WorkItemUpdate {
                description: Some(Text {
                    content_type: "text/html".into(),
                    content: content.clone(),
                }),
                ..WorkItemUpdate::default()
            };
            self.update(item, &update).await?;
        }

        let mut deleted = Vec::new();
        if !append && !options.keep_orphans {
            // Every reference counts here, URLs of other work items included: deleting too
            // little is safe.
            let shown = md::diagram_refs(&content);
            let listed = attachment_ids(&fetched.attachments);
            let orphans: Vec<String> = shown_before
                .difference(&shown)
                .filter(|id| listed.contains(*id))
                .cloned()
                .collect();
            for id in orphans {
                match self.delete_attachment(item, &id).await {
                    Ok(()) => deleted.push(id),
                    Err(e) => warnings.push(format!(
                        "the diagram attachment {id} is no longer shown but could not be deleted: \
                         {e} (later writes do not delete it either; delete it by hand)"
                    )),
                }
            }
        }

        let uploaded: BTreeSet<String> = uploads.into_iter().map(|(name, _)| name).collect();
        let diagrams = diagrams
            .into_iter()
            .map(|d| DiagramAttachment {
                attachment: attachments.get(&d.file_name).cloned(),
                uploaded: uploaded.contains(&d.file_name),
                file_name: d.file_name,
            })
            .collect();
        Ok(MarkdownUpdate {
            updated,
            diagrams,
            deleted,
            warnings,
        })
    }
}

/// What the conversions need: the id (for the default links), the description and the
/// attachments (for the diagrams).
fn description_fields() -> Fields {
    Fields::new(["id", "description", "attachments"])
}

/// The description as HTML: a `text/plain` one converted (without the line breaks at its start
/// and the white space at its end, which would become `<br/>`s), none as `""`.
fn description_html(item: &WorkItem) -> String {
    match &item.description {
        Some(text) if is_plain_text(&text.content_type) => {
            md::text_to_html(text.content.trim_start_matches(['\r', '\n']).trim_end())
        }
        Some(text) => text.content.clone(),
        None => String::new(),
    }
}

/// Whether a MIME type (parameters ignored) is `text/plain`.
fn is_plain_text(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case("text/plain")
}

/// The ids that become links: the option, else the ids with the work item's own prefix (from
/// the id in the reference, else the id the server returned), else any.
fn links(option: Option<&WorkItemIds>, item: &ItemRef, fetched: &WorkItem) -> WorkItemIds {
    if let Some(ids) = option {
        return ids.clone();
    }
    let own = item
        .id
        .as_deref()
        .and_then(prefix)
        .or_else(|| fetched.id.as_deref().and_then(prefix));
    own.map_or(WorkItemIds::Any, |p| {
        WorkItemIds::Prefixes(vec![p.to_owned()])
    })
}

/// The prefix of a work item id: `MP` for `MP-42`.
fn prefix(id: &str) -> Option<&str> {
    let (prefix, number) = id.rsplit_once('-')?;
    (!prefix.is_empty() && !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
        .then_some(prefix)
}

/// The work item's own diagram attachments the HTML shows: `workitemimg:` images and
/// `/wi-attachment/PROJECT/ID/…` URLs of this work item. [`md::diagram_refs`] also returns the
/// ids in URLs of other work items, whose attachments can have the same ids (the same diagram
/// uploaded to two work items first gets the same id on both). Diagram ids are ASCII
/// (`<counter>-mermaid-<hash>.svg`), so they appear in the HTML as they are.
fn own_diagram_refs(html: &str, item: &ItemRef, fetched: &WorkItem) -> BTreeSet<String> {
    let lower = html.to_ascii_lowercase();
    let own_url = match (&item.project, item.id.as_ref().or(fetched.id.as_ref())) {
        (Some(project), Some(id)) => Some(format!("/wi-attachment/{project}/{id}/")),
        _ => None,
    };
    md::diagram_refs(html)
        .into_iter()
        .filter(|id| {
            lower.contains(&format!("workitemimg:{}", id.to_ascii_lowercase()))
                || own_url
                    .as_ref()
                    .is_some_and(|url| html.contains(&format!("{url}{id}")))
        })
        .collect()
}

/// Whether a SOAP fault says that the server does not know the operation.
fn unknown_operation(message: &str) -> bool {
    message.to_ascii_lowercase().contains("no such operation")
}

/// The ids of the attachments.
fn attachment_ids(attachments: &[Attachment]) -> BTreeSet<String> {
    attachments.iter().filter_map(|a| a.id.clone()).collect()
}

/// The attachment that shows each diagram that has one: among the attachments with the
/// diagram's file name, the one the old description shows, else the newest.
fn existing(
    listed: &[Attachment],
    diagrams: &[Diagram],
    shown: &BTreeSet<String>,
) -> DiagramAttachments {
    diagrams
        .iter()
        .filter_map(|d| {
            let candidates = || listed.iter().filter(|a| named(a, &d.file_name));
            let id =
                newest(candidates().filter(|a| a.id.as_ref().is_some_and(|id| shown.contains(id))))
                    .or_else(|| newest(candidates()))?;
            Some((d.file_name.clone(), id))
        })
        .collect()
}

/// Whether an attachment has this file name: its `fileName`, else the one in its id.
fn named(attachment: &Attachment, file_name: &str) -> bool {
    let name = match (&attachment.file_name, &attachment.id) {
        (Some(name), _) => name.as_str(),
        (None, Some(id)) => md::attachment_file_name(id),
        (None, None) => return false,
    };
    name == file_name
}

/// The id of the newest attachment: the highest counter (ids are `<counter>-<file name>`).
fn newest<'x>(attachments: impl Iterator<Item = &'x Attachment>) -> Option<String> {
    attachments
        .filter_map(|a| a.id.as_deref())
        .max_by_key(|id| counter(id))
        .map(str::to_owned)
}

/// The counter of an attachment id (`12` of `12-spec.png`).
fn counter(id: &str) -> Option<u64> {
    let (counter, _) = id.split_once('-')?;
    if counter.is_empty() || !counter.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    counter.parse().ok()
}

/// The warning for a diagram written as a code block, or the error when that is not allowed.
/// The diagram is named by its first line: a number would not match the Markdown's blocks,
/// because empty and repeated blocks are no diagrams of their own.
fn unrendered(source: &str, error: &MermaidError, allow: bool) -> Result<String> {
    let first = first_line(source);
    match error {
        MermaidError::Disabled => Ok(format!(
            "mermaid diagram ({first}) is written as a code block: {error}"
        )),
        _ if allow => Ok(format!(
            "mermaid diagram ({first}) could not be rendered and is written as a code block: {error}"
        )),
        _ => Err(Error::Invalid(format!(
            "mermaid diagram ({first}) could not be rendered: {error}; allow-unrendered writes it as a code block"
        ))),
    }
}

/// The first line of a diagram's source, shortened for messages.
fn first_line(source: &str) -> String {
    let line = source.lines().next().unwrap_or_default().trim();
    match line.char_indices().nth(QUOTED_LINE) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn attachment(id: &str, file_name: Option<&str>) -> Attachment {
        Attachment {
            id: Some(id.to_owned()),
            file_name: file_name.map(str::to_owned),
            ..Attachment::default()
        }
    }

    fn text(content_type: &str, content: &str) -> WorkItem {
        WorkItem {
            description: Some(Text {
                content_type: content_type.into(),
                content: content.into(),
            }),
            ..WorkItem::default()
        }
    }

    #[test]
    fn default_options() {
        let write = MarkdownWriteOptions::default();
        assert!(write.highlight);
        assert!(!write.keep_orphans && !write.allow_unrendered);
        assert_eq!(write.work_items, None);
        let read = MarkdownReadOptions::default();
        assert_eq!((read.raw_html, read.work_items), (RawHtml::Keep, None));
    }

    #[test]
    fn links_default_to_the_items_own_prefix() {
        let mp = || WorkItemIds::Prefixes(vec!["MP".to_owned()]);
        let fetched = WorkItem {
            id: Some("MP-42".into()),
            ..WorkItem::default()
        };
        let none = WorkItem::default();
        let by_uri = ItemRef::from_uri("not parsed");
        assert_eq!(links(None, &ItemRef::new("P", "MP-42"), &none), mp());
        assert_eq!(links(None, &by_uri, &fetched), mp());
        assert_eq!(links(None, &by_uri, &none), WorkItemIds::Any);
        let odd = ItemRef::new("P", "odd");
        assert_eq!(links(None, &odd, &fetched), mp());
        assert_eq!(links(None, &odd, &none), WorkItemIds::Any);
        assert_eq!(
            links(
                Some(&WorkItemIds::Off),
                &ItemRef::new("P", "MP-42"),
                &fetched
            ),
            WorkItemIds::Off
        );

        assert_eq!(prefix("MY_PROJ-12"), Some("MY_PROJ"));
        for id in ["MP-", "-1", "MP42", "MP-4a", ""] {
            assert_eq!(prefix(id), None, "{id:?}");
        }
    }

    #[test]
    fn descriptions_as_html() {
        assert_eq!(description_html(&WorkItem::default()), "");
        assert_eq!(description_html(&text("text/html", "<b>x</b>")), "<b>x</b>");
        assert_eq!(
            description_html(&text("text/plain", "a < b\nc")),
            "a &lt; b<br/>c"
        );
        assert_eq!(
            description_html(&text("Text/Plain; charset=UTF-8", "a&b")),
            "a&amp;b"
        );
    }

    #[test]
    fn existing_attachments_prefer_the_shown_one_else_the_newest() {
        let d = mermaid::diagram("graph TD\n    A-->B");
        let other = mermaid::diagram("graph LR\n    X-->Y");
        let name = d.file_name.as_str();
        let listed = [
            attachment(&format!("9-{name}"), Some(name)),
            attachment(&format!("10-{name}"), Some(name)),
            attachment(&format!("2-{name}"), Some(name)),
            // No `fileName`: the one in the id counts.
            attachment(&format!("3-{}", other.file_name), None),
            attachment("11-spec.png", Some("spec.png")),
        ];
        let diagrams = [d.clone(), other.clone()];

        let by_counter = existing(&listed, &diagrams, &BTreeSet::new());
        assert_eq!(by_counter.get(name).cloned(), Some(format!("10-{name}")));
        assert_eq!(
            by_counter.get(&other.file_name).cloned(),
            Some(format!("3-{}", other.file_name))
        );

        let shown = BTreeSet::from([format!("2-{name}"), "11-spec.png".to_owned()]);
        let reused = existing(&listed, &diagrams, &shown);
        assert_eq!(reused.get(name).cloned(), Some(format!("2-{name}")));

        assert!(existing(&listed[4..], &diagrams, &shown).is_empty());
        assert_eq!(counter("12-x-1.svg"), Some(12));
        assert_eq!(
            (counter("x"), counter("-x"), counter("+1-x")),
            (None, None, None)
        );
    }

    #[test]
    fn unrendered_diagrams() {
        let source = "flowchart LR\n  A --> B";
        let disabled = unrendered(source, &MermaidError::Disabled, false).unwrap();
        assert_eq!(
            disabled,
            "mermaid diagram (flowchart LR) is written as a code block: rendering mermaid diagrams needs the `mermaid` feature"
        );
        let invalid = MermaidError::Invalid("bad".into());
        assert_eq!(
            unrendered(source, &invalid, true).unwrap(),
            "mermaid diagram (flowchart LR) could not be rendered and is written as a code block: bad"
        );
        match unrendered(source, &invalid, false) {
            Err(Error::Invalid(message)) => assert_eq!(
                message,
                "mermaid diagram (flowchart LR) could not be rendered: bad; allow-unrendered writes it as a code block"
            ),
            other => panic!("{other:?}"),
        }

        let long = "x".repeat(QUOTED_LINE + 5);
        assert_eq!(first_line(&long), format!("{}…", &long[..QUOTED_LINE]));
        assert_eq!(
            first_line(&"ä".repeat(QUOTED_LINE)),
            "ä".repeat(QUOTED_LINE)
        );
        assert_eq!(first_line("  graph TD  \nA"), "graph TD");
    }

    #[test]
    fn serialized_form() {
        assert_eq!(
            serde_json::to_value(MarkdownUpdate::default()).unwrap(),
            json!({"updated": false})
        );
        let update = MarkdownUpdate {
            updated: true,
            diagrams: vec![
                DiagramAttachment {
                    file_name: "mermaid-0123456789abcdef.svg".into(),
                    attachment: Some("1-mermaid-0123456789abcdef.svg".into()),
                    uploaded: true,
                },
                DiagramAttachment {
                    file_name: "mermaid-fedcba9876543210.svg".into(),
                    attachment: Some("2-mermaid-fedcba9876543210.svg".into()),
                    uploaded: false,
                },
                DiagramAttachment {
                    file_name: "mermaid-00000000ffffffff.svg".into(),
                    attachment: None,
                    uploaded: false,
                },
            ],
            deleted: vec!["3-mermaid-1111111111111111.svg".into()],
            warnings: vec!["w".into()],
        };
        assert_eq!(
            serde_json::to_value(update).unwrap(),
            json!({
                "updated": true,
                "diagrams": [
                    {"fileName": "mermaid-0123456789abcdef.svg", "attachment": "1-mermaid-0123456789abcdef.svg", "uploaded": true},
                    {"fileName": "mermaid-fedcba9876543210.svg", "attachment": "2-mermaid-fedcba9876543210.svg"},
                    {"fileName": "mermaid-00000000ffffffff.svg"},
                ],
                "deleted": ["3-mermaid-1111111111111111.svg"],
                "warnings": ["w"],
            })
        );
    }
}
