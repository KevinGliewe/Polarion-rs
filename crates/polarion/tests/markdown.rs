//! Work item descriptions as Markdown: the write guard, the diagram attachments (rendered,
//! uploaded, reused, deleted), reading diagrams back, and the requests each call sends.

mod common;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use common::*;
use polarion::polarion_markdown::{
    self as md, DiagramSources, RawHtml, ToHtmlOptions, ToMarkdownOptions, WorkItemIds, mermaid,
};
use polarion::{
    Auth, DiagramAttachment, Error, ItemRef, MarkdownReadOptions, MarkdownUpdate,
    MarkdownWriteOptions, Polarion,
};
use serde_json::json;
use wiremock::matchers::{body_string_contains, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A diagram and its file name (pinned by the conversion's tests).
const SIMPLE: &str = "graph TD\n    A-->B";
const FILE: &str = "mermaid-f093190607556ad0.svg";
/// File names of other diagrams.
const OLD: &str = "mermaid-0123456789abcdef.svg";
const OTHER: &str = "mermaid-1111111111111111.svg";
const STRAY: &str = "mermaid-fedcba9876543210.svg";
const GONE: &str = "mermaid-00000000ffffffff.svg";
/// An SVG without an embedded source.
const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>"#;
/// The style of code blocks.
const PRE: &str = "margin-left: 40px; font-family: 'Courier New', Courier, monospace;";

fn sandbox(id: &str) -> ItemRef {
    ItemRef::new("Sandbox", id)
}

fn fence(source: &str) -> String {
    format!("```mermaid\n{source}\n```\n")
}

/// The image of a diagram attachment, as the conversion writes it.
fn diagram_img(id: &str) -> String {
    format!(
        r#"<img src="workitemimg:{id}" title="{}" style="max-width: 650px;"/>"#,
        md::attachment_file_name(id)
    )
}

/// A work item link, as the conversion writes it.
fn link(id: &str) -> String {
    format!(
        r#"<span class="polarion-rte-link" data-type="workItem" id="fake" data-item-id="{id}" data-option-id="long"></span>"#
    )
}

fn set_options() -> MarkdownWriteOptions {
    MarkdownWriteOptions::default()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#13;", "\r")
        .replace("&amp;", "&")
}

/// The answer to a read (`operation`: `getWorkItemByIdsWithFields`, ...) of the work item, with a
/// description (`(type, content)`) and attachments (`(id, file name)`).
fn answer(
    operation: &str,
    item: &ItemRef,
    description: Option<(&str, &str)>,
    attachments: &[(&str, &str)],
) -> String {
    let mut fields = String::new();
    if let Some(id) = &item.id {
        fields += &format!("<id>{id}</id>");
    }
    if let Some((kind, content)) = description {
        fields += &format!(
            "<description><type>{kind}</type><content>{}</content></description>",
            escape(content)
        );
    }
    if !attachments.is_empty() {
        fields += "<attachments>";
        for (id, file_name) in attachments {
            fields += &format!(
                "<Attachment><fileName>{}</fileName><id>{}</id><title>t</title></Attachment>",
                escape(file_name),
                escape(id)
            );
        }
        fields += "</attachments>";
    }
    format!(
        r#"<{operation}Response><{operation}Return uri="{}">{fields}</{operation}Return></{operation}Response>"#,
        escape(&item.uri)
    )
}

fn work_item(
    item: &ItemRef,
    description: Option<(&str, &str)>,
    attachments: &[(&str, &str)],
) -> String {
    answer("getWorkItemByIdsWithFields", item, description, attachments)
}

/// Answer the `n`-th read of the work item (counting from 1) with `body`.
async fn read(server: &MockServer, n: u8, body: String) {
    Mock::given(path(TRACKER))
        .and(body_string_contains("getWorkItemByIdsWithFields>"))
        .respond_with(xml(&body))
        .up_to_n_times(1)
        .with_priority(n)
        .expect(1)
        .mount(server)
        .await;
}

/// Answer `getAttachment` of one attachment.
async fn attachment(server: &MockServer, id: &str, response: ResponseTemplate) {
    Mock::given(path(TRACKER))
        .and(body_string_contains("getAttachment>"))
        .and(body_string_contains(format!(
            "attachmentId>{}<",
            escape(id)
        )))
        .respond_with(response)
        .expect(1)
        .mount(server)
        .await;
}

fn content(data: &str) -> ResponseTemplate {
    xml(&format!(
        "<getAttachmentResponse><getAttachmentReturn>{}</getAttachmentReturn></getAttachmentResponse>",
        BASE64.encode(data)
    ))
}

fn fault(message: &str) -> ResponseTemplate {
    ResponseTemplate::new(500).set_body_raw(
        envelope(
            "",
            &format!(
                "<soapenv:Fault><faultcode>soapenv:Server.userException</faultcode><faultstring>{message}</faultstring></soapenv:Fault>"
            ),
        ),
        "text/xml",
    )
}

/// After a fault the client asks whether the session is still valid: it is.
async fn session_is_valid(server: &MockServer, times: u64) {
    Mock::given(path(SESSION))
        .and(body_string_contains("hasSubject>"))
        .respond_with(xml(
            "<hasSubjectResponse><hasSubjectReturn>true</hasSubjectReturn></hasSubjectResponse>",
        ))
        .expect(times)
        .mount(server)
        .await;
}

/// The SOAP operations received, in order, logins left out.
async fn operations(server: &MockServer) -> Vec<String> {
    const BODY: &str = "<soapenv:Body><";
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| {
            let body = String::from_utf8_lossy(&r.body);
            let start = body.find(BODY).expect("a SOAP body") + BODY.len();
            let tag = &body[start..];
            let name = &tag[..tag.find(['>', ' ', '/']).unwrap()];
            name.rsplit(':').next().unwrap().to_owned()
        })
        .filter(|op| op != "logInWithToken")
        .collect()
}

/// The text of the first `<qname>` element of a request body, unescaped.
fn element(body: &str, qname: &str) -> String {
    let open = format!("<{qname}>");
    let start = body
        .find(&open)
        .unwrap_or_else(|| panic!("no {open} in {body}"))
        + open.len();
    let end = start + body[start..].find(&format!("</{qname}>")).unwrap();
    unescape(&body[start..end])
}

/// The fields a read asks for.
fn keys(body: &str) -> Vec<String> {
    body.match_indices("<trackeri:keys>")
        .map(|(i, open)| {
            let rest = &body[i + open.len()..];
            rest[..rest.find('<').unwrap()].to_owned()
        })
        .collect()
}

/// The description an `updateWorkItem` of `item` sends, `(type, content)`; nothing else may be
/// sent.
fn sent(item: &ItemRef, body: &str) -> (String, String) {
    let open = format!(r#"<trackeri:content uri="{}">"#, escape(&item.uri));
    let start = body.find(&open).unwrap_or_else(|| panic!("{body}")) + open.len();
    let fields = &body[start..body.find("</trackeri:content>").unwrap()];
    assert!(
        fields.starts_with("<trackert:description>") && fields.ends_with("</trackert:description>"),
        "only the description is written: {fields}"
    );
    (
        element(fields, "types:type"),
        element(fields, "types:content"),
    )
}

/// The description `updateWorkItem` sent, the request expected to be the only update.
async fn written(server: &MockServer, item: &ItemRef) -> String {
    let updates = bodies(server, "updateWorkItem").await;
    assert_eq!(updates.len(), 1);
    let (kind, content) = sent(item, &updates[0]);
    assert_eq!(kind, "text/html");
    content
}

fn refused(result: polarion::Result<MarkdownUpdate>, project: Option<&str>) {
    match result {
        Err(Error::WriteNotAllowed {
            operation,
            project: p,
            ..
        }) => {
            assert_eq!(operation, "updateWorkItem");
            assert_eq!(p.as_deref(), project);
        }
        other => panic!("expected WriteNotAllowed, got {other:?}"),
    }
}

fn diagram(file_name: &str, attachment: Option<&str>, uploaded: bool) -> DiagramAttachment {
    DiagramAttachment {
        file_name: file_name.to_owned(),
        attachment: attachment.map(str::to_owned),
        uploaded,
    }
}

#[tokio::test]
async fn writes_are_refused_before_anything_is_sent() {
    let server = session_server().await;
    let (reader, writer) = (reader(&server), writer(&server));
    let options = set_options();
    let markdown = format!("Text\n\n{}", fence(SIMPLE));
    for (polarion, item, project) in [
        (&reader, sandbox("SB-1"), Some("Sandbox")),
        (&writer, ItemRef::new("Prod", "PR-1"), Some("Prod")),
        (&writer, ItemRef::from_uri("not a polarion uri"), None),
    ] {
        let items = polarion.items();
        refused(
            items
                .set_description_markdown(&item, &markdown, &options)
                .await,
            project,
        );
        refused(
            items
                .append_description_markdown(&item, &markdown, &options)
                .await,
            project,
        );
        // The guard comes before the shortcut for blank Markdown.
        refused(
            items
                .append_description_markdown(&item, " \n", &options)
                .await,
            project,
        );
    }
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "nothing may be sent, not even a login"
    );
}

#[tokio::test]
async fn a_new_diagram_is_rendered_uploaded_and_shown() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    let id = format!("1-{FILE}");
    read(&server, 1, work_item(&item, None, &[])).await;
    read(&server, 2, work_item(&item, None, &[(&id, FILE)])).await;
    expect(
        &server,
        TRACKER,
        &["createAttachment>"],
        "<createAttachmentResponse/>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["updateWorkItem>"],
        "<updateWorkItemResponse/>",
    )
    .await;

    let markdown = format!("Flow:\n\n{}", fence(SIMPLE));
    let update = writer(&server)
        .items()
        .set_description_markdown(&item, &markdown, &set_options())
        .await
        .unwrap();

    assert_eq!(
        update,
        MarkdownUpdate {
            updated: true,
            diagrams: vec![diagram(FILE, Some(&id), true)],
            ..MarkdownUpdate::default()
        }
    );
    assert_eq!(
        serde_json::to_value(&update).unwrap(),
        json!({"updated": true, "diagrams": [{"fileName": FILE, "attachment": id, "uploaded": true}]})
    );
    assert_eq!(
        operations(&server).await,
        [
            "getWorkItemByIdsWithFields",
            "createAttachment",
            "getWorkItemByIdsWithFields",
            "updateWorkItem"
        ]
    );
    // One upload, then one update: single writes, both on the client's session.
    let sessions: Vec<String> = log(&server)
        .await
        .into_iter()
        .filter(|(_, o)| o != "logInWithToken")
        .map(|(s, _)| s)
        .collect();
    assert_eq!(sessions, ["-1", "-1", "-1", "-1"]);

    let reads = bodies(&server, "getWorkItemByIdsWithFields").await;
    assert_eq!(element(&reads[0], "trackeri:projectId"), "Sandbox");
    assert_eq!(element(&reads[0], "trackeri:workitemId"), "SB-1");
    assert_eq!(keys(&reads[0]), ["id", "description", "attachments"]);
    // The attachments again, for the id of the new one.
    assert_eq!(keys(&reads[1]), ["attachments"]);

    let created = bodies(&server, "createAttachment").await;
    assert_eq!(element(&created[0], "trackeri:workitemURI"), item.uri);
    assert_eq!(element(&created[0], "trackeri:fileName"), FILE);
    assert_eq!(element(&created[0], "trackeri:title"), "Mermaid diagram");
    let data = BASE64
        .decode(element(&created[0], "trackeri:data"))
        .unwrap();
    let svg = String::from_utf8(data).unwrap();
    assert!(svg.starts_with("<svg ") && svg.ends_with("</svg>"), "{svg}");
    assert_eq!(mermaid::source_of(&svg).as_deref(), Some(SIMPLE));

    assert_eq!(
        written(&server, &item).await,
        format!("Flow:<br/><br/>{}", diagram_img(&id))
    );
}

/// Several uploads, then the update and a delete: two transactions, each in a session of its
/// own. Polarion lists an attachment uploaded in a transaction only once it is committed, so the
/// ids of the new attachments are read between them, on the client's session.
#[tokio::test]
async fn new_diagrams_are_uploaded_before_the_transaction_that_shows_them() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    let second = "graph TD
    C-->D";
    let second_file = mermaid::diagram(second).file_name;
    let (old, first_id, second_id) = (
        format!("2-{OLD}"),
        format!("3-{FILE}"),
        format!("4-{second_file}"),
    );
    read(
        &server,
        1,
        work_item(
            &item,
            Some(("text/html", &diagram_img(&old))),
            &[(&old, OLD)],
        ),
    )
    .await;
    read(
        &server,
        2,
        work_item(
            &item,
            None,
            &[(&old, OLD), (&first_id, FILE), (&second_id, &second_file)],
        ),
    )
    .await;
    Mock::given(path(TRACKER))
        .and(body_string_contains("createAttachment>"))
        .respond_with(xml("<createAttachmentResponse/>"))
        .expect(2)
        .mount(&server)
        .await;
    expect(
        &server,
        TRACKER,
        &["updateWorkItem>"],
        "<updateWorkItemResponse/>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["deleteAttachment>", &format!("id>{old}<")],
        "<deleteAttachmentResponse/>",
    )
    .await;

    let markdown = format!(
        "{}
{}",
        fence(SIMPLE),
        fence(second)
    );
    let update = writer(&server)
        .items()
        .set_description_markdown(&item, &markdown, &set_options())
        .await
        .unwrap();

    assert!(update.updated);
    assert_eq!(
        update.diagrams,
        [
            diagram(FILE, Some(&first_id), true),
            diagram(&second_file, Some(&second_id), true)
        ]
    );
    assert_eq!(update.deleted, [old]);
    let requests: Vec<(String, String)> = log(&server)
        .await
        .into_iter()
        .filter(|(_, o)| o != "logInWithToken")
        .collect();
    assert_eq!(
        requests,
        [
            ("-1", "getWorkItemByIdsWithFields"),
            ("-2", "beginTransaction"),
            ("-2", "createAttachment"),
            ("-2", "createAttachment"),
            ("-2", "endTransaction"),
            ("-2", "endSession"),
            ("-1", "getWorkItemByIdsWithFields"),
            ("-3", "beginTransaction"),
            ("-3", "updateWorkItem"),
            ("-3", "deleteAttachment"),
            ("-3", "endTransaction"),
            ("-3", "endSession"),
        ]
        .map(|(s, o)| (s.to_owned(), o.to_owned()))
    );
    assert_eq!(
        written(&server, &item).await,
        format!(
            "{}<br/><br/>{}",
            diagram_img(&first_id),
            diagram_img(&second_id)
        )
    );
}

#[tokio::test]
async fn an_upload_missing_from_the_attachments_fails() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    read(&server, 1, work_item(&item, None, &[])).await;
    read(
        &server,
        2,
        work_item(&item, None, &[("1-spec.png", "spec.png")]),
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["createAttachment>"],
        "<createAttachmentResponse/>",
    )
    .await;

    let err = writer(&server)
        .items()
        .set_description_markdown(&item, &fence(SIMPLE), &set_options())
        .await
        .unwrap_err();

    assert!(
        matches!(&err, Error::Invalid(m) if m.contains(FILE) && m.contains("Sandbox/SB-1")),
        "{err:?}"
    );
    assert_eq!(
        operations(&server).await,
        [
            "getWorkItemByIdsWithFields",
            "createAttachment",
            "getWorkItemByIdsWithFields"
        ]
    );
}

#[tokio::test]
async fn highlighting_can_be_turned_off() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    Mock::given(path(TRACKER))
        .and(body_string_contains("getWorkItemByIdsWithFields>"))
        .respond_with(xml(&work_item(&item, None, &[])))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(path(TRACKER))
        .and(body_string_contains("updateWorkItem>"))
        .respond_with(xml("<updateWorkItemResponse/>"))
        .expect(2)
        .mount(&server)
        .await;

    let markdown = "```rust\nfn main() {}\n```\n";
    let polarion = writer(&server);
    for highlight in [true, false] {
        let options = MarkdownWriteOptions {
            highlight,
            ..set_options()
        };
        polarion
            .items()
            .set_description_markdown(&item, markdown, &options)
            .await
            .unwrap();
    }

    let html = |highlight| {
        let options = ToHtmlOptions {
            work_items: WorkItemIds::Prefixes(vec!["SB".into()]),
            highlight,
        };
        md::to_html(markdown, &options)
    };
    let updates = bodies(&server, "updateWorkItem").await;
    assert_eq!(sent(&item, &updates[0]).1, html(true));
    assert_eq!(
        sent(&item, &updates[1]).1,
        format!(r#"<pre class="language-rust" style="{PRE}">fn main() {{}}</pre>"#)
    );
    #[cfg(feature = "highlight")]
    assert_ne!(html(true), html(false));
}

#[tokio::test]
async fn an_existing_diagram_attachment_is_reused() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    let id = format!("7-{FILE}");
    read(
        &server,
        1,
        work_item(&item, Some(("text/html", "<b>old</b>")), &[(&id, FILE)]),
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["updateWorkItem>"],
        "<updateWorkItemResponse/>",
    )
    .await;

    let update = writer(&server)
        .items()
        .set_description_markdown(&item, &fence(SIMPLE), &set_options())
        .await
        .unwrap();

    assert_eq!(
        update,
        MarkdownUpdate {
            updated: true,
            diagrams: vec![diagram(FILE, Some(&id), false)],
            ..MarkdownUpdate::default()
        }
    );
    assert_eq!(
        serde_json::to_value(&update).unwrap(),
        json!({"updated": true, "diagrams": [{"fileName": FILE, "attachment": id}]})
    );
    assert_eq!(
        operations(&server).await,
        ["getWorkItemByIdsWithFields", "updateWorkItem"]
    );
    assert_eq!(written(&server, &item).await, diagram_img(&id));
}

/// A description showing the current diagram, two old ones, an image and a deleted attachment;
/// the work item also has a diagram attachment no description shows.
async fn description_with_old_diagrams(server: &MockServer, item: &ItemRef) {
    let (current, old, other) = (
        format!("1-{FILE}"),
        format!("2-{OLD}"),
        format!("4-{OTHER}"),
    );
    let html = [
        diagram_img(&current),
        diagram_img(&old),
        diagram_img(&other),
        r#"<img src="workitemimg:3-spec.png"/>"#.to_owned(),
        diagram_img(&format!("9-{GONE}")),
    ]
    .join("<br/><br/>");
    let stray = format!("5-{STRAY}");
    read(
        server,
        1,
        work_item(
            item,
            Some(("text/html", &html)),
            &[
                (&current, FILE),
                (&old, OLD),
                ("3-spec.png", "spec.png"),
                (&other, OTHER),
                (&stray, STRAY),
            ],
        ),
    )
    .await;
    expect(
        server,
        TRACKER,
        &["updateWorkItem>"],
        "<updateWorkItemResponse/>",
    )
    .await;
}

#[tokio::test]
async fn diagram_attachments_only_the_old_description_showed_are_deleted() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    description_with_old_diagrams(&server, &item).await;
    let (old, other) = (format!("2-{OLD}"), format!("4-{OTHER}"));
    expect(
        &server,
        TRACKER,
        &["deleteAttachment>", &format!("id>{old}<")],
        "<deleteAttachmentResponse/>",
    )
    .await;
    Mock::given(path(TRACKER))
        .and(body_string_contains("deleteAttachment>"))
        .and(body_string_contains(format!("id>{other}<")))
        .respond_with(fault("locked"))
        .expect(1)
        .mount(&server)
        .await;
    session_is_valid(&server, 1).await;

    let update = writer(&server)
        .items()
        .set_description_markdown(&item, &fence(SIMPLE), &set_options())
        .await
        .unwrap();

    assert!(update.updated);
    assert_eq!(
        update.diagrams,
        [diagram(FILE, Some(&format!("1-{FILE}")), false)]
    );
    // Not the image, nor the diagram attachment no description showed, nor the one that is gone.
    assert_eq!(update.deleted, std::slice::from_ref(&old));
    assert_eq!(update.warnings.len(), 1);
    assert!(
        update.warnings[0].starts_with(&format!(
            "the diagram attachment {other} is no longer shown but could not be deleted: "
        )) && update.warnings[0].contains("locked")
            && update.warnings[0]
                .ends_with("(later writes do not delete it either; delete it by hand)"),
        "{:?}",
        update.warnings
    );
    assert_eq!(
        operations(&server).await,
        [
            "getWorkItemByIdsWithFields",
            "beginTransaction",
            "updateWorkItem",
            "deleteAttachment",
            "deleteAttachment",
            "hasSubject",
            "endTransaction",
            "endSession"
        ]
    );
    let deletes = bodies(&server, "deleteAttachment").await;
    for (body, id) in deletes.iter().zip([&old, &other]) {
        assert_eq!(element(body, "trackeri:workitemURI"), item.uri);
        assert_eq!(&element(body, "trackeri:id"), id);
    }
    assert_eq!(
        written(&server, &item).await,
        diagram_img(&format!("1-{FILE}"))
    );
}

#[tokio::test]
async fn keep_orphans_deletes_nothing() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    description_with_old_diagrams(&server, &item).await;

    let options = MarkdownWriteOptions {
        keep_orphans: true,
        ..set_options()
    };
    let update = writer(&server)
        .items()
        .set_description_markdown(&item, &fence(SIMPLE), &options)
        .await
        .unwrap();

    assert!(update.updated && update.deleted.is_empty() && update.warnings.is_empty());
    assert_eq!(
        operations(&server).await,
        ["getWorkItemByIdsWithFields", "updateWorkItem"]
    );
}

#[tokio::test]
async fn of_several_attachments_of_a_diagram_the_shown_one_is_reused() {
    let (first, newest) = (format!("1-{FILE}"), format!("4-{FILE}"));
    for (old, shown) in [
        // The attachment the old description shows, although there is a newer one.
        (format!("Old<br/><br/>{}", diagram_img(&first)), &first),
        // None shown: the newest.
        ("<b>old</b>".to_owned(), &newest),
    ] {
        let server = session_server().await;
        let item = sandbox("SB-1");
        read(
            &server,
            1,
            work_item(
                &item,
                Some(("text/html", &old)),
                &[(&newest, FILE), (&first, FILE)],
            ),
        )
        .await;
        expect(
            &server,
            TRACKER,
            &["updateWorkItem>"],
            "<updateWorkItemResponse/>",
        )
        .await;

        let markdown = format!("New\n\n{}", fence(SIMPLE));
        let update = writer(&server)
            .items()
            .set_description_markdown(&item, &markdown, &set_options())
            .await
            .unwrap();

        assert_eq!(update.diagrams, [diagram(FILE, Some(shown), false)]);
        assert!(update.deleted.is_empty(), "{update:?}");
        assert_eq!(
            operations(&server).await,
            ["getWorkItemByIdsWithFields", "updateWorkItem"]
        );
        assert_eq!(
            written(&server, &item).await,
            format!("New<br/><br/>{}", diagram_img(shown))
        );
    }
}

#[tokio::test]
async fn appending_keeps_the_description() {
    let old_diagram = format!("2-{OLD}");
    let html = format!("{}<br/><br/><b>old</b>", diagram_img(&old_diagram));
    for (description, expected) in [
        (
            ("text/html", html.as_str()),
            format!(r#"{html}<br/><br/><span style="font-weight: bold;">new</span>"#),
        ),
        // A text/plain description becomes HTML.
        (
            ("text/plain", "line 1\nline 2 & more"),
            r#"line 1<br/>line 2 &amp; more<br/><br/><span style="font-weight: bold;">new</span>"#
                .to_owned(),
        ),
        // The line breaks at its ends are no extra empty lines.
        (
            ("text/plain", "\nline 1\nline 2\n\n"),
            r#"line 1<br/>line 2<br/><br/><span style="font-weight: bold;">new</span>"#.to_owned(),
        ),
    ] {
        let server = session_server().await;
        let item = sandbox("SB-1");
        read(
            &server,
            1,
            work_item(&item, Some(description), &[(&old_diagram, OLD)]),
        )
        .await;
        expect(
            &server,
            TRACKER,
            &["updateWorkItem>"],
            "<updateWorkItemResponse/>",
        )
        .await;

        let update = writer(&server)
            .items()
            .append_description_markdown(&item, "**new**", &set_options())
            .await
            .unwrap();

        assert_eq!(
            update,
            MarkdownUpdate {
                updated: true,
                ..MarkdownUpdate::default()
            }
        );
        // Nothing is deleted.
        assert_eq!(
            operations(&server).await,
            ["getWorkItemByIdsWithFields", "updateWorkItem"]
        );
        assert_eq!(written(&server, &item).await, expected);
    }
}

#[tokio::test]
async fn appending_blank_markdown_sends_nothing() {
    let server = session_server().await;
    let update = writer(&server)
        .items()
        .append_description_markdown(&sandbox("SB-1"), " \n\t\n", &set_options())
        .await
        .unwrap();
    assert_eq!(update, MarkdownUpdate::default());
    assert_eq!(
        serde_json::to_value(&update).unwrap(),
        json!({"updated": false})
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_diagram_that_cannot_be_rendered_fails_before_anything_is_written() {
    let bad = "this is not a diagram";
    let markdown = format!("{}\n{}", fence(SIMPLE), fence(bad));
    let item = sandbox("SB-1");

    let refusing = session_server().await;
    read(
        &refusing,
        1,
        work_item(&item, Some(("text/html", "<b>old</b>")), &[]),
    )
    .await;
    // An empty block and a repeated one before it: the message names the failing diagram by its
    // first line, not by a number that would count neither.
    let with_others = format!("```mermaid\n```\n\n{}\n{markdown}", fence(SIMPLE));
    let err = writer(&refusing)
        .items()
        .set_description_markdown(&item, &with_others, &set_options())
        .await
        .unwrap_err();
    match &err {
        Error::Invalid(message) => {
            assert!(
                message.starts_with(&format!("mermaid diagram ({bad}) could not be rendered: "))
                    && message.contains("unknown or missing Mermaid diagram header")
                    && message.ends_with("; allow-unrendered writes it as a code block"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    // The first diagram rendered, but is not uploaded either.
    assert_eq!(operations(&refusing).await, ["getWorkItemByIdsWithFields"]);

    // Allowed: the diagram is a code block, with a warning.
    let server = session_server().await;
    let id = format!("1-{FILE}");
    read(&server, 1, work_item(&item, None, &[])).await;
    read(&server, 2, work_item(&item, None, &[(&id, FILE)])).await;
    expect(
        &server,
        TRACKER,
        &["createAttachment>", &format!("fileName>{FILE}<")],
        "<createAttachmentResponse/>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["updateWorkItem>"],
        "<updateWorkItemResponse/>",
    )
    .await;
    let options = MarkdownWriteOptions {
        allow_unrendered: true,
        ..set_options()
    };
    let update = writer(&server)
        .items()
        .set_description_markdown(&item, &markdown, &options)
        .await
        .unwrap();

    let bad_file = mermaid::diagram(bad).file_name;
    assert!(update.updated);
    assert_eq!(
        update.diagrams,
        [
            diagram(FILE, Some(&id), true),
            diagram(&bad_file, None, false)
        ]
    );
    assert_eq!(update.warnings.len(), 1);
    assert!(
        update.warnings[0].starts_with(&format!(
            "mermaid diagram ({bad}) could not be rendered and is written as a code block: "
        )),
        "{:?}",
        update.warnings
    );
    assert_eq!(
        operations(&server).await,
        [
            "getWorkItemByIdsWithFields",
            "createAttachment",
            "getWorkItemByIdsWithFields",
            "updateWorkItem"
        ]
    );
    assert_eq!(
        written(&server, &item).await,
        format!(
            r#"{}<br/><br/><pre class="language-mermaid" style="{PRE}">{bad}</pre>"#,
            diagram_img(&id)
        )
    );
}

#[tokio::test]
async fn an_unchanged_description_is_not_written() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    let id = format!("1-{FILE}");
    let html = format!(
        r#"<span style="font-weight: bold;">same</span><br/><br/>{}"#,
        diagram_img(&id)
    );
    read(
        &server,
        1,
        work_item(&item, Some(("text/html", &html)), &[(&id, FILE)]),
    )
    .await;

    let markdown = format!("**same**\n\n{}", fence(SIMPLE));
    let update = writer(&server)
        .items()
        .set_description_markdown(&item, &markdown, &set_options())
        .await
        .unwrap();

    assert_eq!(
        update,
        MarkdownUpdate {
            updated: false,
            diagrams: vec![diagram(FILE, Some(&id), false)],
            ..MarkdownUpdate::default()
        }
    );
    assert_eq!(
        serde_json::to_value(&update).unwrap(),
        json!({"updated": false, "diagrams": [{"fileName": FILE, "attachment": id}]})
    );
    assert_eq!(operations(&server).await, ["getWorkItemByIdsWithFields"]);
}

#[tokio::test]
async fn ids_with_the_work_items_own_prefix_become_links() {
    let server = session_server().await;
    let polarion = Polarion::builder(&base_url(&server), Auth::Token("t".into()))
        .allow_writes(["MyProject"])
        .build()
        .unwrap();
    let item = ItemRef::new("MyProject", "MP-42");
    // Without the id in the reference, the id the server returns counts.
    let by_uri = ItemRef {
        id: None,
        ..item.clone()
    };
    Mock::given(path(TRACKER))
        .and(body_string_contains("getWorkItemByIdsWithFields>"))
        .respond_with(xml(&work_item(&item, None, &[])))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(path(TRACKER))
        .and(body_string_contains("getWorkItemByUriWithFields>"))
        .respond_with(xml(&answer("getWorkItemByUriWithFields", &item, None, &[])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path(TRACKER))
        .and(body_string_contains("updateWorkItem>"))
        .respond_with(xml("<updateWorkItemResponse/>"))
        .expect(3)
        .mount(&server)
        .await;

    let markdown = "See MP-1 and UTF-8.";
    let any = MarkdownWriteOptions {
        work_items: Some(WorkItemIds::Any),
        ..set_options()
    };
    let items = polarion.items();
    for (target, options) in [
        (&item, set_options()),
        (&by_uri, set_options()),
        (&item, any),
    ] {
        let update = items
            .set_description_markdown(target, markdown, &options)
            .await
            .unwrap();
        assert!(update.updated);
    }

    let updates = bodies(&server, "updateWorkItem").await;
    let own = format!("See {} and UTF-8.", link("MP-1"));
    assert_eq!(sent(&item, &updates[0]).1, own);
    assert_eq!(sent(&item, &updates[1]).1, own);
    assert_eq!(
        sent(&item, &updates[2]).1,
        format!("See {} and {}.", link("MP-1"), link("UTF-8"))
    );
    // Text only: one write each, on the client's session.
    assert!(
        log(&server)
            .await
            .iter()
            .all(|(s, o)| s == "-1" || (s.is_empty() && o == "logInWithToken")),
        "{:?}",
        log(&server).await
    );
}

#[tokio::test]
async fn diagrams_are_read_back_from_their_svg() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    let (with_source, failing, without_source) = (
        format!("1-{FILE}"),
        format!("2-{OLD}"),
        format!("4-{OTHER}"),
    );
    let html = [
        diagram_img(&with_source),
        diagram_img(&failing),
        diagram_img(&without_source),
        // Shown, but not an attachment (any more).
        diagram_img(&format!("9-{GONE}")),
        r#"<img src="workitemimg:3-spec.png"/>"#.to_owned(),
        format!("See {} and {}.", link("SB-2"), link("OT-9")),
    ]
    .join("<br/><br/>");
    let stray = format!("5-{STRAY}");
    read(
        &server,
        1,
        work_item(
            &item,
            Some(("text/html", &html)),
            &[
                (&with_source, FILE),
                (&failing, OLD),
                ("3-spec.png", "spec.png"),
                (&without_source, OTHER),
                (&stray, STRAY),
            ],
        ),
    )
    .await;
    attachment(
        &server,
        &with_source,
        content(&mermaid::embed_source(SVG, SIMPLE)),
    )
    .await;
    attachment(&server, &failing, fault("unknown operation")).await;
    attachment(&server, &without_source, content(SVG)).await;
    session_is_valid(&server, 1).await;

    let markdown = reader(&server)
        .items()
        .description_markdown(&item, &MarkdownReadOptions::default())
        .await
        .unwrap();

    assert_eq!(
        operations(&server).await,
        [
            "getWorkItemByIdsWithFields",
            "getAttachment",
            "getAttachment",
            "hasSubject",
            "getAttachment"
        ]
    );
    let reads = bodies(&server, "getWorkItemByIdsWithFields").await;
    assert_eq!(keys(&reads[0]), ["id", "description", "attachments"]);
    for body in bodies(&server, "getAttachment").await {
        assert_eq!(element(&body, "trackeri:workitemURI"), item.uri);
    }

    // Links to ids of the work item's own prefix are read as plain ids, the others stay HTML.
    let options = ToMarkdownOptions {
        raw_html: RawHtml::Keep,
        work_items: WorkItemIds::Prefixes(vec!["SB".into()]),
    };
    let sources = DiagramSources::from([(with_source.clone(), SIMPLE.to_owned())]);
    assert_eq!(
        markdown,
        md::to_markdown_with_diagrams(&html, &options, &sources)
    );
    let any = ToMarkdownOptions {
        work_items: WorkItemIds::Any,
        ..options
    };
    assert_ne!(
        markdown,
        md::to_markdown_with_diagrams(&html, &any, &sources)
    );
    assert!(
        markdown.contains(&format!("```mermaid\n{SIMPLE}\n```")),
        "{markdown}"
    );
    for kept in [&failing, &without_source] {
        assert!(
            markdown.contains(&format!("workitemimg:{kept}")),
            "{markdown}"
        );
    }
}

#[tokio::test]
async fn read_options_are_passed_on() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    let html = format!(
        r#"<span style="color: #FF0000;">Red</span> {}"#,
        link("SB-2")
    );
    read(
        &server,
        1,
        work_item(&item, Some(("text/html", &html)), &[]),
    )
    .await;

    let options = MarkdownReadOptions {
        raw_html: RawHtml::Drop,
        work_items: Some(WorkItemIds::Off),
    };
    let markdown = reader(&server)
        .items()
        .description_markdown(&item, &options)
        .await
        .unwrap();

    let convert = |raw_html, work_items| {
        md::to_markdown_with_diagrams(
            &html,
            &ToMarkdownOptions {
                raw_html,
                work_items,
            },
            &DiagramSources::new(),
        )
    };
    assert_eq!(markdown, convert(RawHtml::Drop, WorkItemIds::Off));
    // The options matter for this description.
    let own = WorkItemIds::Prefixes(vec!["SB".into()]);
    assert_ne!(markdown, convert(RawHtml::Keep, WorkItemIds::Off));
    assert_ne!(markdown, convert(RawHtml::Drop, own));
    assert_eq!(operations(&server).await, ["getWorkItemByIdsWithFields"]);
}

#[tokio::test]
async fn a_plain_text_description_is_read_as_text() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    let plain = "Plain & simple\n<b>not bold</b>";
    read(
        &server,
        1,
        work_item(&item, Some(("text/plain", plain)), &[]),
    )
    .await;

    let markdown = reader(&server)
        .items()
        .description_markdown(&item, &MarkdownReadOptions::default())
        .await
        .unwrap();

    let options = ToMarkdownOptions {
        raw_html: RawHtml::Keep,
        work_items: WorkItemIds::Prefixes(vec!["SB".into()]),
    };
    assert_eq!(
        markdown,
        md::to_markdown_with_diagrams(&md::text_to_html(plain), &options, &DiagramSources::new())
    );
    assert_eq!(operations(&server).await, ["getWorkItemByIdsWithFields"]);
}

#[tokio::test]
async fn other_errors_reading_a_diagram_fail_the_read() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    let id = format!("1-{FILE}");
    read(
        &server,
        1,
        work_item(
            &item,
            Some(("text/html", &diagram_img(&id))),
            &[(&id, FILE)],
        ),
    )
    .await;
    attachment(
        &server,
        &id,
        ResponseTemplate::new(503).set_body_string("unavailable"),
    )
    .await;
    session_is_valid(&server, 1).await;

    let err = reader(&server)
        .items()
        .description_markdown(&item, &MarkdownReadOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Soap(polarion::soap::Error::Status { .. })),
        "{err:?}"
    );
}

#[tokio::test]
async fn line_breaks_at_the_end_of_a_plain_text_description_are_not_read() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    read(
        &server,
        1,
        work_item(&item, Some(("text/plain", "line 1\nline 2\n\n")), &[]),
    )
    .await;

    let markdown = reader(&server)
        .items()
        .description_markdown(&item, &MarkdownReadOptions::default())
        .await
        .unwrap();

    let options = ToMarkdownOptions {
        raw_html: RawHtml::Keep,
        work_items: WorkItemIds::Prefixes(vec!["SB".into()]),
    };
    assert_eq!(
        markdown,
        md::to_markdown_with_diagrams(
            &md::text_to_html("line 1\nline 2"),
            &options,
            &DiagramSources::new()
        )
    );
    assert!(!markdown.contains("<br"), "{markdown:?}");
}

#[tokio::test]
async fn a_server_without_get_attachment_is_asked_once() {
    let server = session_server().await;
    let item = sandbox("SB-1");
    let ids = [
        format!("1-{FILE}"),
        format!("2-{OLD}"),
        format!("4-{OTHER}"),
    ];
    let html = ids
        .iter()
        .map(|id| diagram_img(id))
        .collect::<Vec<_>>()
        .join("<br/><br/>");
    let listed: Vec<(&str, &str)> = ids
        .iter()
        .map(|id| (id.as_str(), md::attachment_file_name(id)))
        .collect();
    read(
        &server,
        1,
        work_item(&item, Some(("text/html", &html)), &listed),
    )
    .await;
    Mock::given(path(TRACKER))
        .and(body_string_contains("getAttachment>"))
        .respond_with(fault("No such operation 'getAttachment'"))
        .expect(1)
        .mount(&server)
        .await;
    session_is_valid(&server, 1).await;

    let markdown = reader(&server)
        .items()
        .description_markdown(&item, &MarkdownReadOptions::default())
        .await
        .unwrap();

    assert_eq!(
        operations(&server).await,
        ["getWorkItemByIdsWithFields", "getAttachment", "hasSubject"]
    );
    for id in &ids {
        assert!(
            markdown.contains(&format!("workitemimg:{id}")),
            "{markdown}"
        );
    }
}

/// The same diagram uploaded to two work items first has the same attachment id on both.
#[tokio::test]
async fn another_work_items_diagram_is_not_this_ones() {
    let item = sandbox("SB-1");
    let (theirs, own_old, own_newest) =
        (format!("1-{FILE}"), format!("2-{OLD}"), format!("3-{FILE}"));
    let html = format!(
        r#"<img src="/polarion/wi-attachment/Sandbox/SB-2/{theirs}"/><br/><br/><img src="https://polarion.example.com/polarion/wi-attachment/Sandbox/SB-1/{own_old}?revision=2"/>"#
    );
    let listed = [
        (theirs.as_str(), FILE),
        (own_old.as_str(), OLD),
        (own_newest.as_str(), FILE),
    ];

    // Writing: SB-1's `1-…` is neither preferred nor deleted; its own old diagram is deleted.
    let server = session_server().await;
    read(
        &server,
        1,
        work_item(&item, Some(("text/html", &html)), &listed),
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["updateWorkItem>"],
        "<updateWorkItemResponse/>",
    )
    .await;
    expect(
        &server,
        TRACKER,
        &["deleteAttachment>", &format!("id>{own_old}<")],
        "<deleteAttachmentResponse/>",
    )
    .await;
    let update = writer(&server)
        .items()
        .set_description_markdown(&item, &fence(SIMPLE), &set_options())
        .await
        .unwrap();
    assert_eq!(update.diagrams, [diagram(FILE, Some(&own_newest), false)]);
    assert_eq!(update.deleted, std::slice::from_ref(&own_old));
    assert_eq!(
        operations(&server).await,
        [
            "getWorkItemByIdsWithFields",
            "beginTransaction",
            "updateWorkItem",
            "deleteAttachment",
            "endTransaction",
            "endSession"
        ]
    );

    // Reading: only the work item's own diagram is fetched.
    let server = common::session_server().await;
    read(
        &server,
        1,
        work_item(&item, Some(("text/html", &html)), &listed),
    )
    .await;
    attachment(
        &server,
        &own_old,
        content(&mermaid::embed_source(SVG, "graph LR\n    X-->Y")),
    )
    .await;
    let markdown = reader(&server)
        .items()
        .description_markdown(&item, &MarkdownReadOptions::default())
        .await
        .unwrap();
    assert_eq!(
        operations(&server).await,
        ["getWorkItemByIdsWithFields", "getAttachment"]
    );
    assert!(markdown.contains("```mermaid\ngraph LR"), "{markdown}");
    assert!(markdown.contains(&format!("SB-2/{theirs}")), "{markdown}");
}

/// The commands run these futures on multi-threaded runtimes.
#[test]
fn the_futures_are_send() {
    fn send<T: Send>(_: &T) {}
    let polarion = Polarion::new("http://localhost/polarion", Auth::Token("t".into())).unwrap();
    let items = polarion.items();
    let item = sandbox("SB-1");
    let options = set_options();
    send(&items.description_markdown(&item, &MarkdownReadOptions::default()));
    send(&items.set_description_markdown(&item, "x", &options));
    send(&items.append_description_markdown(&item, "x", &options));
}
