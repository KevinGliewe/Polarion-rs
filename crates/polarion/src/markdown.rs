//! Work item descriptions as Markdown (feature `markdown`), converted by [`polarion_markdown`].
//!
//! Mermaid diagrams are attached to the work item as SVG files named after a hash of their
//! source (`mermaid-<hash>.svg`, see [`polarion_markdown::mermaid`]), and the description shows
//! them as images (`workitemimg:<attachment id>`). Writing renders and uploads only the diagrams
//! that have no attachment yet, renders all of them before it changes anything, and (when it
//! replaces the description) deletes the diagram attachments only the old description showed.
//! Reading fetches the SVGs the description shows and turns them back into
//! ```` ```mermaid ```` blocks: each SVG carries its source.

use std::collections::{BTreeMap, BTreeSet};

use polarion_markdown::mermaid::{self, Diagram, MermaidError};
use polarion_markdown::{
    self as md, DiagramAttachments, DiagramSources, ImageAttachments, ImagePaths, RawHtml,
    ToHtmlOptions, ToMarkdownOptions, WorkItemIds,
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
    /// Images of the Markdown to show from attachments of the work item (default: none): each is
    /// attached unless the work item already has an attachment with the same content.
    pub images: Vec<LocalImage>,
}

/// An image of the Markdown and its content (see [`MarkdownWriteOptions::images`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalImage {
    /// The image's source as [`polarion_markdown::image_urls`] returns it for the Markdown.
    pub url: String,
    /// The file name to attach it under.
    pub file_name: String,
    pub data: Vec<u8>,
}

impl Default for MarkdownWriteOptions {
    fn default() -> Self {
        Self {
            work_items: None,
            highlight: true,
            keep_orphans: false,
            allow_unrendered: false,
            images: Vec::new(),
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
    /// The images shown from attachments ([`MarkdownWriteOptions::images`]), in the order given.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageAttachment>,
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

/// An image of written Markdown, and the attachment that shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct ImageAttachment {
    /// The image's source in the Markdown.
    pub url: String,
    /// The id of the attachment the description shows.
    pub attachment: String,
    /// Uploaded by this call; `false`: an attachment with the same content was reused.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub uploaded: bool,
}

/// A description as Markdown with the images it shows ([`WorkItems::description_markdown_export`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkdownExport {
    /// The work item's id, when the reference or the server gave one.
    pub id: Option<String>,
    /// The Markdown; the exported images have their `path` as source.
    pub markdown: String,
    /// The image attachments the description shows, each once.
    pub images: Vec<ExportedImage>,
    /// Images that could not be fetched: they keep their `workitemimg:` source.
    pub warnings: Vec<String>,
}

/// An image attachment a description shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedImage {
    /// The source the Markdown names it by: the image folder, `/`, and a file name made of the
    /// attachment id (only `A-Z a-z 0-9 . _ -`, anything else as `_`).
    pub path: String,
    /// The attachment's id.
    pub attachment: String,
    pub data: Vec<u8>,
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
        let sources = self.diagram_sources(item, &html, &fetched).await?;
        let options = ToMarkdownOptions {
            raw_html: options.raw_html,
            work_items: links(options.work_items.as_ref(), item, &fetched),
        };
        Ok(md::to_markdown_with_diagrams(&html, &options, &sources))
    }

    /// [`WorkItems::description_markdown`] together with the images the description shows.
    ///
    /// Every image of an attachment of this work item (a `workitemimg:` image or a
    /// `/wi-attachment/` URL of this work item) is fetched with `getAttachment` and written in
    /// the Markdown as `<image_dir>/<file name>` (see [`ExportedImage::path`]); with an empty
    /// `image_dir`, as the file name alone. Diagrams that become ```` ```mermaid ```` blocks are
    /// not among the images. An image the server answers with a SOAP fault for keeps its source
    /// and is named in the warnings; when the fault says the server does not know the operation
    /// (before Polarion 3.20.1), the remaining images are not asked for.
    ///
    /// Writing the Markdown back with these images as [`MarkdownWriteOptions::images`] shows the
    /// same attachments without uploading them again.
    pub async fn description_markdown_export(
        &self,
        item: &ItemRef,
        options: &MarkdownReadOptions,
        image_dir: &str,
    ) -> Result<MarkdownExport> {
        let fetched = self.get_fields(item, &description_fields()).await?;
        let html = description_html(&fetched);
        let sources = self.diagram_sources(item, &html, &fetched).await?;
        let listed = attachment_ids(&fetched.attachments);
        let own_url = own_attachment_url(item, &fetched);

        let mut paths = ImagePaths::new();
        let mut images: Vec<ExportedImage> = Vec::new();
        let mut warnings = Vec::new();
        // Attachments asked for that the server did not return.
        let mut failed: BTreeSet<String> = BTreeSet::new();
        let mut unknown = false;
        for (src, id) in md::image_refs(&html) {
            let own = src.to_ascii_lowercase().starts_with("workitemimg:")
                || own_url
                    .as_ref()
                    .is_some_and(|url| src.contains(url.as_str()));
            if !own || !listed.contains(&id) || sources.contains_key(&id) || failed.contains(&id) {
                continue;
            }
            if let Some(image) = images.iter().find(|i| i.attachment == id) {
                paths.insert(src, image.path.clone());
                continue;
            }
            if unknown {
                continue;
            }
            match self.attachment(item, &id).await {
                Ok(data) => {
                    let name = safe_file_name(&id, images.iter().map(|i| file_of(&i.path)));
                    let path = if image_dir.is_empty() {
                        name
                    } else {
                        format!("{image_dir}/{name}")
                    };
                    paths.insert(src, path.clone());
                    images.push(ExportedImage {
                        path,
                        attachment: id,
                        data,
                    });
                }
                Err(Error::Soap(polarion_soap::Error::Fault { message, .. })) => {
                    if unknown_operation(&message) {
                        unknown = true;
                        warnings.push(format!(
                            "the server cannot return attachments (getAttachment needs Polarion \
                             3.20.1 or later): the image {id} and any after it keep their \
                             `workitemimg:` source"
                        ));
                    } else {
                        warnings.push(format!(
                            "the image {id} could not be fetched and keeps its source: {message}"
                        ));
                    }
                    failed.insert(id);
                }
                Err(e) => return Err(e),
            }
        }
        let options = ToMarkdownOptions {
            raw_html: options.raw_html,
            work_items: links(options.work_items.as_ref(), item, &fetched),
        };
        Ok(MarkdownExport {
            id: item.id.clone().or_else(|| fetched.id.clone()),
            markdown: md::to_markdown_with_images(&html, &options, &sources, &paths),
            images,
            warnings,
        })
    }

    /// The sources of the diagram attachments of the work item the description shows, by
    /// attachment id, read from their SVGs.
    async fn diagram_sources(
        &self,
        item: &ItemRef,
        html: &str,
        fetched: &WorkItem,
    ) -> Result<DiagramSources> {
        let listed = attachment_ids(&fetched.attachments);
        let shown: Vec<String> = own_diagram_refs(html, item, fetched)
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
        Ok(sources)
    }

    /// The attachment of the work item that has the content of an image, if any: among the
    /// listed attachments of the same length, the one whose id is the file name of the image
    /// first, then those with its file name, then the others. A SOAP fault means it cannot be
    /// compared.
    async fn same_content(
        &self,
        item: &ItemRef,
        listed: &[Attachment],
        image: &LocalImage,
    ) -> Result<Option<String>> {
        let length = i64::try_from(image.data.len()).ok();
        let mut candidates: Vec<(u8, &str)> = listed
            .iter()
            .filter(|a| a.length.is_none() || a.length == length)
            .filter_map(|a| {
                let id = a.id.as_deref()?;
                let rank = if id == image.file_name {
                    0
                } else if named(a, &image.file_name) {
                    1
                } else {
                    2
                };
                Some((rank, id))
            })
            .collect();
        candidates.sort_by_key(|(rank, _)| *rank);
        for (_, id) in candidates {
            match self.attachment(item, id).await {
                Ok(data) if data == image.data => return Ok(Some(id.to_owned())),
                Ok(_) => {}
                Err(Error::Soap(polarion_soap::Error::Fault { message, .. })) => {
                    if unknown_operation(&message) {
                        break;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(None)
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
    /// The reads and the rendering come first. Then the uploads, then the update and the deletes;
    /// each of the two steps of more than one write runs as one transaction in a Polarion session
    /// of its own. Two steps, because Polarion does not list an attachment uploaded in a
    /// transaction until it is committed, and the description needs the new attachments' ids. A
    /// call that fails in the second step leaves the uploaded diagrams behind, not shown; a later
    /// write of the same diagrams shows them instead of uploading them again. A delete that fails
    /// stays a warning, as said above.
    ///
    /// Images ([`MarkdownWriteOptions::images`]): each image of the Markdown named there is shown
    /// from an attachment of the work item. An attachment that already has the content of the
    /// image (same length, then compared with `getAttachment`) is reused; the others are
    /// uploaded with the diagrams, under the file name of the image (titled with it). Two
    /// images with the same file name and different content are refused before anything is
    /// written. Image attachments are never deleted, whatever the old description showed.
    ///
    /// Rendering is CPU-bound and blocks the calling thread until it is done: milliseconds for a
    /// typical diagram, up to a second for a large one (see
    /// [`polarion_markdown::mermaid::render_svg`]).
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
        let attachments = existing(&fetched.attachments, &diagrams, &shown_before);

        // Every diagram without an attachment is rendered before the first write.
        let mut warnings = Vec::new();
        let mut uploads: Vec<Upload> = Vec::new();
        for diagram in &diagrams {
            if attachments.contains_key(&diagram.file_name) {
                continue;
            }
            match mermaid::render_svg(&diagram.source) {
                Ok(svg) => uploads.push(Upload {
                    file_name: diagram.file_name.clone(),
                    title: DIAGRAM_TITLE.to_owned(),
                    data: svg.into_bytes(),
                }),
                Err(e) => warnings.push(unrendered(&diagram.source, &e, options.allow_unrendered)?),
            }
        }

        // The images: the attachment that has the content of each, else the name to upload it
        // under. All of it is read and checked before the first write.
        let mut images = ImageAttachments::new();
        let mut new_images: BTreeMap<String, String> = BTreeMap::new();
        for image in &options.images {
            if let Some(id) = self.same_content(item, &fetched.attachments, image).await? {
                images.insert(image.url.clone(), id);
                continue;
            }
            let file_name = upload_name(image, &fetched.attachments);
            match uploads.iter().find(|u| u.file_name == file_name) {
                Some(upload) if upload.data == image.data => {}
                Some(_) => {
                    return Err(Error::Invalid(format!(
                        "two different images would be attached as {file_name}: rename one of them"
                    )));
                }
                None => uploads.push(Upload {
                    file_name: file_name.clone(),
                    title: file_name.clone(),
                    data: image.data.clone(),
                }),
            }
            new_images.insert(image.url.clone(), file_name);
        }

        let mut plan = Plan {
            item: item.clone(),
            markdown: markdown.to_owned(),
            html_options: ToHtmlOptions {
                work_items: links(options.work_items.as_ref(), item, &fetched),
                highlight: options.highlight,
            },
            old,
            append,
            prune: !append && !options.keep_orphans,
            shown_before,
            listed: attachment_ids(&fetched.attachments),
            uploads,
            attachments,
            images,
        };
        let uploaded: BTreeSet<String> = plan.uploads.iter().map(|u| u.file_name.clone()).collect();
        if !plan.uploads.is_empty() {
            let uploads = std::mem::take(&mut plan.uploads);
            let target = item.clone();
            self.polarion
                .atomic("createAttachment", uploads.len(), move |p| async move {
                    let items = p.items();
                    for upload in &uploads {
                        items
                            .add_attachment(&target, &upload.file_name, &upload.title, &upload.data)
                            .await?;
                    }
                    Ok(())
                })
                .await?;
            // Committed, so listed. `createAttachment` returns nothing; the ids are
            // `<counter>-<file name>`.
            let listed = self
                .get_fields(item, &Fields::new(["attachments"]))
                .await?
                .attachments;
            let mut ids: BTreeMap<&String, String> = BTreeMap::new();
            for file_name in &uploaded {
                let id =
                    newest(listed.iter().filter(|a| named(a, file_name))).ok_or_else(|| {
                        Error::Invalid(format!(
                            "{file_name} was uploaded to {item} but is not among its attachments"
                        ))
                    })?;
                ids.insert(file_name, id);
            }
            for diagram in &diagrams {
                if let Some(id) = ids.get(&diagram.file_name) {
                    plan.attachments
                        .insert(diagram.file_name.clone(), id.clone());
                }
            }
            for (url, file_name) in &new_images {
                if let Some(id) = ids.get(file_name) {
                    plan.images.insert(url.clone(), id.clone());
                }
            }
        }
        let images: Vec<ImageAttachment> = options
            .images
            .iter()
            .filter_map(|image| {
                Some(ImageAttachment {
                    url: image.url.clone(),
                    attachment: plan.images.get(&image.url)?.clone(),
                    uploaded: new_images.contains_key(&image.url),
                })
            })
            .collect();
        let (content, updated) = plan.content(&plan.attachments);
        let writes = usize::from(updated) + plan.orphans(&content).len();
        let done = self
            .polarion
            .atomic("updateWorkItem", writes, move |p| {
                plan.execute(p, content, updated)
            })
            .await?;
        warnings.extend(done.warnings);

        let diagrams = diagrams
            .into_iter()
            .map(|d| DiagramAttachment {
                attachment: done.attachments.get(&d.file_name).cloned(),
                uploaded: uploaded.contains(&d.file_name),
                file_name: d.file_name,
            })
            .collect();
        Ok(MarkdownUpdate {
            updated: done.updated,
            diagrams,
            images,
            deleted: done.deleted,
            warnings,
        })
    }
}

/// The writes of a Markdown description, planned on the client: everything
/// [`Plan::execute`] needs, owned, so it can run in a session of its own.
struct Plan {
    item: ItemRef,
    markdown: String,
    html_options: ToHtmlOptions,
    /// The description before, as HTML.
    old: String,
    append: bool,
    /// Delete the diagram attachments no longer shown.
    prune: bool,
    /// The diagram attachments of this work item the old description shows.
    shown_before: BTreeSet<String>,
    /// The attachment ids the work item had.
    listed: BTreeSet<String>,
    /// Rendered diagrams and new images to upload.
    uploads: Vec<Upload>,
    /// The diagrams that have an attachment (the uploaded ones once they are): file name and
    /// attachment id.
    attachments: DiagramAttachments,
    /// The images that have an attachment (the uploaded ones once they are): source in the
    /// Markdown and attachment id.
    images: ImageAttachments,
}

/// An attachment to create.
struct Upload {
    file_name: String,
    title: String,
    data: Vec<u8>,
}

/// What [`Plan::execute`] did.
struct Done {
    updated: bool,
    attachments: DiagramAttachments,
    deleted: Vec<String>,
    warnings: Vec<String>,
}

impl Plan {
    /// The new description with these diagram attachments, and whether it differs from the old.
    fn content(&self, attachments: &DiagramAttachments) -> (String, bool) {
        let html = md::to_html_with_images(
            &self.markdown,
            &self.html_options,
            attachments,
            &self.images,
        );
        let content = if self.append {
            md::append_html(&self.old, &html)
        } else {
            html
        };
        let updated = content != self.old;
        (content, updated)
    }

    /// The attachments to delete once `content` is written.
    fn orphans(&self, content: &str) -> Vec<String> {
        if !self.prune {
            return Vec::new();
        }
        // Every reference counts here, URLs of other work items included: deleting too little
        // is safe.
        let shown = md::diagram_refs(content);
        self.shown_before
            .difference(&shown)
            .filter(|id| self.listed.contains(*id))
            .cloned()
            .collect()
    }

    /// Write `content` (when `updated`), then delete the orphans: through `p`, the client or
    /// (several writes) the session of its own of [`crate::Polarion::atomic`].
    async fn execute(self, p: crate::Polarion, content: String, updated: bool) -> Result<Done> {
        let items = p.items();
        let item = &self.item;
        if updated {
            let update = WorkItemUpdate {
                description: Some(Text {
                    content_type: "text/html".into(),
                    content: content.clone(),
                }),
                ..WorkItemUpdate::default()
            };
            // One write: it runs in this session (inside its transaction, if any).
            items.update(item, &update).await?;
        }

        let mut deleted = Vec::new();
        let mut warnings = Vec::new();
        for id in self.orphans(&content) {
            match items.delete_attachment(item, &id).await {
                Ok(()) => deleted.push(id),
                Err(e) => warnings.push(format!(
                    "the diagram attachment {id} is no longer shown but could not be deleted: \
                     {e} (later writes do not delete it either; delete it by hand)"
                )),
            }
        }
        Ok(Done {
            updated,
            attachments: self.attachments,
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
    let own_url = own_attachment_url(item, fetched);
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

/// The start of the `/wi-attachment/` URLs of the attachments of this work item.
fn own_attachment_url(item: &ItemRef, fetched: &WorkItem) -> Option<String> {
    match (&item.project, item.id.as_ref().or(fetched.id.as_ref())) {
        (Some(project), Some(id)) => Some(format!("/wi-attachment/{project}/{id}/")),
        _ => None,
    }
}

/// The file name to attach a new image under: its own, or, when that is the id of a listed
/// attachment (an exported image that was changed), the file name of that attachment.
fn upload_name(image: &LocalImage, listed: &[Attachment]) -> String {
    listed
        .iter()
        .find(|a| a.id.as_deref() == Some(image.file_name.as_str()))
        .map(|a| {
            a.file_name
                .clone()
                .unwrap_or_else(|| md::attachment_file_name(&image.file_name).to_owned())
        })
        .unwrap_or_else(|| image.file_name.clone())
}

/// The last segment of the path of an exported image.
fn file_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A file name for an attachment id: `A-Z a-z 0-9 . _ -` kept, anything else as `_`; never empty,
/// `.` or `..`, and none of `taken` (a number is put in front then).
fn safe_file_name<'t>(id: &str, taken: impl Iterator<Item = &'t str>) -> String {
    let mut name: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.chars().all(|c| c == '.') {
        name = "image".to_owned();
    }
    let taken: BTreeSet<&str> = taken.collect();
    if !taken.contains(name.as_str()) {
        return name;
    }
    (2..)
        .map(|n| format!("{n}_{name}"))
        .find(|candidate| !taken.contains(candidate.as_str()))
        .unwrap_or(name)
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
            images: vec![
                ImageAttachment {
                    url: "img/a.png".into(),
                    attachment: "4-a.png".into(),
                    uploaded: true,
                },
                ImageAttachment {
                    url: "P-1/5-b.png".into(),
                    attachment: "5-b.png".into(),
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
                "images": [
                    {"url": "img/a.png", "attachment": "4-a.png", "uploaded": true},
                    {"url": "P-1/5-b.png", "attachment": "5-b.png"},
                ],
                "deleted": ["3-mermaid-1111111111111111.svg"],
                "warnings": ["w"],
            })
        );
    }

    #[test]
    fn exported_images_get_safe_file_names() {
        let none = || std::iter::empty::<&str>();
        assert_eq!(safe_file_name("1-a.png", none()), "1-a.png");
        assert_eq!(safe_file_name("2-ä b/c\\d.png", none()), "2-__b_c_d.png");
        assert_eq!(safe_file_name("..", none()), "image");
        assert_eq!(safe_file_name("", none()), "image");
        assert_eq!(
            safe_file_name("1-a b.png", ["1-a_b.png", "2_1-a_b.png"].into_iter()),
            "3_1-a_b.png"
        );
        assert_eq!(file_of("P-1/1-a.png"), "1-a.png");
        assert_eq!(file_of("1-a.png"), "1-a.png");
    }

    #[test]
    fn a_changed_export_is_uploaded_under_the_attachments_file_name() {
        let listed = [
            attachment("3-shot.png", Some("shot.png")),
            attachment("4-x.png", None),
        ];
        let image = |file_name: &str| LocalImage {
            url: file_name.into(),
            file_name: file_name.into(),
            data: vec![1],
        };
        assert_eq!(upload_name(&image("3-shot.png"), &listed), "shot.png");
        assert_eq!(upload_name(&image("4-x.png"), &listed), "x.png");
        assert_eq!(
            upload_name(&image("2024-plan.png"), &listed),
            "2024-plan.png"
        );
    }
}
