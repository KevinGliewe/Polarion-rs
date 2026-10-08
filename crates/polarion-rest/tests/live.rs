//! Live tests against Siemens' public Polarion test drive server.
//!
//! The server URL is hard-coded on purpose: these tests create and delete
//! data and must never run against a production instance.
//!
//! ```sh
//! export POLARION_TESTDRIVE_TOKEN=<personal access token from testdrive>
//! export POLARION_TESTDRIVE_PROJECT=<your project in the testdrive Sandbox>
//! cargo test -p polarion-rest --test live -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Optional: `POLARION_TESTDRIVE_WI_TYPE` (default: first type the project
//! offers), `POLARION_TESTDRIVE_LINK_ROLE` (default: `relates_to`).

use std::error::Error as StdError;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use polarion_rest::models::{
    LinkedWorkItem, LinkedWorkItemAttributes, ProjectAttributes, UserAttributes, WorkItem,
    WorkItemAttachment, WorkItemAttachmentAttributes, WorkItemAttributes, WorkItemComment,
    WorkItemCommentAttributes,
};
use polarion_rest::{Client, FilePart, Query, ResourceKind, ResourceRef, TextContent};

const TESTDRIVE: &str = "https://testdrive.polarion.com/polarion";

type TestResult<T = ()> = Result<T, Box<dyn StdError>>;

/// Like `assert!`, but returns an error so cleanup still runs.
macro_rules! check {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err(format!($($msg)+).into());
        }
    };
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn client(read_only: bool) -> Client {
    let token = env("POLARION_TESTDRIVE_TOKEN").expect(
        "set POLARION_TESTDRIVE_TOKEN to a personal access token created on testdrive.polarion.com",
    );
    Client::builder(TESTDRIVE, &token)
        .read_only(read_only)
        .build()
        .expect("client")
}

/// `PROJ/WI-1` → `WI-1`.
fn short_id(full: &str) -> &str {
    full.rsplit('/').next().unwrap_or(full)
}

#[tokio::test]
#[ignore = "live: needs POLARION_TESTDRIVE_TOKEN"]
async fn read_tour() {
    let c = client(true);

    let me = c.get_current_user(&Query::new()).await.expect("GET /user");
    eprintln!("authenticated as {:?}", me.data.id);

    let meta = c.get_metadata(&Query::new()).await.expect("GET /metadata");
    eprintln!("server metadata: {:?}", meta.data.attributes);

    let projects = c
        .get_projects(
            &Query::new()
                .page_size(10)
                .all_fields(ProjectAttributes::TYPE),
        )
        .await
        .expect("GET /projects");
    eprintln!("{:?} projects visible", projects.total_count());
    let project = env("POLARION_TESTDRIVE_PROJECT")
        .or_else(|| projects.data.first().and_then(|p| p.id.clone()))
        .expect("no project visible");

    c.get_project(&project, &Query::new().all_fields(ProjectAttributes::TYPE))
        .await
        .expect("GET project");
    c.get_users(&Query::new().page_size(5).all_fields(UserAttributes::TYPE))
        .await
        .expect("GET /users");
    c.get_documents(&project, &Query::new().page_size(5))
        .await
        .expect("GET documents");
    let types = c
        .get_available_enum_options_for_work_item_type(&project, "type", &Query::new())
        .await
        .expect("GET work item type options");
    eprintln!(
        "work item types in {project}: {:?}",
        types
            .data
            .unwrap_or_default()
            .iter()
            .filter_map(|o| o.id.clone())
            .collect::<Vec<_>>()
    );

    let items = c
        .get_work_items(
            &project,
            &Query::new()
                .all_fields(WorkItemAttributes::TYPE)
                .include("author")
                .page_size(5),
        )
        .await
        .expect("GET work items");
    eprintln!("{:?} work items in {project}", items.total_count());
    let Some(item) = items.data.first() else {
        return;
    };
    let full_id = item.id.clone().expect("work item id");
    let id = short_id(&full_id);
    let attrs = item
        .attributes
        .as_ref()
        .expect("attributes with fields=@all");
    eprintln!(
        "{full_id}: {:?}, custom fields: {:?}",
        attrs.title,
        attrs.extra.keys().collect::<Vec<_>>()
    );
    if let Some(author) = item.relationship("author").and_then(|r| r.refs().first()) {
        let included = items.find_included::<UserAttributes>(&author.resource_type, &author.id);
        eprintln!(
            "included author: {:?}",
            included.and_then(|u| u.attributes).and_then(|a| a.name)
        );
    }

    c.get_work_item(
        &project,
        id,
        &Query::new().all_fields(WorkItemAttributes::TYPE),
    )
    .await
    .expect("GET work item");
    c.get_linked_work_items(&project, id, &Query::new())
        .await
        .expect("GET linked work items");
    c.get_comments(&project, id, &Query::new())
        .await
        .expect("GET comments");
    c.get_work_item_attachments(&project, id, &Query::new())
        .await
        .expect("GET attachments");
    c.get_fields_metadata_for_work_item(&project, id)
        .await
        .expect("GET fields metadata");

    let err = c
        .get_work_item(&project, "POLARION-RS-DOES-NOT-EXIST-0", &Query::new())
        .await
        .expect_err("missing work item");
    assert!(err.is_not_found(), "expected 404, got {err}");
}

#[tokio::test]
#[ignore = "live: needs POLARION_TESTDRIVE_TOKEN and POLARION_TESTDRIVE_PROJECT; writes to that project"]
async fn work_item_lifecycle() {
    let c = client(false);
    let project = env("POLARION_TESTDRIVE_PROJECT")
        .expect("set POLARION_TESTDRIVE_PROJECT to a project you own in the testdrive Sandbox");

    let mut created = Vec::new();
    let result = lifecycle(&c, &project, &mut created).await;

    if !created.is_empty() {
        let refs: Vec<ResourceRef> = created
            .iter()
            .map(|id| WorkItem::reference(id.clone()))
            .collect();
        match c.delete_work_items(&project, &refs).await {
            Ok(()) => {
                let gone = c
                    .get_work_item(&project, short_id(&created[0]), &Query::new())
                    .await;
                assert!(
                    matches!(&gone, Err(e) if e.is_not_found()),
                    "deleted work item still readable: {gone:?}"
                );
            }
            Err(e) => panic!("cleanup failed ({e}); delete manually in {project}: {created:?}"),
        }
    }
    result.unwrap();
}

async fn lifecycle(c: &Client, project: &str, created: &mut Vec<String>) -> TestResult {
    let wi_type = match env("POLARION_TESTDRIVE_WI_TYPE") {
        Some(t) => t,
        None => c
            .get_available_enum_options_for_work_item_type(project, "type", &Query::new())
            .await?
            .data
            .unwrap_or_default()
            .into_iter()
            .find_map(|o| o.id)
            .ok_or("project offers no work item types")?,
    };
    let role = env("POLARION_TESTDRIVE_LINK_ROLE").unwrap_or_else(|| "relates_to".into());
    let tag = format!(
        "polarion-rs-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis()
    );
    eprintln!("creating 3 `{wi_type}` work items tagged {tag} in {project}");

    // Create.
    let new: Vec<WorkItem> = (1..=3)
        .map(|i| {
            WorkItem::from_attributes(WorkItemAttributes {
                type_: Some(wi_type.clone()),
                title: Some(format!("{tag} #{i}")),
                description: Some(TextContent::html(format!(
                    "<p>Created by the polarion-rs live test ({i}).</p>"
                ))),
                ..Default::default()
            })
        })
        .collect();
    let resp = c.post_work_items(project, &new).await?;
    created.extend(resp.data.iter().filter_map(|r| r.id.clone()));
    check!(
        created.len() == 3,
        "expected 3 created ids, got {created:?}"
    );
    let ids: Vec<String> = created.iter().map(|id| short_id(id).to_owned()).collect();

    // Read back.
    let all_fields = Query::new().all_fields(WorkItemAttributes::TYPE);
    let got = c.get_work_item(project, &ids[0], &all_fields).await?;
    let attrs = got.data.attributes.ok_or("no attributes")?;
    check!(
        attrs.title.as_deref() == Some(&*format!("{tag} #1")),
        "title: {:?}",
        attrs.title
    );
    check!(
        attrs.type_.as_deref() == Some(&*wi_type),
        "type: {:?}",
        attrs.type_
    );
    check!(
        attrs
            .description
            .as_ref()
            .is_some_and(|d| d.value.contains("live test")),
        "description: {:?}",
        attrs.description
    );

    // Update one.
    let renamed = format!("{tag} #1 renamed");
    let patch = WorkItem::from_attributes(WorkItemAttributes {
        title: Some(renamed.clone()),
        ..Default::default()
    })
    .with_id(created[0].clone());
    c.patch_work_item(project, &ids[0], &Query::new(), &patch)
        .await?;
    let title = c
        .get_work_item(project, &ids[0], &all_fields)
        .await?
        .data
        .attributes
        .and_then(|a| a.title);
    check!(
        title.as_deref() == Some(&*renamed),
        "after patch: {title:?}"
    );

    // Update many.
    let bulk: Vec<WorkItem> = created[1..]
        .iter()
        .map(|id| {
            WorkItem::from_attributes(WorkItemAttributes {
                title: Some(format!("{tag} bulk")),
                ..Default::default()
            })
            .with_id(id.clone())
        })
        .collect();
    c.patch_work_items(project, &Query::new(), &bulk).await?;
    let title = c
        .get_work_item(project, &ids[2], &all_fields)
        .await?
        .data
        .attributes
        .and_then(|a| a.title);
    check!(
        title.as_deref() == Some(&*format!("{tag} bulk")),
        "after bulk patch: {title:?}"
    );

    // Query + pagination (the search index can lag behind writes).
    let query = Query::new()
        .query(format!("id:({})", ids.join(" ")))
        .page_size(2);
    let mut found = 0;
    for _ in 0..10 {
        let all = c
            .paginate(query.clone(), async |q| c.get_work_items(project, &q).await)
            .await?;
        found = all.data.len();
        if found == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    check!(found == 3, "query found {found} of 3 work items");

    // Links.
    let link = LinkedWorkItem::from_attributes(LinkedWorkItemAttributes {
        role: Some(role.clone()),
        suspect: Some(false),
        ..Default::default()
    })
    .with_relationship("workItem", WorkItem::reference(created[1].clone()));
    c.post_linked_work_items(project, &ids[0], &[link]).await?;
    let links = c
        .get_linked_work_items(project, &ids[0], &Query::new())
        .await?;
    let link_id = links
        .data
        .iter()
        .find(|l| {
            l.relationship("workItem")
                .is_some_and(|r| r.refs().iter().any(|t| t.id == created[1]))
        })
        .and_then(|l| l.id.clone())
        .ok_or_else(|| format!("link not listed: {:?}", links.data))?;
    let back = c
        .get_backlinked_work_items(project, &ids[1], &Query::new())
        .await?;
    check!(!back.data.is_empty(), "no backlinks on {}", ids[1]);
    c.delete_linked_work_items(
        project,
        &ids[0],
        &[ResourceRef::new(LinkedWorkItemAttributes::TYPE, link_id)],
    )
    .await?;
    let links = c
        .get_linked_work_items(project, &ids[0], &Query::new())
        .await?;
    check!(links.data.is_empty(), "link not deleted: {:?}", links.data);

    // Comments.
    let text = format!("comment from {tag}");
    let comment = WorkItemComment::from_attributes(WorkItemCommentAttributes {
        text: Some(TextContent::plain(text.clone())),
        ..Default::default()
    });
    c.post_comments(project, &ids[0], &[comment]).await?;
    let comments = c
        .get_comments(
            project,
            &ids[0],
            &Query::new().all_fields(WorkItemCommentAttributes::TYPE),
        )
        .await?;
    check!(
        comments.data.iter().any(|cm| cm
            .attributes
            .as_ref()
            .and_then(|a| a.text.as_ref())
            .is_some_and(|t| t.value.contains(&text))),
        "comment not found: {:?}",
        comments.data
    );

    // Attachments: multipart upload, then download.
    let content = format!("hello from {tag}\n").into_bytes();
    let resource = WorkItemAttachment::from_attributes(WorkItemAttachmentAttributes {
        file_name: Some("hello.txt".into()),
        title: Some("polarion-rs live test".into()),
        ..Default::default()
    })
    .with_lid("hello");
    let uploaded = c
        .post_work_item_attachments(
            project,
            &ids[0],
            Some(&[resource]),
            vec![FilePart::new("hello.txt", content.clone()).with_mime_type("text/plain")],
        )
        .await?;
    let attachment_id = uploaded
        .data
        .first()
        .and_then(|a| a.id.as_deref())
        .map(short_id)
        .ok_or("no attachment id")?
        .to_owned();
    let downloaded = c
        .get_work_item_attachment_content(project, &ids[0], &attachment_id, &Query::new())
        .await?;
    check!(
        downloaded == content,
        "downloaded {} bytes, expected {}",
        downloaded.len(),
        content.len()
    );

    Ok(())
}
