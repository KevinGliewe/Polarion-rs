//! The live suite against a real server: the tiers `read`, `write` and `permanent`, one
//! `#[ignore]` test each, their steps in a fixed order (the front-end tier is in `polarion-cli`
//! and `polarion-mcp`). See docs/testing.md and `live_support.rs` for the gates.
//!
//! ```sh
//! cargo test -p polarion --all-features --test live read -- --ignored --nocapture
//! cargo test -p polarion --all-features --test live write -- --ignored --nocapture
//! cargo test -p polarion --all-features --test live permanent -- --ignored --nocapture
//! ```
//!
//! Environment:
//! - every tier: `POLARION_URL`, `POLARION_TOKEN`, `POLARION_PROJECT` (`POLARION_INSECURE=1`
//!   optional); the read tier also uses the fixture variables below when they are set;
//! - `write` and `permanent`: `POLARION_LIVE_WRITES` (the host of `POLARION_URL`), and
//!   `POLARION_LIVE_ITEMS`, `POLARION_LIVE_TEST_RUN`, `POLARION_LIVE_DOCUMENT`,
//!   `POLARION_LIVE_USER` as the bootstrap (`examples/live_fixtures.rs`) prints them;
//!   `POLARION_LIVE_BASELINE` optional (default `~/.polarion-rs-live/<host>/<project>/`
//!   `baseline.json`, never inside `target/`; the bootstrap stores it);
//! - `permanent`: `POLARION_LIVE_PERMANENT` set to the value the read tier prints for today
//!   (`YYYY-MM-DD:A-ID`; comments cannot be deleted, the tier runs once and logs its run), and
//!   optionally `POLARION_LIVE_COMMENT_TAG`.
//!
//! The read tier only reads, with a read-only client. The writing tiers write only to the
//! fixtures, through a client restricted to their URIs, after the gates of
//! `live_support::write_gate`; each step reads the state, changes it, verifies it, restores it,
//! and the fixtures are then compared with the baseline field by field. They end with the
//! index probe and the audit.
//!
//! Never, in any tier: workflow transitions or status and resolution writes, deletes of work
//! items, documents or plans, creates (only the bootstrap creates), test executions, summary
//! defects, external, OSLC and SCM links, auto-suspect, auto-assign, users, global writes,
//! assigning or approving anyone but `POLARION_LIVE_USER`, test results other than `passed`,
//! and the writer's `soap()`.

#[macro_use]
#[path = "live_support.rs"]
mod live_support;

#[path = "../../polarion-markdown/tests/support/visual.rs"]
mod visual;

use std::cell::Cell;
use std::collections::BTreeSet;
use std::fmt::Display;

use live_support::*;
use polarion::polarion_markdown::{
    self as md, DiagramAttachments, DiagramSources, RawHtml, ToHtmlOptions, ToMarkdownOptions,
    WorkItemIds,
};
use polarion::{
    ActivityScope, DocumentRef, DocumentUpdate, Error, FieldValue, Fields, ItemRef,
    MarkdownReadOptions, MarkdownWriteOptions, NewWorkRecord, Polarion, Query, TestRecord,
    TestRunUpdate, TestStepResult, Text, WorkItem, WorkItemUpdate,
};
use serde::Serialize;
use serde_json::Value;

fn to_json(value: &impl Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// A write of a step: `OK label`, or the error that ends the step.
fn sent<T>(
    report: &mut Report,
    label: impl Display,
    result: polarion::Result<T>,
) -> Result<T, String> {
    match result {
        Ok(value) => {
            report.ok(label);
            Ok(value)
        }
        Err(e) => Err(format!("{label}: {e}")),
    }
}

/// Whether a write was refused before anything was sent (by the guard or the SOAP filter).
fn refused_before_sending<T>(result: &polarion::Result<T>) -> bool {
    matches!(
        result,
        Err(Error::WriteNotAllowed { .. }
            | Error::Soap(
                polarion::soap::Error::WriteFiltered { .. }
                    | polarion::soap::Error::ReadOnly { .. }
            ))
    )
}

/// A read of a step: its error ends the step.
fn got<T>(what: impl Display, result: polarion::Result<T>) -> Result<T, String> {
    result.map_err(|e| format!("{what}: {e}"))
}

// ==================================================================== read

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: reads from the server in POLARION_URL (see docs/testing.md)"]
async fn read() {
    let _tier = one_tier("read");
    let settings = Settings::from_env();
    let reader = reader(&settings);
    let mut report = Report::default();
    println!(
        "read tier against {} (project {})",
        settings.host, settings.project
    );
    let p = settings.project.as_str();
    projects_and_users(&mut report, &reader, p).await;
    let item = items(&mut report, &reader, p).await;
    documents(&mut report, &reader, p).await;
    other_areas(&mut report, &reader, p, item.as_ref()).await;
    uris(&mut report, &reader, p).await;
    fixtures_read_only(&mut report, &settings, &reader).await;
    not_found(&mut report, &reader, p).await;
    markdown_read(&mut report, &reader, p).await;
    session_renewal(&mut report, &reader, p).await;
    report.result("logout", reader.logout().await);
    report.finish("read");
}

async fn projects_and_users(report: &mut Report, reader: &Polarion, p: &str) {
    if let Some(project) = report.result(
        format!("projects().get({p})"),
        reader.projects().get(p).await,
    ) {
        report.info(format!("{:?} ({:?})", project.name, project.id));
    }
    match env("POLARION_LIVE_USER") {
        Some(user) => {
            if let Some(u) = report.result(
                format!("users().get({user})"),
                reader.users().get(&user).await,
            ) {
                report.info(format!("{:?}", u.name));
            }
        }
        None => report.skip("users().get", "POLARION_LIVE_USER is not set"),
    }
}

/// Item reads on the most recently updated work item; returns it.
async fn items(report: &mut Report, reader: &Polarion, p: &str) -> Option<ItemRef> {
    let items = reader.items();
    let base = Query::new("").project(p).sort("~updated");
    let first = report
        .result(
            "items().query (limit 1)",
            items.query(&base.clone().limit(1)).await,
        )?
        .into_iter()
        .next();
    if let Some(count) = report.result("items().count", items.count(&base).await) {
        report.info(format!("{count} work items in {p}"));
    }
    if let Some(refs) = report.result(
        "items().query_refs (limit 1)",
        items.query_refs(&base.clone().limit(1)).await,
    ) {
        report.info(format!(
            "{:?}",
            refs.iter().map(|r| r.uri.clone()).collect::<Vec<_>>()
        ));
    }
    let Some(item) = first.and_then(|w| w.reference()) else {
        report.skip("work item reads", "the project has no work items");
        return None;
    };
    if let Some(w) = report.result(format!("items().get({item})"), items.get(&item).await) {
        report.info(format!(
            "type {:?}, status {:?}, {} custom fields",
            w.kind,
            w.status,
            w.custom_fields.len()
        ));
        check!(
            report,
            w.uri.as_deref()
                == Some(
                    ItemRef::new(p, w.id.as_deref().unwrap_or_default())
                        .uri
                        .as_str()
                ),
            "ItemRef::new builds the server's URI",
            "server {:?}",
            w.uri
        );
    }
    if let Some(history) = report.result("items().history", items.history(&item).await) {
        for c in history.iter().rev().take(5) {
            report.info(format!(
                "revision {:?} at {:?} by {:?} ({} field changes)",
                c.revision,
                c.date,
                c.user,
                c.changes.len()
            ));
        }
        let users: BTreeSet<String> = history.iter().filter_map(|c| c.user.clone()).collect();
        report.info(format!("users in the history: {users:?}"));
    }
    if let Some(actions) = report.result("items().actions", items.actions(&item).await) {
        report.info(format!(
            "{:?}",
            actions.iter().map(|a| a.name.clone()).collect::<Vec<_>>()
        ));
    }
    if let Some(links) = report.result("items().back_links", items.back_links(&item).await) {
        report.info(format!("{} back links", links.len()));
    }
    let keys = report
        .result(
            "items().custom_field_keys",
            items.custom_field_keys(&item).await,
        )
        .unwrap_or_default();
    let mut fields = Fields::new(["id", "module", "plannedIn"]);
    if let Some(key) = keys.first() {
        fields = fields.with_custom(key);
    }
    if let Some(w) = report.result(
        format!("items().get_fields({:?})", fields.keys()),
        items.get_fields(&item, &fields).await,
    ) {
        report.info(format!(
            "document {:?}, planned in {:?}, custom fields {}",
            w.document.map(|d| d.to_string()),
            w.planned_in,
            short(&to_json(&w.custom_fields))
        ));
    }
    if let Some(revisions) = report.result("items().revisions", items.revisions(&item).await)
        && let Some(last) = revisions.last()
        && let Some(revision) = report.result(
            format!("revisions().get(default, {last})"),
            reader.revisions().get("default", last).await,
        )
    {
        report.info(format!("{:?} by {:?}", revision.created, revision.author));
    }
    Some(item)
}

async fn documents(report: &mut Report, reader: &Polarion, p: &str) {
    let docs = reader.documents();
    if let Some(spaces) = report.result("documents().spaces", docs.spaces(p).await) {
        report.info(format!("{} spaces", spaces.len()));
    }
    let Some(list) = report.result(
        "documents().query (limit 1)",
        docs.query(&Query::new("").project(p).limit(1)).await,
    ) else {
        return;
    };
    let Some((space, name)) = list
        .into_iter()
        .next()
        .and_then(|d| Some((d.space?, d.name?)))
    else {
        report.skip("document reads", "no document found");
        return;
    };
    if let Some(in_space) = report.result(
        format!("documents().list({space})"),
        docs.list(p, Some(&space)).await,
    ) {
        report.info(format!("{} documents in {space}", in_space.len()));
    }
    if let Some(doc) = report.result(
        format!("documents().get({space}/{name}) by location"),
        docs.get(p, &space, &name, false).await,
    ) {
        let built = DocumentRef::new(p, &space, &name);
        check!(
            report,
            doc.uri.as_deref() == Some(built.uri.as_str()),
            "DocumentRef::new builds the server's URI",
            "server {:?}, built {}",
            doc.uri,
            built.uri
        );
        if let Some(uri) = &doc.uri {
            revisions_readable(report, reader, "a document", uri).await;
        }
        if let Some(reference) = doc.reference()
            && let Some(refs) = report.result(
                "documents().work_item_refs (top level)",
                docs.work_item_refs(&reference, None, false).await,
            )
        {
            report.info(format!("{} work items at the top level", refs.len()));
        }
    }
}

async fn other_areas(report: &mut Report, reader: &Polarion, p: &str, item: Option<&ItemRef>) {
    let schema = reader.schema();
    if let Some(fields) = report.result(
        format!("schema().custom_fields({p}, {TYPE_A})"),
        schema.custom_fields(p, TYPE_A).await,
    ) {
        let types: BTreeSet<String> = fields.iter().map(type_key).collect();
        report.info(format!("{} fields, types {types:?}", fields.len()));
    }
    if let Some(options) = report.result(
        "schema().field_options(type)",
        schema.field_options(p, "type").await,
    ) {
        report.info(format!(
            "{:?}",
            options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>()
        ));
    }
    if let Some(c) = report.result("schema().categories", schema.categories(p).await) {
        report.info(format!("{} categories", c.len()));
    }
    if let Some(t) = report.result("schema().time_points", schema.time_points(p).await) {
        report.info(format!("{} time points", t.len()));
    }
    if let Some(l) = report.result("schema().languages", schema.languages(p).await) {
        report.info(format!(
            "{:?}",
            l.iter().map(|l| l.id.clone()).collect::<Vec<_>>()
        ));
    }
    if let Some(h) = report.result(
        "schema().history_available",
        schema.history_available().await,
    ) {
        report.info(format!("{h}"));
    }

    let tests = reader.tests();
    let runs = report.result(
        "tests().search (limit 1)",
        tests
            .search(
                &Query::new("")
                    .project(p)
                    .limit(1)
                    .fields(Fields::new(["id", "title", "status", "created"])),
            )
            .await,
    );
    match runs.and_then(|r| r.into_iter().next()) {
        Some(run) if run.uri.is_some() => {
            let uri = run.uri.clone().unwrap_or_default();
            if let Some(id) = &run.id {
                check!(
                    report,
                    polarion::uri::test_run_uri(p, id) == base_uri(&uri),
                    "uri::test_run_uri builds the server's URI",
                    "server {uri}, built {}",
                    polarion::uri::test_run_uri(p, id)
                );
            }
            report.result(
                "tests().get_by_uri",
                tests
                    .get_by_uri(&uri, Some(&Fields::new(["id", "title", "status"])))
                    .await,
            );
            if let Some(a) = report.result("tests().attachments", tests.attachments(&uri).await) {
                report.info(format!("{} attachments", a.len()));
            }
            revisions_readable(report, reader, "a test run", &uri).await;
            if let Some(r) = report.result("tests().report", tests.report(&uri).await) {
                report.info(format!("{} characters", r.content.chars().count()));
            }
        }
        _ => report.skip("test run reads", "no test run found"),
    }
    let cases = report.result(
        "items().query (type:testcase, limit 1)",
        reader
            .items()
            .query(&Query::new(format!("type:{TYPE_B}")).project(p).limit(1))
            .await,
    );
    if let Some(case) = cases
        .and_then(|c| c.into_iter().next())
        .and_then(|c| c.reference())
    {
        if let Some(steps) =
            report.result(format!("tests().steps({case})"), tests.steps(&case).await)
        {
            report.info(format!(
                "columns {:?}, {} steps",
                steps.columns,
                steps.steps.len()
            ));
        }
        report.result(
            "tests().parameter_names",
            tests.parameter_names(&case).await,
        );
    }
    if let Some(columns) = report.result(
        format!("tests().steps_configuration({TYPE_B})"),
        tests.steps_configuration(p, Some(TYPE_B)).await,
    ) {
        report.info(format!(
            "{:?}",
            columns.iter().map(|c| c.id.as_str()).collect::<Vec<_>>()
        ));
    }
    if let Some(fields) = report.result("tests().custom_fields", tests.custom_fields(Some(p)).await)
    {
        report.info(format!("{} test run custom fields", fields.len()));
    }
    if let Some(config) = report.result("tests().configuration", tests.configuration(p).await) {
        report.info("test management configuration (defect reuse and creation):");
        show(&to_json(&config));
    }

    let plans = reader.plans();
    let found = report.result(
        "plans().search (limit 1)",
        plans.search(&Query::new("").project(p).limit(1)).await,
    );
    match found
        .and_then(|l| l.into_iter().next())
        .and_then(|plan| plan.uri)
    {
        Some(uri) => {
            report.result("plans().get_by_uri", plans.get_by_uri(&uri).await);
            report.result("plans().statistics", plans.statistics(&uri).await);
            report.result("plans().was_started", plans.was_started(&uri).await);
        }
        None => report.skip("plan reads", "no plan found"),
    }

    let wiki = reader.wiki();
    if let Some(spaces) = report.result("wiki().spaces", wiki.spaces(p).await) {
        report.info(format!("{} wiki spaces", spaces.len()));
    }
    report.result(
        "wiki().query (limit 1)",
        wiki.query(&Query::new("").project(p).limit(1)).await,
    );
    if let Some(b) = report.result(
        "baselines().query",
        reader.baselines().query(&Query::new("").project(p)).await,
    ) {
        report.info(format!("{} baselines", b.len()));
    }
    report.result(
        "activities().list (1)",
        reader
            .activities()
            .list(ActivityScope::Project(p), None, 1)
            .await,
    );
    if let Some(folders) = report.result("folders().list", reader.folders().list(Some(p)).await) {
        report.info(format!("{} folders", folders.len()));
    }
    if let Some(builds) = report.result("builds().of_project", reader.builds().of_project(p).await)
    {
        report.info(format!("{} builds", builds.len()));
    }

    let security = reader.security();
    if let Some(item) = item {
        report.result("security().can_read", security.can_read(&item.uri).await);
        report.result(
            "security().can_modify",
            security.can_modify(&item.uri).await,
        );
        if let Some(location) = report.result(
            "security().location_of",
            security.location_of(&item.uri).await,
        ) {
            report.info(location);
        }
    }
    report.result("security().global_roles", security.global_roles().await);
    if let Some(user) = env("POLARION_LIVE_USER") {
        report.result(
            "security().roles_for_user",
            security.roles_for_user(&user, None).await,
        );
    }
    report.result(
        "security().product_license",
        security.product_license().await,
    );
}

/// The revisions of a document or test run, as the audit reads them: `getRevisions`, then the
/// newest revision's author and date (which must parse).
async fn revisions_readable(report: &mut Report, reader: &Polarion, what: &str, uri: &str) {
    let Some(revisions) = report.result(
        format!("getRevisions of {what} (the audit reads them)"),
        reader.soap().tracker().get_revisions(uri).await,
    ) else {
        return;
    };
    report.info(format!("{} revisions", revisions.len()));
    let Some(last) = revisions.last() else {
        report.skip(format!("the newest revision of {what}"), "none");
        return;
    };
    if let Some(revision) = report.result(
        format!("revisions().get(default, {last}) of {what}"),
        reader.revisions().get("default", last).await,
    ) {
        check!(
            report,
            revision.author.is_some() && revision.created.as_deref().and_then(parse_iso).is_some(),
            format!("the newest revision of {what} has an author and a date that parses"),
            "author {:?}, created {:?}",
            revision.author,
            revision.created
        );
    }
}

/// The URI formats the guards rely on, checked against real URIs.
async fn uris(report: &mut Report, reader: &Polarion, p: &str) {
    match owner_samples(reader, p).await {
        Ok(samples) => {
            check_owner_samples(report, "work item comments", &samples.comments);
            check_owner_samples(report, "work records", &samples.work_records);
            check_owner_samples(report, "document comments", &samples.document_comments);
        }
        Err(e) => report.fail("comment and work record URIs", e),
    }
    // The audit's query; the form `TO *` is printed for comparison.
    let since = lucene_date(now() - 86_400);
    for upper in ["99991231", "*"] {
        let lucene = format!("updated:[{since} TO {upper}]");
        if let Some(count) = report.result(
            format!("items().count(`{lucene}`)"),
            reader
                .items()
                .count(&Query::new(lucene.as_str()).project(p))
                .await,
        ) {
            report.info(format!("{count} work items"));
        }
    }
    // The same query for documents and test runs, as the audit makes it.
    let lucene = format!("updated:[{since} TO 99991231]");
    let query = Query::new(lucene.as_str()).project(p).limit(1);
    report.result(
        format!("documents().query(`{lucene}`, limit 1)"),
        reader.documents().query(&query).await,
    );
    report.result(
        format!("tests().search(`{lucene}`, limit 1)"),
        reader.tests().search(&query).await,
    );
}

/// With the fixture variables set: the gate's read-only checks, and what the audit relies on.
async fn fixtures_read_only(report: &mut Report, settings: &Settings, reader: &Polarion) {
    let ids = match FixtureIds::from_env() {
        Ok(ids) => ids,
        Err(e) => {
            report.skip("fixtures (read-only)", e);
            return;
        }
    };
    let f = match Fixtures::resolve(reader, &settings.project, &ids).await {
        Ok(f) => f,
        Err(e) => {
            report.fail("fixtures", e);
            return;
        }
    };
    report.info(f.describe());
    check_identity(report, reader, &f).await;
    // Gate 3 reads its fields by key: `get()` must give the same, or one of the two leaves
    // something out.
    for which in [Which::A, Which::B] {
        let item = f.get(which);
        let full = reader.items().get(item).await;
        let by_key = reader
            .items()
            .get_fields(item, &Fields::new(IDENTITY_FIELDS))
            .await;
        match (full, by_key) {
            (Ok(full), Ok(by_key)) => {
                let (full, by_key) = (identity_of(&full), identity_of(&by_key));
                check!(
                    report,
                    full == by_key,
                    format!(
                        "fixtures: {} has the same {IDENTITY_FIELDS:?} by get() and by key",
                        which.label()
                    ),
                    "get() {}, by key {}",
                    short(&full),
                    short(&by_key)
                );
            }
            (Err(e), _) | (_, Err(e)) => report.fail(format!("fixtures: {}", which.label()), e),
        }
    }
    // What restoring relies on: the formats of planning constraints, records and the localized
    // title.
    match planning_constraints(reader, &f.a).await {
        Ok(list) => report.info(format!("A's planning constraints: {list:?}")),
        Err(e) => report.fail("fixtures: A's planning constraints", e),
    }
    if let Some(run) = report.result("fixtures: R", reader.tests().get_by_uri(&f.r, None).await) {
        report.info(format!("R's records: {}", short(&to_json(&run.records))));
    }
    if let Some(title) = report.result(
        "fixtures: A's localized title (default language)",
        reader.items().localized_title(&f.a, None).await,
    ) {
        report.info(format!("{title:?}"));
    }
    report.info(format!(
        "to run the permanent tier today: POLARION_LIVE_PERMANENT={}",
        permanent_key(&ids)
    ));
    match back_links(reader, &f).await {
        Ok((all, foreign)) => {
            check!(
                report,
                foreign.is_empty(),
                format!("fixtures: {} back links, from fixtures only", all.len()),
                "other items link to the fixtures: {foreign:?}"
            );
        }
        Err(e) => report.fail("fixtures: back links", e),
    }
    if let Some(records) = report.result(
        "tests().records(R, B)",
        reader.tests().records(&f.r, &f.b).await,
    ) {
        report.info(format!("{} records", records.len()));
    }
    check!(
        report,
        polarion::uri::test_run_uri(&f.project, &f.r_id) == f.r,
        "uri::test_run_uri builds R's URI",
        "server {}, built {}",
        f.r,
        polarion::uri::test_run_uri(&f.project, &f.r_id)
    );
    match reader.items().history(&f.a).await {
        Ok(history) => {
            let users: BTreeSet<String> = history.iter().filter_map(|c| c.user.clone()).collect();
            check!(
                report,
                users.iter().any(|u| u.eq_ignore_ascii_case(&f.user)),
                "fixtures: A's history names POLARION_LIVE_USER (the audit relies on it)",
                "users in A's history: {users:?}"
            );
            let unreadable: Vec<String> = history
                .iter()
                .filter_map(|c| c.date.clone())
                .filter(|d| parse_iso(d).is_none())
                .collect();
            check!(
                report,
                unreadable.is_empty(),
                "fixtures: the dates in A's history parse (the audit relies on it)",
                "unreadable dates: {unreadable:?}"
            );
        }
        Err(e) => report.fail("fixtures: A's history", e),
    }
    let current = match definitions(reader, &f).await {
        Ok(defs) => match snapshot(reader, &f, &defs).await {
            Ok(current) => Some(current),
            Err(e) => {
                report.fail("fixtures: snapshot", e);
                None
            }
        },
        Err(e) => {
            report.fail("fixtures: custom field definitions", e);
            None
        }
    };
    let Some(current) = current else {
        return;
    };
    let problems = baseline_problems(&f, &current);
    if problems.is_empty() {
        report.ok("fixtures: nothing a stopped run leaves behind");
    } else {
        report.skip(
            "fixtures: what a stopped run leaves behind",
            format!(
                "{} findings (the bootstrap does not store such a state as the baseline)",
                problems.len()
            ),
        );
        for p in problems {
            report.info(p);
        }
    }
    match load_baseline(settings, &f) {
        Ok(Some(baseline)) => {
            let diffs = differences(&baseline, &current);
            if diffs.is_empty() {
                report.ok("fixtures: they match the baseline");
            } else {
                report.skip(
                    "fixtures: compared with the baseline",
                    format!(
                        "{} differences (the write tier restores them first)",
                        diffs.len()
                    ),
                );
                for d in diffs {
                    report.info(d);
                }
            }
        }
        Ok(None) => report.skip(
            "fixtures: baseline",
            format!(
                "no file {} (the bootstrap stores it)",
                baseline_path(settings).map_or_else(|e| e, |p| p.display().to_string())
            ),
        ),
        Err(e) => report.fail("fixtures: baseline", e),
    }
}

async fn not_found(report: &mut Report, reader: &Polarion, p: &str) {
    match reader
        .items()
        .get(&ItemRef::new(p, "POLARIONRS-DOES-NOT-EXIST-1"))
        .await
    {
        Err(Error::NotFound { .. }) => report.ok("items().get(missing) -> NotFound"),
        Err(e) => report.fail(
            "items().get(missing)",
            format!("expected NotFound, got: {e}"),
        ),
        Ok(_) => report.fail("items().get(missing)", "expected NotFound, got a work item"),
    }
}

/// Three recent descriptions read as Markdown, and the offline round trip of their HTML.
async fn markdown_read(report: &mut Report, reader: &Polarion, p: &str) {
    let query = Query::new("")
        .project(p)
        .sort("~updated")
        .limit(20)
        .fields(Fields::new(["id", "description"]));
    let Some(list) = report.result(
        "items().query with descriptions (20)",
        reader.items().query(&query).await,
    ) else {
        return;
    };
    let described: Vec<WorkItem> = list
        .into_iter()
        .filter(|w| {
            w.description
                .as_ref()
                .is_some_and(|d| d.content_type == "text/html" && !d.content.trim().is_empty())
        })
        .take(3)
        .collect();
    if described.is_empty() {
        report.skip(
            "Markdown read",
            "none of the 20 most recently updated work items has an HTML description",
        );
        return;
    }
    for w in &described {
        let (Some(item), Some(id), Some(description)) =
            (w.reference(), w.id.clone(), &w.description)
        else {
            continue;
        };
        if let Some(markdown) = report.result(
            format!("{id}: description_markdown"),
            reader
                .items()
                .description_markdown(&item, &MarkdownReadOptions::default())
                .await,
        ) {
            report.info(format!(
                "{} characters of Markdown from {} of HTML",
                markdown.chars().count(),
                description.content.chars().count()
            ));
        }
        let prefix = id
            .rsplit_once('-')
            .map_or(id.as_str(), |(p, _)| p)
            .to_owned();
        round_trip(
            report,
            &id,
            &description.content,
            &WorkItemIds::Prefixes(vec![prefix]),
        );
    }
}

/// The offline round trip of a description in both modes: no fallbacks, a fixpoint, and the
/// visual check (formatting kept: it must look the same; dropped: the difference is shown).
/// Whether the visual models of `html` and `back` differ only by blank lines (`space` rows) that
/// `back` adds next to a heading line.
fn only_blank_lines_at_headings(html: &str, back: &str) -> bool {
    let (expected, actual) = (visual::render(html), visual::render(back));
    let (expected, actual): (Vec<&str>, Vec<&str>) =
        (expected.lines().collect(), actual.lines().collect());
    let heading = |row: Option<&&str>| row.is_some_and(|r| r.starts_with("line heading="));
    let (mut i, mut j) = (0, 0);
    while j < actual.len() {
        if i < expected.len() && expected[i] == actual[j] {
            i += 1;
        } else if actual[j].starts_with("space ")
            && (heading(actual.get(j + 1)) || j > 0 && heading(actual.get(j - 1)))
        {
            // A blank line the conversion added next to a heading.
        } else {
            return false;
        }
        j += 1;
    }
    i == expected.len()
}

fn round_trip(report: &mut Report, id: &str, html: &str, ids: &WorkItemIds) {
    for raw_html in [RawHtml::Keep, RawHtml::Drop] {
        let mode = match raw_html {
            RawHtml::Keep => "formatting kept",
            RawHtml::Drop => "formatting dropped",
        };
        let options = ToMarkdownOptions {
            raw_html,
            work_items: ids.clone(),
        };
        let converted = md::to_markdown_report(html, &options, &DiagramSources::new());
        check!(
            report,
            converted.fallbacks == 0 && converted.invalid == 0,
            format!("{id} ({mode}): no fallbacks"),
            "{} fallbacks, {} invalid trees",
            converted.fallbacks,
            converted.invalid
        );
        let back = md::to_html(
            &converted.markdown,
            &ToHtmlOptions {
                work_items: ids.clone(),
                highlight: true,
            },
        );
        let again = md::to_markdown(&back, &options);
        check!(
            report,
            again == converted.markdown,
            format!("{id} ({mode}): the Markdown is a fixpoint"),
            "{}",
            first_difference(&converted.markdown, &again)
        );
        // Checked in keep mode, as `polarion-markdown/tests/round_trip.rs` does: kept formatting
        // must look the same. Drop mode leaves formatting out on purpose: its difference is only
        // shown (docs/testing.md).
        let label = format!("{id} ({mode}): the HTML of the Markdown looks like the description");
        match (raw_html, visual::difference(html, &back)) {
            (RawHtml::Keep, None) => report.ok(label),
            (RawHtml::Keep, Some(_)) if only_blank_lines_at_headings(html, &back) => {
                report.skip(
                    label,
                    "only blank lines next to headings differ: a heading becomes a Markdown \
                     heading, which the conversion separates by a paragraph break (a known \
                     limitation, docs/markdown.md)",
                );
            }
            (RawHtml::Keep, Some(difference)) => {
                report.fail(
                    label,
                    difference.lines().take(8).collect::<Vec<_>>().join(" | "),
                );
            }
            (RawHtml::Drop, None) => report.info(format!("{id} ({mode}): looks the same")),
            (RawHtml::Drop, Some(difference)) => {
                report.info(format!(
                    "{id} ({mode}): looks different (formatting dropped, not checked):"
                ));
                report.info(difference.lines().take(4).collect::<Vec<_>>().join("\n"));
            }
        }
    }
}

/// The server ends the reader's session; the next read must log in again and return real data.
async fn session_renewal(report: &mut Report, reader: &Polarion, p: &str) {
    match reader.soap().session().end_session().await {
        Ok(()) => report.ok("endSession: the server ended the reader's session"),
        Err(e) => {
            report.fail("endSession", e);
            return;
        }
    }
    match reader.projects().get(p).await {
        Ok(project) => {
            check!(
                report,
                project.id.as_deref() == Some(p),
                "a read after endSession returns real data (the session was renewed)",
                "got project {:?}",
                project.id
            );
        }
        Err(e) => report.fail("a read after endSession", e),
    }
}

// ==================================================================== write

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: writes to the fixtures on the server in POLARION_URL (see docs/testing.md)"]
async fn write() {
    let _tier = one_tier("write");
    let mut report = Report::default();
    if let Some(g) = write_gate(&mut report).await {
        write_steps(&mut report, &g).await;
        g.close(&mut report).await;
    }
    report.finish("write");
}

async fn write_steps(report: &mut Report, g: &Gate) {
    let token = run_token();
    report.info(format!("run token {token}"));
    // Set once D's body was written: then the audit's document query must see D.
    let wrote_d = Cell::new(false);
    // A step, then the comparison with the baseline (and the repair); when the fixtures cannot
    // be restored, no further write: only the audit, once the index shows A as it is (without
    // the index probe's writes).
    macro_rules! step {
        ($name:expr, $call:expr) => {
            let result = $call.await;
            if !g.after(report, $name, result).await {
                wait_for_index(report, g).await;
                audit(report, g, wrote_d.get()).await;
                return;
            }
        };
    }
    step!(
        "A: title and description",
        a_title_description(report, g, &token)
    );
    step!("A: custom fields", a_custom_fields(report, g, &token));
    step!("A: hyperlinks", a_hyperlinks(report, g, &token));
    step!("A: categories", a_categories(report, g));
    step!("A: planning constraints", a_planning_constraints(report, g));
    step!("A: link to B", a_link(report, g));
    step!("A: assignee", a_assignee(report, g));
    step!("A: approval", a_approval(report, g));
    step!("A: localized title", a_localized_title(report, g, &token));
    step!("A: attachments", a_attachments(report, g, &token));
    step!("A: work record", a_work_record(report, g));
    step!("A: Markdown description", a_markdown(report, g, &token));
    step!(
        "B and R: test steps and records",
        b_and_r_records(report, g, &token)
    );
    step!("R: fields and report", r_fields(report, g, &token));
    step!("R: attachments", r_attachments(report, g, &token));
    step!("D: body", d_body(report, g, &token, &wrote_d));
    step!("transactions", transactions(report, g, &token));
    if index_probe(report, g).await {
        report.ok("the index shows the run's last change");
    }
    if g.after(report, "index probe", Ok(())).await {
        report.ok("every step left the fixtures at the baseline");
    }
    audit(report, g, wrote_d.get()).await;
}

/// Fields of A, read with the reader.
async fn a_fields(g: &Gate, fields: &[&str]) -> Result<WorkItem, String> {
    got(
        "A",
        g.reader
            .items()
            .get_fields(&g.fixtures.a, &Fields::new(fields.iter().copied()))
            .await,
    )
}

async fn a_title_description(report: &mut Report, g: &Gate, token: &str) -> Result<(), String> {
    let (f, items) = (&g.fixtures, g.writer.items());
    let before = a_fields(g, &["title", "description"]).await?;
    let title = format!("{MARKER_A} title changed by the live suite {token}");
    let description = html(format!(
        "<p>{MARKER_A} description changed by the live suite {token}</p>"
    ));
    let update = WorkItemUpdate {
        title: Some(title.clone()),
        description: Some(description),
        ..WorkItemUpdate::default()
    };
    sent(
        report,
        "A: updateWorkItem (title, description)",
        items.update(&f.a, &update).await,
    )?;
    let changed = a_fields(g, &["title", "description"]).await?;
    check!(
        report,
        changed.title.as_deref() == Some(title.as_str()),
        "A: the title was written (marker kept)",
        "title is {:?}",
        changed.title
    );
    check!(
        report,
        changed
            .description
            .as_ref()
            .is_some_and(|d| d.content.contains(token)),
        "A: the description was written",
        "description is {}",
        short(&to_json(&changed.description))
    );
    let restore = WorkItemUpdate {
        title: before.title.clone(),
        description: before.description.clone(),
        ..WorkItemUpdate::default()
    };
    sent(
        report,
        "A: title and description restored",
        items.update(&f.a, &restore).await,
    )?;
    if before.description.is_none() {
        sent(
            report,
            "A: description cleared",
            items.clear_fields(&f.a, &["description".to_owned()]).await,
        )?;
    }
    let after = a_fields(g, &["title", "description"]).await?;
    check!(
        report,
        after.title == before.title && after.description == before.description,
        "A: title and description as before",
        "title {:?}, description {}",
        after.title,
        short(&to_json(&after.description))
    );
    Ok(())
}

async fn a_custom_fields(report: &mut Report, g: &Gate, token: &str) -> Result<(), String> {
    let f = &g.fixtures;
    let (items, reader) = (g.writer.items(), g.reader.items());
    let fields = representatives(&g.defs.a);
    if fields.is_empty() {
        report.skip("A: custom fields", format!("{TYPE_A} defines none"));
        return Ok(());
    }
    for def in fields {
        let label = format!("A: custom field {} ({})", def.id, type_key(def));
        if let Some(control) = &def.depends_on {
            report.skip(&label, format!("its options depend on the field {control}"));
            continue;
        }
        let original = got(&label, reader.custom_field(&f.a, &def.id).await)?;
        let value = test_value(
            &g.reader,
            &f.project,
            def,
            original.as_ref(),
            Some(f.user.as_str()),
            token,
        )
        .await?;
        let Some(value) = value else {
            report.skip(&label, "no safe test value for this type");
            continue;
        };
        sent(
            report,
            format!("{label}: setCustomField {}", short(&to_json(&value))),
            items.set_custom_field(&f.a, &def.id, Some(&value)).await,
        )?;
        let read = got(&label, reader.custom_field(&f.a, &def.id).await)?;
        // Polarion stores a boolean `false` as no value (checked against a real server).
        let false_as_none = value == FieldValue::Bool(false) && read.is_none();
        check!(
            report,
            same_value(&value, read.as_ref()) || false_as_none,
            format!("{label}: read back"),
            "wrote {}, read {}",
            short(&to_json(&value)),
            short(&to_json(&read))
        );
        sent(
            report,
            format!("{label}: setFieldsNull"),
            items
                .clear_fields(&f.a, std::slice::from_ref(&def.id))
                .await,
        )?;
        let read = got(&label, reader.custom_field(&f.a, &def.id).await)?;
        check!(
            report,
            read.is_none(),
            format!("{label}: cleared"),
            "read {}",
            short(&to_json(&read))
        );
        if let Some(original) = &original {
            sent(
                report,
                format!("{label}: restored"),
                items.set_custom_field(&f.a, &def.id, Some(original)).await,
            )?;
        }
        let read = got(&label, reader.custom_field(&f.a, &def.id).await)?;
        check!(
            report,
            read == original,
            format!("{label}: as before"),
            "before {}, now {}",
            short(&to_json(&original)),
            short(&to_json(&read))
        );
    }
    Ok(())
}

async fn a_hyperlinks(report: &mut Report, g: &Gate, token: &str) -> Result<(), String> {
    let (f, items) = (&g.fixtures, g.writer.items());
    let url = format!("{TEST_URL}{token}");
    let role = match g.reader.items().field_options(&f.a, "hyperlinks").await {
        Ok(options) => options
            .into_iter()
            .find(|o| !o.hidden && !o.id.is_empty())
            .map_or_else(|| "ref_ext".to_owned(), |o| o.id),
        Err(e) => {
            report.info(format!("the hyperlink roles cannot be read ({e}): ref_ext"));
            "ref_ext".to_owned()
        }
    };
    let urls = |w: &WorkItem| {
        w.hyperlinks
            .iter()
            .map(|h| h.url.clone())
            .collect::<Vec<_>>()
    };
    let added = sent(
        report,
        format!("A: addHyperlink {url} ({role})"),
        items.add_hyperlink(&f.a, &url, &role).await,
    )?;
    let listed = urls(&a_fields(g, &["hyperlinks"]).await?);
    check!(
        report,
        added && listed.contains(&url),
        "A: the hyperlink is listed",
        "changed: {added}, hyperlinks {listed:?}"
    );
    let removed = sent(
        report,
        "A: removeHyperlink",
        items.remove_hyperlink(&f.a, &url).await,
    )?;
    let listed = urls(&a_fields(g, &["hyperlinks"]).await?);
    check!(
        report,
        removed && !listed.contains(&url),
        "A: the hyperlink is gone",
        "changed: {removed}, hyperlinks {listed:?}"
    );
    Ok(())
}

async fn a_categories(report: &mut Report, g: &Gate) -> Result<(), String> {
    let (f, items) = (&g.fixtures, g.writer.items());
    let categories = got("categories", g.reader.schema().categories(&f.project).await)?;
    let have = a_fields(g, &["categories"]).await?.categories;
    let Some(category) = categories
        .iter()
        .filter_map(|c| c.id.clone())
        .find(|c| !have.contains(c))
    else {
        report.skip("A: categories", "no category A does not have yet");
        return Ok(());
    };
    let added = sent(
        report,
        format!("A: addCategory {category}"),
        items.add_category(&f.a, &category).await,
    )?;
    let listed = a_fields(g, &["categories"]).await?.categories;
    check!(
        report,
        added && listed.contains(&category),
        "A: the category is listed",
        "changed: {added}, categories {listed:?}"
    );
    let removed = sent(
        report,
        "A: removeCategory",
        items.remove_category(&f.a, &category).await,
    )?;
    let listed = a_fields(g, &["categories"]).await?.categories;
    check!(
        report,
        removed && listed == have,
        "A: the categories as before",
        "changed: {removed}, categories {listed:?}"
    );
    Ok(())
}

async fn a_planning_constraints(report: &mut Report, g: &Gate) -> Result<(), String> {
    let (f, items) = (&g.fixtures, g.writer.items());
    let mut options = g
        .reader
        .items()
        .field_options(&f.a, "planningConstraints")
        .await
        .unwrap_or_default();
    if options.is_empty() {
        options = g
            .reader
            .schema()
            .field_options(&f.project, "planningConstraints")
            .await
            .unwrap_or_default();
    }
    let Some(constraint) = options
        .into_iter()
        .find(|o| !o.hidden && !o.id.is_empty())
        .map(|o| o.id)
    else {
        report.skip(
            "A: planning constraints",
            "the constraint options cannot be read",
        );
        return Ok(());
    };
    let date = "2001-02-03";
    let before = planning_constraints(&g.reader, &f.a).await?;
    let added = sent(
        report,
        format!("A: addPlaningContraint {constraint} {date}"),
        items.add_planning_constraint(&f.a, date, &constraint).await,
    )?;
    let listed = planning_constraints(&g.reader, &f.a).await?;
    check!(
        report,
        added
            && listed
                .iter()
                .any(|(d, c)| d.starts_with(date) && *c == constraint),
        "A: the planning constraint is listed",
        "changed: {added}, constraints {listed:?}"
    );
    let removed = sent(
        report,
        "A: removePlaningConstraint",
        items
            .remove_planning_constraint(&f.a, date, &constraint)
            .await,
    )?;
    let listed = planning_constraints(&g.reader, &f.a).await?;
    check!(
        report,
        removed && listed == before,
        "A: the planning constraints as before",
        "changed: {removed}, constraints {listed:?}"
    );
    Ok(())
}

async fn a_link(report: &mut Report, g: &Gate) -> Result<(), String> {
    let (f, items) = (&g.fixtures, g.writer.items());
    let to_b = |w: &WorkItem| {
        w.links
            .iter()
            .any(|l| l.role.as_deref() == Some(LINK_ROLE) && f.item(&l.item) == Some(Which::B))
    };
    let from_a = |links: &[polarion::Link]| links.iter().any(|l| f.item(&l.item) == Some(Which::A));
    if to_b(&a_fields(g, &["linkedWorkItems"]).await?) {
        report.skip("A: link to B", "A already links to B");
        return Ok(());
    }
    let added = sent(
        report,
        format!("A: addLinkedItem {LINK_ROLE} B"),
        items.add_link(&f.a, LINK_ROLE, &f.b).await,
    )?;
    let linked = to_b(&a_fields(g, &["linkedWorkItems"]).await?);
    let back = got("B's back links", g.reader.items().back_links(&f.b).await)?;
    check!(
        report,
        added && linked && from_a(&back),
        "A: the link to B is listed, and among B's back links",
        "changed: {added}, listed: {linked}, back link: {}",
        from_a(&back)
    );
    let removed = sent(
        report,
        "A: removeLinkedItem",
        items.remove_link(&f.a, LINK_ROLE, &f.b).await,
    )?;
    let linked = to_b(&a_fields(g, &["linkedWorkItems"]).await?);
    let back = got("B's back links", g.reader.items().back_links(&f.b).await)?;
    check!(
        report,
        removed && !linked && !from_a(&back),
        "A: the link to B is gone",
        "changed: {removed}, listed: {linked}"
    );
    Ok(())
}

async fn a_assignee(report: &mut Report, g: &Gate) -> Result<(), String> {
    let (f, items) = (&g.fixtures, g.writer.items());
    let before = a_fields(g, &["assignee"]).await?.assignees;
    if before.contains(&f.user) {
        report.skip("A: assignee", "the user is assigned already");
        return Ok(());
    }
    let added = sent(
        report,
        "A: addAssignee (the user)",
        items.add_assignee(&f.a, &f.user).await,
    )?;
    let listed = a_fields(g, &["assignee"]).await?.assignees;
    check!(
        report,
        added && listed.contains(&f.user),
        "A: the user is assigned",
        "changed: {added}, assignees {listed:?}"
    );
    let removed = sent(
        report,
        "A: removeAssignee",
        items.remove_assignee(&f.a, &f.user).await,
    )?;
    let listed = a_fields(g, &["assignee"]).await?.assignees;
    check!(
        report,
        removed && listed == before,
        "A: the assignees as before",
        "changed: {removed}, assignees {listed:?}"
    );
    Ok(())
}

async fn a_approval(report: &mut Report, g: &Gate) -> Result<(), String> {
    let (f, items) = (&g.fixtures, g.writer.items());
    let before = a_fields(g, &["approvals", "status"]).await?;
    let mine = |w: &WorkItem| {
        w.approvals
            .iter()
            .find(|a| a.user.as_deref() == Some(f.user.as_str()))
            .map(|a| a.status.clone())
    };
    if mine(&before).is_some() {
        report.skip("A: approval", "the user is an approver already");
        return Ok(());
    }
    let seen = mark(g).await?;
    sent(
        report,
        "A: addApprovee (the user)",
        items.add_approvee(&f.a, &f.user).await,
    )?;
    audit_sees(report, g, "approvals", &seen).await;
    let added = a_fields(g, &["approvals", "status"]).await?;
    check!(
        report,
        mine(&added).is_some(),
        "A: the user is an approver",
        "approvals {}",
        short(&to_json(&added.approvals))
    );
    sent(
        report,
        "A: editApproval (the user, waiting)",
        items.set_approval(&f.a, &f.user, "waiting").await,
    )?;
    let waiting = a_fields(g, &["approvals", "status"]).await?;
    check!(
        report,
        mine(&waiting) == Some(Some("waiting".to_owned())),
        "A: the user's approval is waiting",
        "approvals {}",
        short(&to_json(&waiting.approvals))
    );
    sent(
        report,
        "A: removeApprovee",
        items.remove_approvee(&f.a, &f.user).await,
    )?;
    let after = a_fields(g, &["approvals", "status"]).await?;
    check!(
        report,
        after.approvals == before.approvals,
        "A: the approvals as before",
        "approvals {}",
        short(&to_json(&after.approvals))
    );
    check!(
        report,
        added.status == before.status
            && waiting.status == before.status
            && after.status == before.status,
        "A: the status never changed",
        "status {:?}, then {:?}, {:?}, {:?}",
        before.status,
        added.status,
        waiting.status,
        after.status
    );
    Ok(())
}

async fn a_localized_title(report: &mut Report, g: &Gate, token: &str) -> Result<(), String> {
    let (f, items, reader) = (&g.fixtures, g.writer.items(), g.reader.items());
    let title = a_fields(g, &["title"]).await?.title.unwrap_or_default();
    let original = got(
        "A: localized title",
        reader.localized_title(&f.a, None).await,
    )?;
    report.info(format!(
        "the localized title in the default language is {}the title",
        if original == title { "" } else { "not " }
    ));
    let localized = format!("{MARKER_A} localized title {token}");
    sent(
        report,
        "A: setLocalizedWorkItemTitle (default language)",
        items
            .set_localized_title(&f.a, None, Some(&localized))
            .await,
    )?;
    let read = got(
        "A: localized title",
        reader.localized_title(&f.a, None).await,
    )?;
    check!(
        report,
        read == localized,
        "A: the localized title was written",
        "read {read:?}"
    );
    let restore = if original.is_empty() {
        None
    } else {
        Some(original.as_str())
    };
    sent(
        report,
        "A: localized title restored",
        items.set_localized_title(&f.a, None, restore).await,
    )?;
    let read = got(
        "A: localized title",
        reader.localized_title(&f.a, None).await,
    )?;
    let after = a_fields(g, &["title"]).await?.title.unwrap_or_default();
    check!(
        report,
        read == original && after == title,
        "A: the localized title and the title as before",
        "localized {read:?}, title {after:?}"
    );
    Ok(())
}

async fn a_attachments(report: &mut Report, g: &Gate, token: &str) -> Result<(), String> {
    let (f, items, reader) = (&g.fixtures, g.writer.items(), g.reader.items());
    let name = format!("{FILE_PREFIX}{token}.txt");
    let data = format!("{TEST_DATA} {token}: attachment\n").into_bytes();
    let seen = mark(g).await?;
    sent(
        report,
        format!("A: createAttachment {name}"),
        items
            .add_attachment(&f.a, &name, "polarion-rs live test attachment", &data)
            .await,
    )?;
    audit_sees(report, g, "attachments", &seen).await;
    let listed = a_fields(g, &["attachments"]).await?.attachments;
    let id = listed
        .iter()
        .filter(|a| a.file_name.as_deref() == Some(name.as_str()))
        .filter_map(|a| a.id.clone())
        .next_back()
        .ok_or_else(|| format!("{name} is not among A's attachments"))?;
    report.ok(format!(
        "A: the attachment is listed (id {id}, fileName {name})"
    ));
    let content = got("A: getAttachment", reader.attachment(&f.a, &id).await)?;
    check!(
        report,
        content == data,
        "A: getAttachment returns the content unchanged",
        "{} bytes read, {} written",
        content.len(),
        data.len()
    );
    let updated = format!("{TEST_DATA} {token}: attachment, updated\n").into_bytes();
    let title = "polarion-rs live test attachment (updated)";
    sent(
        report,
        "A: updateAttachment (title, content)",
        items
            .update_attachment(&f.a, &id, None, Some(title), Some(updated.as_slice()))
            .await,
    )?;
    let listed = a_fields(g, &["attachments"]).await?.attachments;
    let content = got("A: getAttachment", reader.attachment(&f.a, &id).await)?;
    check!(
        report,
        content == updated
            && listed
                .iter()
                .any(|a| a.id.as_deref() == Some(id.as_str()) && a.title.as_deref() == Some(title)),
        "A: the attachment was updated",
        "{} bytes, attachments {}",
        content.len(),
        short(&to_json(&listed))
    );
    sent(
        report,
        "A: deleteAttachment",
        items.delete_attachment(&f.a, &id).await,
    )?;
    let listed = a_fields(g, &["attachments"]).await?.attachments;
    check!(
        report,
        !listed.iter().any(|a| a.id.as_deref() == Some(id.as_str())),
        "A: the attachment is gone",
        "attachments {}",
        short(&to_json(&listed))
    );
    Ok(())
}

async fn a_work_record(report: &mut Report, g: &Gate) -> Result<(), String> {
    let (f, items) = (&g.fixtures, g.writer.items());
    // A work record the guard cannot delete (an unknown URI format) would stay on A.
    let samples = owner_samples(&g.reader, &f.project).await?;
    if check_owner_samples(
        report,
        "work records (before adding one)",
        &samples.work_records,
    ) != Some(true)
    {
        report.skip(
            "A: work record",
            "uri::owner_of is not confirmed on existing work record URIs",
        );
        return Ok(());
    }
    let fields = ["workRecords", "timeSpent", "remainingEstimate"];
    let before = a_fields(g, &fields).await?;
    let record = NewWorkRecord {
        user: f.user.clone(),
        date: date(now()),
        time_spent: "1h".into(),
        kind: None,
        comment: None,
    };
    let seen = mark(g).await?;
    match items.add_work_record(&f.a, &record).await {
        Ok(()) => report.ok("A: createWorkRecord (the user, 1h)"),
        // The user may lack the permission to create work records: nothing was created.
        Err(e) if e.to_string().contains("PermissionDeniedException") => {
            report.skip(
                "A: work record",
                format!("the user may not create work records: {e}"),
            );
            return Ok(());
        }
        Err(e) => return Err(format!("A: createWorkRecord (the user, 1h): {e}")),
    }
    audit_sees(report, g, "work records", &seen).await;
    let after = a_fields(g, &fields).await?;
    let new: Vec<&polarion::WorkRecord> = after
        .work_records
        .iter()
        .filter(|r| !before.work_records.contains(r))
        .collect();
    let [new] = new.as_slice() else {
        return Err(format!("expected one new work record, found {}", new.len()));
    };
    let uri = new.uri.clone().ok_or("the new work record has no URI")?;
    report.ok(format!("A: the work record is listed: {uri}"));
    if !check!(
        report,
        polarion::uri::owner_of(&uri).as_deref() == Some(f.a.uri.as_str()),
        "uri::owner_of on the new work record gives A",
        "parsed {:?}",
        polarion::uri::owner_of(&uri)
    ) {
        return Err("the work record cannot be deleted through the guard".into());
    }
    sent(
        report,
        "A: deleteWorkRecord",
        items.delete_work_record(&f.a, &uri).await,
    )?;
    let last = a_fields(g, &fields).await?;
    check!(
        report,
        last.work_records == before.work_records,
        "A: the work records as before",
        "{}",
        short(&to_json(&last.work_records))
    );
    check!(
        report,
        last.time_spent == before.time_spent
            && last.remaining_estimate == before.remaining_estimate,
        "A: timeSpent and remainingEstimate as before",
        "timeSpent {:?} (before {:?}), remainingEstimate {:?} (before {:?})",
        last.time_spent,
        before.time_spent,
        last.remaining_estimate,
        before.remaining_estimate
    );
    Ok(())
}

async fn a_markdown(report: &mut Report, g: &Gate, token: &str) -> Result<(), String> {
    let (f, items, reader) = (&g.fixtures, g.writer.items(), g.reader.items());
    let before = a_fields(g, &["description"]).await?.description;
    let options = MarkdownWriteOptions::default();
    let with_diagram = markdown_test_text(token, f.b.id.as_deref().unwrap_or_default());
    let diagrams = md::mermaid::diagrams(&with_diagram);
    let [diagram] = diagrams.as_slice() else {
        return Err(format!("expected one diagram, found {}", diagrams.len()));
    };
    let update = sent(
        report,
        "A: set_description_markdown with a mermaid diagram",
        items
            .set_description_markdown(&f.a, &with_diagram, &options)
            .await,
    )?;
    let attachment = update.diagrams.first().and_then(|d| d.attachment.clone());
    check!(
        report,
        update.updated
            && update.diagrams.len() == 1
            && update.diagrams[0].uploaded
            && attachment.is_some(),
        "A: the diagram was rendered and uploaded",
        "{}",
        short(&to_json(&update))
    );
    let id = attachment.ok_or("no diagram attachment")?;
    let svg = got(
        "A: getAttachment (the diagram)",
        reader.attachment(&f.a, &id).await,
    )?;
    let rendered =
        md::mermaid::render_svg(&diagram.source).map_err(|e| format!("rendering: {e}"))?;
    check!(
        report,
        svg == rendered.as_bytes(),
        "A: getAttachment returns the SVG unchanged",
        "{} bytes read, {} rendered",
        svg.len(),
        rendered.len()
    );
    let kept = a_fields(g, &["description"]).await?.description;
    let attachments: DiagramAttachments = [(diagram.file_name.clone(), id.clone())]
        .into_iter()
        .collect();
    let prefix =
        f.a.id
            .as_deref()
            .and_then(|i| i.rsplit_once('-'))
            .map(|(p, _)| p.to_owned())
            .unwrap_or_default();
    let expected = md::to_html_with_diagrams(
        &with_diagram,
        &ToHtmlOptions {
            work_items: WorkItemIds::Prefixes(vec![prefix]),
            highlight: true,
        },
        &attachments,
    );
    let kept = kept.map(|d| d.content).unwrap_or_default();
    check!(
        report,
        kept == expected,
        "A: the description keeps the HTML as written",
        "{}",
        first_difference(&expected, &kept)
    );
    let markdown = got(
        "A: description_markdown",
        reader
            .description_markdown(&f.a, &MarkdownReadOptions::default())
            .await,
    )?;
    check!(
        report,
        markdown == with_diagram,
        "A: the description reads back as the Markdown, the diagram from its SVG",
        "{}",
        first_difference(&with_diagram, &markdown)
    );
    let again = sent(
        report,
        "A: set_description_markdown with the same text",
        items
            .set_description_markdown(&f.a, &with_diagram, &options)
            .await,
    )?;
    check!(
        report,
        !again.updated && again.diagrams.iter().all(|d| !d.uploaded),
        "A: the same text again uploads and writes nothing",
        "{}",
        short(&to_json(&again))
    );
    let without = format!(
        "polarion-rs live test of the Markdown descriptions ({token}), without the diagram."
    );
    let pruned = sent(
        report,
        "A: set_description_markdown without the diagram",
        items
            .set_description_markdown(&f.a, &without, &options)
            .await,
    )?;
    let listed = a_fields(g, &["attachments"]).await?.attachments;
    check!(
        report,
        pruned.deleted == [id.clone()]
            && !listed.iter().any(|a| a.id.as_deref() == Some(id.as_str())),
        "A: the diagram attachment was pruned",
        "deleted {:?}, attachments {}",
        pruned.deleted,
        short(&to_json(&listed))
    );
    let appended = sent(
        report,
        "A: append_description_markdown",
        items
            .append_description_markdown(
                &f.a,
                &format!("Appended by the live suite ({token})."),
                &options,
            )
            .await,
    )?;
    let described = a_fields(g, &["description"]).await?.description;
    check!(
        report,
        appended.updated
            && described.as_ref().is_some_and(|d| {
                d.content.contains("Appended by the live suite")
                    && d.content.contains("without the diagram")
            }),
        "A: the paragraph was appended",
        "{}",
        short(&to_json(&described))
    );
    match &before {
        Some(text) => sent(
            report,
            "A: description restored",
            items
                .update(
                    &f.a,
                    &WorkItemUpdate {
                        description: Some(text.clone()),
                        ..WorkItemUpdate::default()
                    },
                )
                .await,
        )?,
        None => sent(
            report,
            "A: description cleared",
            items.clear_fields(&f.a, &["description".to_owned()]).await,
        )?,
    }
    let last = a_fields(g, &["description"]).await?.description;
    check!(
        report,
        last == before,
        "A: the description as before",
        "{}",
        short(&to_json(&last))
    );
    Ok(())
}

async fn b_and_r_records(report: &mut Report, g: &Gate, token: &str) -> Result<(), String> {
    let f = &g.fixtures;
    let (tests, read) = (g.writer.tests(), g.reader.tests());
    let columns: Vec<String> = got(
        "test step columns",
        read.steps_configuration(&f.project, Some(TYPE_B)).await,
    )?
    .into_iter()
    .map(|o| o.id)
    .collect();
    if columns.is_empty() {
        report.skip("B: test steps", "no test step columns configured");
        return Ok(());
    }
    let steps_before = got("B's test steps", read.steps(&f.b).await)?;
    let steps: Vec<Vec<Text>> = (1..=2)
        .map(|n| {
            columns
                .iter()
                .map(|c| html(format!("{TEST_DATA} {token}: step {n}, {c}")))
                .collect()
        })
        .collect();
    sent(
        report,
        "B: setTestSteps (2 steps)",
        tests.set_steps(&f.b, &steps).await,
    )?;
    let written = got("B's test steps", read.steps(&f.b).await)?;
    check!(
        report,
        written.steps.len() == 2
            && written
                .steps
                .iter()
                .flatten()
                .all(|cell| cell.content.contains(token)),
        "B: the test steps were written",
        "{}",
        short(&to_json(&written))
    );

    let run_before = got("R", read.get_by_uri(&f.r, None).await)?;
    let index = u32::try_from(run_before.records.len()).unwrap_or(u32::MAX);
    let at = index as usize;
    let record = TestRecord {
        test_case: Some(f.b.clone()),
        result: Some("passed".into()),
        comment: Some(plain(format!("{TEST_DATA} {token}: record"))),
        executed: Some(iso(now())),
        duration: Some(1.0),
        ..TestRecord::default()
    };
    sent(
        report,
        format!("R: addTestRecordToTestRun (B, passed) at {index}"),
        tests.add_record(&f.r, &record).await,
    )?;
    // Nothing is done at the index until it is this run's record: the server might not append.
    let run = got("R", read.get_by_uri(&f.r, None).await)?;
    our_record(f, &run, at, token)?;
    report.ok(format!("R: the new record is at {index}"));
    check!(
        report,
        run.records.get(at).is_some_and(|r| {
            r.test_case.as_ref().and_then(|t| f.item(t)) == Some(Which::B)
                && r.result.as_deref() == Some("passed")
        }),
        "R: the record of B is listed, passed",
        "records {}",
        short(&to_json(&run.records))
    );
    let updated = TestRecord {
        comment: Some(plain(format!("{TEST_DATA} {token}: record, updated"))),
        step_results: vec![
            TestStepResult {
                result: Some("passed".into()),
                comment: Some(plain("step passed")),
                attachments: Vec::new(),
            };
            steps.len()
        ],
        ..record.clone()
    };
    sent(
        report,
        "R: updateTestRecordAtIndex (step results passed)",
        tests.update_record(&f.r, index, &updated).await,
    )?;
    let run = got("R", read.get_by_uri(&f.r, None).await)?;
    check!(
        report,
        run.records.get(at).is_some_and(|r| {
            r.step_results.len() == steps.len()
                && r.step_results
                    .iter()
                    .all(|s| s.result.as_deref() == Some("passed"))
                && r.comment
                    .as_ref()
                    .is_some_and(|c| c.content.contains("updated"))
        }),
        "R: the record has the step results",
        "record {}",
        short(&to_json(&run.records.get(at)))
    );

    let step_file = format!("{FILE_PREFIX}{token}-step.txt");
    sent(
        report,
        format!("R: addAttachmentToTestStep {step_file}"),
        tests
            .add_step_attachment(
                &f.r,
                index,
                0,
                &step_file,
                "polarion-rs live test step attachment",
                b"step\n",
            )
            .await,
    )?;
    // The server stores it as `<test case id>_<step>_<file name>`.
    let step_name = |run: &polarion::TestRun| {
        run.records.get(at).and_then(|r| {
            r.step_results.first().and_then(|s| {
                s.attachments
                    .iter()
                    .filter_map(|a| a.file_name.clone())
                    .find(|n| server_name_of(n, &step_file))
            })
        })
    };
    let run = got("R", read.get_by_uri(&f.r, None).await)?;
    let Some(stored) = step_name(&run) else {
        return Err(format!(
            "R: the step attachment is not listed: record {}",
            short(&to_json(&run.records.get(at)))
        ));
    };
    report.ok(format!("R: the step attachment is listed as {stored}"));
    sent(
        report,
        format!("R: deleteAttachmentFromTestStep {stored}"),
        tests.delete_step_attachment(&f.r, index, 0, &stored).await,
    )?;
    let run = got("R", read.get_by_uri(&f.r, None).await)?;
    check!(
        report,
        step_name(&run).is_none(),
        "R: the step attachment is gone from the step",
        "it is still listed"
    );
    run_attachment_gone(report, g, &step_file).await?;

    let record_file = format!("{FILE_PREFIX}{token}-record.txt");
    sent(
        report,
        format!("R: addAttachmentToTestRecord {record_file}"),
        tests
            .add_record_attachment(
                &f.r,
                index,
                &record_file,
                "polarion-rs live test record attachment",
                b"record\n",
            )
            .await,
    )?;
    // The server stores it as `<test case id>_<file name>`.
    let record_name = |run: &polarion::TestRun| {
        run.records.get(at).and_then(|r| {
            r.attachments
                .iter()
                .filter_map(|a| a.file_name.clone())
                .find(|n| server_name_of(n, &record_file))
        })
    };
    let run = got("R", read.get_by_uri(&f.r, None).await)?;
    let Some(stored) = record_name(&run) else {
        return Err(format!(
            "R: the record attachment is not listed: record {}",
            short(&to_json(&run.records.get(at)))
        ));
    };
    report.ok(format!("R: the record attachment is listed as {stored}"));
    sent(
        report,
        format!("R: deleteAttachmentFromTestRecord {stored}"),
        tests.delete_record_attachment(&f.r, index, &stored).await,
    )?;
    let run = got("R", read.get_by_uri(&f.r, None).await)?;
    check!(
        report,
        record_name(&run).is_none(),
        "R: the record attachment is gone from the record",
        "it is still listed"
    );
    run_attachment_gone(report, g, &record_file).await?;

    let run = got("R", read.get_by_uri(&f.r, None).await)?;
    our_record(f, &run, at, token)?;
    sent(
        report,
        format!("R: updateTestRecordAtIndex {index} (remove)"),
        tests.remove_record(&f.r, index).await,
    )?;
    let run = got("R", read.get_by_uri(&f.r, None).await)?;
    check!(
        report,
        run.records == run_before.records,
        "R: the records as before",
        "records {}",
        short(&to_json(&run.records))
    );
    sent(
        report,
        "B: setTestSteps (restored)",
        set_steps_of_b(&g.writer, f, &steps_before.steps).await,
    )?;
    let restored = got("B's test steps", read.steps(&f.b).await)?;
    check!(
        report,
        restored == steps_before,
        "B: the test steps as before",
        "{}",
        short(&to_json(&restored))
    );
    Ok(())
}

/// The records of `run` that this run added (test case B, the token in the comment): exactly
/// one, at `at`; otherwise an error, and nothing is done at that index.
fn our_record(f: &Fixtures, run: &polarion::TestRun, at: usize, token: &str) -> Result<(), String> {
    let ours: Vec<usize> = run
        .records
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.test_case.as_ref().and_then(|t| f.item(t)) == Some(Which::B)
                && r.comment
                    .as_ref()
                    .is_some_and(|c| c.content.contains(token))
        })
        .map(|(i, _)| i)
        .collect();
    if ours == [at] {
        Ok(())
    } else {
        Err(format!(
            "this run's record is at {ours:?}, not at {at} only: nothing is done at {at}"
        ))
    }
}

async fn r_fields(report: &mut Report, g: &Gate, token: &str) -> Result<(), String> {
    let f = &g.fixtures;
    let (tests, read) = (g.writer.tests(), g.reader.tests());
    let before = got("R", read.get_by_uri(&f.r, None).await)?;
    let title = format!("{MARKER_R} title changed by the live suite {token}");
    let update = |title: Option<String>| TestRunUpdate {
        title,
        ..TestRunUpdate::default()
    };
    sent(
        report,
        "R: updateTestRun (title)",
        tests.update(&f.r, &update(Some(title.clone()))).await,
    )?;
    let changed = got("R", read.get_by_uri(&f.r, None).await)?;
    check!(
        report,
        changed.title.as_deref() == Some(title.as_str()),
        "R: the title was written (marker kept)",
        "title {:?}",
        changed.title
    );
    sent(
        report,
        "R: title restored",
        tests.update(&f.r, &update(before.title.clone())).await,
    )?;
    let after = got("R", read.get_by_uri(&f.r, None).await)?;
    check!(
        report,
        after.title == before.title,
        "R: the title as before",
        "title {:?}",
        after.title
    );

    let report_before = got("R's report", read.report(&f.r).await)?;
    let content = html(format!(
        "<p>{MARKER_R} report written by the live suite {token}</p>"
    ));
    sent(
        report,
        "R: updateWikiContentForTestRun",
        tests.set_report(&f.r, &content).await,
    )?;
    let written = got("R's report", read.report(&f.r).await)?;
    check!(
        report,
        written.content.contains(token),
        "R: the report was written",
        "report {}",
        cut(&written.content)
    );
    sent(
        report,
        "R: report restored",
        tests.set_report(&f.r, &report_before).await,
    )?;
    let restored = got("R's report", read.report(&f.r).await)?;
    check!(
        report,
        restored == report_before,
        "R: the report as before",
        "{}",
        first_difference(&report_before.content, &restored.content)
    );
    Ok(())
}

async fn r_attachments(report: &mut Report, g: &Gate, token: &str) -> Result<(), String> {
    let f = &g.fixtures;
    let (tests, read) = (g.writer.tests(), g.reader.tests());
    let name = format!("{FILE_PREFIX}{token}-run.txt");
    let title = "polarion-rs live test run attachment";
    sent(
        report,
        format!("R: addAttachmentToTestRun {name}"),
        tests
            .add_attachment(&f.r, &name, title, b"run attachment\n")
            .await,
    )?;
    let listed = got("R's attachments", read.attachments(&f.r).await)?;
    check!(
        report,
        listed
            .iter()
            .any(|a| a.file_name.as_deref() == Some(name.as_str())),
        "R: the attachment is listed",
        "attachments {}",
        short(&to_json(&listed))
    );
    let meta = got(
        "R: getTestRunAttachment",
        read.attachment(&f.r, &name).await,
    )?;
    report.ok(format!("R: getTestRunAttachment ({:?} bytes)", meta.length));
    let updated = "polarion-rs live test run attachment (updated)";
    sent(
        report,
        "R: updateTestRunAttachment (title, content)",
        tests
            .update_attachment(
                &f.r,
                &name,
                Some(updated),
                Some(b"run attachment, updated\n".as_slice()),
            )
            .await,
    )?;
    let listed = got("R's attachments", read.attachments(&f.r).await)?;
    check!(
        report,
        listed
            .iter()
            .any(|a| a.file_name.as_deref() == Some(name.as_str())
                && a.title.as_deref() == Some(updated)),
        "R: the attachment was updated",
        "attachments {}",
        short(&to_json(&listed))
    );
    sent(
        report,
        "R: deleteTestRunAttachment",
        tests.delete_attachment(&f.r, &name).await,
    )?;
    let listed = got("R's attachments", read.attachments(&f.r).await)?;
    check!(
        report,
        !listed
            .iter()
            .any(|a| a.file_name.as_deref() == Some(name.as_str())),
        "R: the attachment is gone",
        "attachments {}",
        short(&to_json(&listed))
    );
    Ok(())
}

/// The macros of a document body (`… id="polarion_wiki macro name=…"`), in order.
fn macros(body: &str) -> Vec<String> {
    body.match_indices("polarion_wiki macro")
        .map(|(i, _)| {
            let rest = &body[i..];
            rest[..rest.find('"').unwrap_or(rest.len())].to_owned()
        })
        .collect()
}

async fn d_body(
    report: &mut Report,
    g: &Gate,
    token: &str,
    wrote_d: &Cell<bool>,
) -> Result<(), String> {
    let f = &g.fixtures;
    let (docs, read) = (g.writer.documents(), g.reader.documents());
    let before = got("D", read.get_by_ref(&f.d, None).await)?;
    let body = before.body.clone().unwrap_or_else(|| html(""));
    let items_before = got(
        "D's work items",
        read.work_item_refs(&f.d, None, true).await,
    )?;
    let paragraph = format!("<p>{TEST_DATA} {token}: a paragraph appended to D.</p>");
    let extended = Text {
        content_type: body.content_type.clone(),
        content: format!("{}{paragraph}", body.content),
    };
    let update = |text: Text| DocumentUpdate {
        body: Some(text),
        ..DocumentUpdate::default()
    };
    let written = docs.update(&f.d, &update(extended)).await;
    // Sent (even when it failed, it may have been applied) unless a guard refused it.
    if !refused_before_sending(&written) {
        wrote_d.set(true);
    }
    sent(report, "D: updateModule (a paragraph appended)", written)?;
    let changed = got("D", read.get_by_ref(&f.d, None).await)?;
    let content = changed.body.map(|b| b.content).unwrap_or_default();
    check!(
        report,
        content.contains(token),
        "D: the paragraph is in the body",
        "body {}",
        cut(&content)
    );
    check!(
        report,
        macros(&content) == macros(&body.content),
        "D: every macro kept, none added",
        "before {:?}, now {:?}",
        macros(&body.content),
        macros(&content)
    );
    sent(
        report,
        "D: body restored",
        docs.update(&f.d, &update(body.clone())).await,
    )?;
    let after = got("D", read.get_by_ref(&f.d, None).await)?;
    let restored = after.body.unwrap_or_else(|| html(""));
    check!(
        report,
        restored.content == body.content,
        "D: the body is restored exactly",
        "{}",
        first_difference(&body.content, &restored.content)
    );
    let items_after = got(
        "D's work items",
        read.work_item_refs(&f.d, None, true).await,
    )?;
    check!(
        report,
        items_after == items_before,
        "D: its work items unchanged",
        "before {:?}, now {:?}",
        items_before
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        items_after
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
    Ok(())
}

async fn transactions(report: &mut Report, g: &Gate, token: &str) -> Result<(), String> {
    let (f, writer, items) = (&g.fixtures, &g.writer, g.writer.items());
    let title = a_fields(g, &["title"]).await?.title.unwrap_or_default();
    let set_title = |title: &str| WorkItemUpdate {
        title: Some(title.to_owned()),
        ..WorkItemUpdate::default()
    };

    sent(report, "beginTransaction", writer.begin_transaction().await)?;
    let rolled_back = format!("{MARKER_A} rolled back by the live suite {token}");
    let changed = items.update(&f.a, &set_title(&rolled_back)).await;
    let ended = writer.rollback().await;
    sent(report, "A: title changed inside the transaction", changed)?;
    sent(report, "rollback", ended)?;
    let open = got("transactionExists", writer.in_transaction().await)?;
    if open {
        return Err("a transaction is still open after the rollback".into());
    }
    let after = a_fields(g, &["title"]).await?.title.unwrap_or_default();
    check!(
        report,
        after == title,
        "A: the title is unchanged after the rollback",
        "title {after:?}"
    );

    sent(report, "beginTransaction", writer.begin_transaction().await)?;
    let committed = format!("{MARKER_A} committed by the live suite {token}");
    let changed = items.update(&f.a, &set_title(&committed)).await;
    let ended = if changed.is_ok() {
        writer.commit().await
    } else {
        writer.rollback().await
    };
    sent(
        report,
        "A: title changed inside the second transaction",
        changed,
    )?;
    sent(report, "commit", ended)?;
    let open = got("transactionExists", writer.in_transaction().await)?;
    let after = a_fields(g, &["title"]).await?.title.unwrap_or_default();
    check!(
        report,
        after == committed && !open,
        "A: the committed title is there, no transaction is open",
        "title {after:?}, transaction open: {open}"
    );
    sent(
        report,
        "A: title restored",
        items.update(&f.a, &set_title(&title)).await,
    )?;
    let after = a_fields(g, &["title"]).await?.title.unwrap_or_default();
    check!(
        report,
        after == title,
        "A: the title as before",
        "title {after:?}"
    );
    Ok(())
}

// ==================================================================== permanent

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: writes comments, which cannot be deleted, to the fixtures (see docs/testing.md)"]
async fn permanent() {
    let _tier = one_tier("permanent");
    let mut report = Report::default();
    // Read-only, before any gate writes: the opt-in for today, no permanent run before, and no
    // comment of the suite on A or D from the last day.
    if let Err(e) = permanent_allowed(&mut report).await {
        report.fail("permanent tier", e);
        report.finish("permanent");
        return;
    }
    if let Some(g) = write_gate(&mut report).await {
        let result = comments(&mut report, &g).await;
        if g.after(&mut report, "comments", result).await {
            if index_probe(&mut report, &g).await {
                report.ok("the index shows the run's last change");
            }
            g.after(&mut report, "index probe", Ok(())).await;
        } else {
            wait_for_index(&mut report, &g).await;
        }
        audit(&mut report, &g, false).await;
        g.close(&mut report).await;
    }
    report.finish("permanent");
}

/// Whether the permanent tier may run (read-only): `POLARION_LIVE_PERMANENT` is today's key
/// for these fixtures ([`permanent_key`], printed by the read tier), the log of permanent runs
/// names none for A, and A and D have no comment of the suite from the last day (an
/// unreadable date counts as recent).
async fn permanent_allowed(report: &mut Report) -> Result<(), String> {
    let settings = Settings::from_env();
    let ids = FixtureIds::from_env()?;
    let key = permanent_key(&ids);
    match env("POLARION_LIVE_PERMANENT") {
        Some(given) if given == key => {}
        given => {
            return Err(format!(
                "POLARION_LIVE_PERMANENT is {given:?}: set it to {key} to write the comments of \
                 the permanent tier today (they cannot be deleted; the tier runs once)"
            ));
        }
    }
    let log = permanent_log_path(&settings)?;
    if let Some(earlier) = permanent_runs(&log)?
        .into_iter()
        .find(|entry| entry["a"] == ids.a.as_str())
    {
        return Err(format!(
            "{} records a permanent run for {} already ({}): it runs once; remove that line \
             only to run it again on purpose",
            log.display(),
            ids.a,
            short(&earlier)
        ));
    }
    let reader = reader(&settings);
    let result = recent_comments(&reader, &settings, &ids).await;
    report.result("logout (reader)", reader.logout().await);
    match result? {
        0 => {
            report.ok("permanent tier: allowed today, not run before, no recent suite comment");
            Ok(())
        }
        n => Err(format!(
            "{n} comments of the suite on A or D from the last day: the tier ran already"
        )),
    }
}

/// The lines of the log of permanent runs (none: no file).
fn permanent_runs(log: &std::path::Path) -> Result<Vec<Value>, String> {
    let text = match std::fs::read_to_string(log) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", log.display())),
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l)
                .map_err(|e| format!("{}: not a log line ({e}): {l}", log.display()))
        })
        .collect()
}

/// Record the permanent run before its first comment is written.
fn log_permanent_run(settings: &Settings, f: &Fixtures) -> Result<(), String> {
    use std::io::Write as _;
    let log = permanent_log_path(settings)?;
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let id = f.a.id.clone().unwrap_or_default();
    let line = serde_json::json!({"a": id, "uri": f.a.uri, "started": iso(now())});
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .and_then(|mut file| writeln!(file, "{line}"))
        .map_err(|e| format!("cannot append to {}: {e}", log.display()))
}

/// Comments of the suite on A and D from the last day (an unreadable date counts).
async fn recent_comments(
    reader: &Polarion,
    settings: &Settings,
    ids: &FixtureIds,
) -> Result<usize, String> {
    let f = Fixtures::resolve(reader, &settings.project, ids).await?;
    let a = got("A", reader.items().get(&f.a).await)?;
    let d = got("D", reader.documents().get_by_ref(&f.d, None).await)?;
    let recent = |c: &&polarion::Comment| {
        c.text
            .as_ref()
            .is_some_and(|t| t.content.contains("polarion-rs live suite"))
            && c.created
                .as_deref()
                .and_then(parse_iso)
                .is_none_or(|t| t > now() - 86_400)
    };
    Ok(a.comments.iter().filter(recent).count() + d.comments.iter().filter(recent).count())
}

/// Whether a comment URI is among `comments`.
fn listed(comments: &[polarion::Comment], uri: &str) -> Option<polarion::Comment> {
    comments
        .iter()
        .find(|c| c.uri.as_deref().map(base_uri) == Some(base_uri(uri)))
        .cloned()
}

/// Each write at most once, never retried: a comment on A, a reply, resolved and tags; a comment
/// on D (without `about`). The run is logged before the first comment.
async fn comments(report: &mut Report, g: &Gate) -> Result<(), String> {
    let (f, items) = (&g.fixtures, g.writer.items());
    let token = run_token();
    log_permanent_run(&g.settings, f)?;
    report.ok("permanent tier: the run is logged");
    let seen = mark(g).await?;
    let text = plain(format!(
        "{MARKER_A} Comment written by the polarion-rs live suite ({token}). Comments cannot be \
         deleted: kept on purpose."
    ));
    let comment = sent(
        report,
        "A: addComment",
        items
            .add_comment(&f.a, Some("polarion-rs live test comment"), &text)
            .await,
    )?;
    report.info(format!("comment {comment}"));
    audit_sees(report, g, "comments", &seen).await;
    let owned = check!(
        report,
        polarion::uri::owner_of(&comment).as_deref() == Some(f.a.uri.as_str()),
        "uri::owner_of on the new comment gives A",
        "parsed {:?}: the reply, resolve and tags are refused by the guard",
        polarion::uri::owner_of(&comment)
    );
    let a = got("A", g.reader.items().get(&f.a).await)?;
    check!(
        report,
        listed(&a.comments, &comment).is_some(),
        "A: the comment is listed",
        "comments {}",
        short(&to_json(&a.comments))
    );
    if owned {
        let reply_text = plain(format!(
            "Reply written by the polarion-rs live suite ({token})."
        ));
        let reply = sent(
            report,
            "A: addComment (reply)",
            items.reply_to_comment(&f.a, &comment, &reply_text).await,
        )?;
        let a = got("A", g.reader.items().get(&f.a).await)?;
        check!(
            report,
            listed(&a.comments, &reply)
                .is_some_and(|r| r.parent.as_deref().map(base_uri) == Some(base_uri(&comment))),
            "A: the reply is listed under the comment",
            "comments {}",
            short(&to_json(&a.comments))
        );
        sent(
            report,
            "A: setResolvedComment",
            items.set_comment_resolved(&f.a, &comment, true).await,
        )?;
        let resolved = got(
            "isResolvedComment",
            g.reader.items().is_comment_resolved(&comment).await,
        )?;
        check!(report, resolved, "A: the comment is resolved", "it is not");
        let tag = env("POLARION_LIVE_COMMENT_TAG");
        let tags: Vec<&str> = tag.as_deref().into_iter().collect();
        sent(
            report,
            format!("A: setCommentTags {tags:?}"),
            items.set_comment_tags(&f.a, &comment, &tags).await,
        )?;
        let a = got("A", g.reader.items().get(&f.a).await)?;
        let read = listed(&a.comments, &comment)
            .map(|c| c.tags)
            .unwrap_or_default();
        check!(
            report,
            read == tags,
            "A: the comment's tags",
            "tags {read:?}, expected {tags:?}"
        );
    }
    let d_text = plain(format!(
        "{MARKER_D} Comment written by the polarion-rs live suite ({token}). Comments cannot be \
         deleted: kept on purpose."
    ));
    let d_comment = sent(
        report,
        "D: addComment",
        g.writer.documents().add_comment(&f.d, None, &d_text).await,
    )?;
    report.info(format!("comment {d_comment}"));
    check!(
        report,
        polarion::uri::owner_of(&d_comment).as_deref() == Some(f.d.uri.as_str()),
        "uri::owner_of on D's comment gives D",
        "parsed {:?}",
        polarion::uri::owner_of(&d_comment)
    );
    let d = got("D", g.reader.documents().get_by_ref(&f.d, None).await)?;
    check!(
        report,
        listed(&d.comments, &d_comment).is_some(),
        "D: the comment is listed",
        "comments {}",
        short(&to_json(&d.comments))
    );
    Ok(())
}

// ==================================================================== offline checks

// These run with every `cargo test`: no server, no environment, only the suite's own logic.

/// Fixtures for the offline checks.
/// The mermaid source of the Markdown step.
const MARKDOWN_DIAGRAM: &str = "graph TD\n    A[polarion-rs] --> B[live test]";

/// The Markdown the write tier sets as A's description: every construct whose HTML Polarion must
/// keep as written (a heading, the spans of bold, italic, strike, underline, highlight, sup/sub,
/// code and formulas, a link, a work item link to B, a list, an alert, a table, a highlighted
/// code block and a diagram), in canonical form so that it reads back exactly.
fn markdown_test_text(token: &str, b: &str) -> String {
    format!(
        "polarion-rs live test of the Markdown descriptions ({token}).\n\n\
         ## Formatting\n\n\
         **bold**, *italic*, ~~strike~~, ++underline++, ==highlight==, ^sup^, ~sub~, `code`, \
         $E = mc^2$ and a [link](https://example.invalid/polarion-rs-live/{token}).\n\n\
         The test case {b}.\n\n\
         - one\n\
         - two\n\n\
         > [!NOTE]\n\
         > A note.\n\n\
         | a | b |\n\
         | --- | --: |\n\
         | 1 | 2 |\n\n\
         ```rust\n\
         fn main() {{}}\n\
         ```\n\n\
         ```mermaid\n\
         {MARKDOWN_DIAGRAM}\n\
         ```"
    )
}

fn offline_fixtures() -> Fixtures {
    Fixtures {
        project: "P".into(),
        user: "jdoe".into(),
        a: ItemRef::new("P", "P-1"),
        b: ItemRef::new("P", "P-2"),
        r: polarion::uri::test_run_uri("P", "R"),
        r_id: "R".into(),
        d: DocumentRef::new("P", DEFAULT_SPACE, DOCUMENT_NAME),
    }
}

/// A state as the bootstrap leaves the fixtures (A with its creation values).
fn clean_state() -> Value {
    let text = |content: String| serde_json::json!({"type": "text/html", "content": content});
    serde_json::json!({
        "a": {
            "title": fixture_title(MARKER_A),
            "description": text(fixture_description(MARKER_A, "work item")),
            "customFields": {
                "risk": 7,
                "ratio": 2.5,
                "due": "2001-02-03",
                "note": format!("{TEST_DATA} bootstrap"),
            },
        },
        "aLocalizedTitle": fixture_title(MARKER_A),
        "b": {
            "title": fixture_title(MARKER_B),
            "description": text(fixture_description(MARKER_B, "work item")),
        },
        "bSteps": {"steps": []},
        "r": {"title": fixture_title(MARKER_R), "records": []},
        "rReport": text(String::new()),
        "d": {"body": text(format!(
            "<div>{}</div>",
            fixture_description(MARKER_D, "document").replace(' ', "&nbsp;")
        ))},
    })
}

#[test]
fn a_state_as_the_bootstrap_made_it_can_be_the_baseline() {
    let problems = baseline_problems(&offline_fixtures(), &clean_state());
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn what_a_stopped_run_leaves_behind_is_never_the_baseline() {
    use serde_json::json;
    type Change = Box<dyn Fn(&mut Value)>;
    let token = "prsabc";
    let changes: Vec<(&str, Change)> = vec![
        (
            "A's title",
            Box::new(move |s| {
                s["a"]["title"] = json!(format!(
                    "{MARKER_A} title changed by the live suite {token}"
                ))
            }),
        ),
        (
            "A's description",
            Box::new(|s| s["a"]["description"]["content"] = json!("<p>x</p>")),
        ),
        (
            "the localized title",
            Box::new(move |s| {
                s["aLocalizedTitle"] = json!(format!("{MARKER_A} localized title {token}"))
            }),
        ),
        (
            "a hyperlink",
            Box::new(move |s| {
                s["a"]["hyperlinks"] =
                    json!([{"url": format!("{TEST_URL}{token}"), "role": "ref_ext"}])
            }),
        ),
        (
            "an attachment",
            Box::new(move |s| {
                s["b"]["attachments"] =
                    json!([{"id": "1", "fileName": format!("{FILE_PREFIX}{token}.txt")}])
            }),
        ),
        (
            "a diagram",
            Box::new(|s| {
                s["a"]["attachments"] =
                    json!([{"id": "2", "fileName": "mermaid-0123.svg", "title": DIAGRAM_TITLE}])
            }),
        ),
        (
            "a work record",
            Box::new(|s| s["a"]["workRecords"] = json!([{"uri": "x", "user": "jdoe"}])),
        ),
        (
            "an approval",
            Box::new(|s| s["a"]["approvals"] = json!([{"user": "jdoe", "status": "waiting"}])),
        ),
        (
            "an assignment",
            Box::new(|s| s["b"]["assignees"] = json!(["jdoe"])),
        ),
        (
            "a link",
            Box::new(|s| {
                s["a"]["links"] =
                    json!([{"role": LINK_ROLE, "item": {"project": "P", "id": "P-2"}}])
            }),
        ),
        (
            "a planning constraint",
            Box::new(|s| {
                s["a"]["planningConstraints"] =
                    json!([["2001-02-03T00:00:00.000+01:00", "mustStartOn"]])
            }),
        ),
        (
            "a second test value",
            Box::new(|s| s["a"]["customFields"]["risk"] = json!(8)),
        ),
        (
            "a second test date",
            Box::new(|s| s["a"]["customFields"]["due"] = json!("2001-02-04")),
        ),
        (
            "a test value with a token",
            Box::new(move |s| {
                s["a"]["customFields"]["note"] = json!(format!("{TEST_DATA} {token}"))
            }),
        ),
        (
            "test steps",
            Box::new(
                move |s| s["bSteps"]["steps"] = json!([[{"type": "text/html", "content": format!("{TEST_DATA} {token}: step 1, step")}]]),
            ),
        ),
        (
            "a record",
            Box::new(|s| s["r"]["records"] = json!([{"result": "passed"}])),
        ),
        (
            "R's report",
            Box::new(move |s| {
                s["rReport"]["content"] = json!(format!(
                    "<p>{MARKER_R} report written by the live suite {token}</p>"
                ))
            }),
        ),
        ("R's title", Box::new(|s| s["r"]["title"] = json!(MARKER_R))),
        (
            "an attachment of R",
            Box::new(|s| {
                s["r"]["attachments"] = json!([{"fileName": format!("{FILE_PREFIX}x-run.txt")}])
            }),
        ),
        (
            "D's paragraph",
            Box::new(move |s| {
                let body = s["d"]["body"]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                s["d"]["body"]["content"] =
                    json!(format!("{body}<p>{TEST_DATA} {token}: a paragraph</p>"))
            }),
        ),
        (
            "D's body",
            Box::new(|s| s["d"]["body"]["content"] = json!("<p>other</p>")),
        ),
    ];
    let f = offline_fixtures();
    for (what, change) in changes {
        let mut state = clean_state();
        change(&mut state);
        assert!(
            !baseline_problems(&f, &state).is_empty(),
            "{what} is not found"
        );
    }
}

#[test]
fn dates_of_planning_constraints_are_written_as_days() {
    assert_eq!(day_of("2001-02-03T00:00:00.000+01:00"), "2001-02-03");
    assert_eq!(day_of("2001-02-03"), "2001-02-03");
    assert_eq!(day_of("03.02.2001"), "03.02.2001");
    assert_eq!(day_of("x"), "x");
}

#[test]
fn state_files_are_never_in_target() {
    let target = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target"));
    assert!(inside(&target.join("live-baseline.json"), &target));
    assert!(inside(
        &target.join("debug").join("..").join("baseline.json"),
        &target
    ));
    let elsewhere =
        std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../x/b.json"));
    assert!(!inside(&elsewhere, &target));
    assert!(!inside(
        &target.join("..").join("target-other").join("b.json"),
        &target
    ));
}

#[test]
fn records_are_used_only_at_the_index_they_were_found() {
    let f = offline_fixtures();
    let record = |case: &ItemRef, comment: &str| TestRecord {
        test_case: Some(case.clone()),
        comment: Some(plain(comment)),
        ..TestRecord::default()
    };
    let run = |records: Vec<TestRecord>| polarion::TestRun {
        records,
        ..polarion::TestRun::default()
    };
    let other = ItemRef::new("P", "P-9");
    let ours = record(&f.b, "polarion-rs live test data prsabc: record");
    assert!(our_record(&f, &run(vec![ours.clone()]), 0, "prsabc").is_ok());
    // Inserted elsewhere, twice, or another test case's.
    assert!(
        our_record(
            &f,
            &run(vec![ours.clone(), record(&other, "x")]),
            1,
            "prsabc"
        )
        .is_err()
    );
    assert!(our_record(&f, &run(vec![ours.clone(), ours.clone()]), 1, "prsabc").is_err());
    assert!(our_record(&f, &run(vec![record(&other, "prsabc")]), 0, "prsabc").is_err());
    assert!(our_record(&f, &run(Vec::new()), 0, "prsabc").is_err());
}

#[test]
fn the_permanent_key_is_for_today_and_the_fixture() {
    let ids = FixtureIds {
        a: "P-1".into(),
        b: "P-2".into(),
        run: "R".into(),
        space: DEFAULT_SPACE.into(),
        name: DOCUMENT_NAME.into(),
        user: "jdoe".into(),
    };
    assert_eq!(permanent_key(&ids), format!("{}:P-1", date(now())));
}

#[test]
fn record_and_step_attachments_are_recognized_under_the_server_names() {
    let file = format!("{FILE_PREFIX}abc-step.txt");
    for stored in [
        file.clone(),
        format!("PRJ-19060_1_{file}"),
        format!("PRJ-19060_{file}"),
    ] {
        assert!(is_suite_file(&stored), "{stored}");
        assert!(server_name_of(&stored, &file), "{stored}");
    }
    for other in [
        "report.pdf".to_owned(),
        format!("x {FILE_PREFIX}abc.txt"),
        format!("_{FILE_PREFIX}abc.txt"),
        format!("a/b_{FILE_PREFIX}abc.txt"),
    ] {
        assert!(!is_suite_file(&other), "{other}");
    }
    assert!(!server_name_of(
        &format!("PRJ-1_{FILE_PREFIX}other.txt"),
        &file
    ));
}

#[test]
fn the_markdown_of_the_write_tier_reads_back_exactly() {
    let text = markdown_test_text("tok", "PRJ-2");
    let diagram = &md::mermaid::diagrams(&text)[0];
    let id = format!("1-{}", diagram.file_name);
    let attachments: DiagramAttachments = [(diagram.file_name.clone(), id.clone())]
        .into_iter()
        .collect();
    let ids = WorkItemIds::Prefixes(vec!["PRJ".into()]);
    let html = md::to_html_with_diagrams(
        &text,
        &ToHtmlOptions {
            work_items: ids.clone(),
            highlight: true,
        },
        &attachments,
    );
    assert!(html.contains("data-item-id=\"PRJ-2\""), "{html}");
    let sources: DiagramSources = [(id, diagram.source.clone())].into_iter().collect();
    let back = md::to_markdown_with_diagrams(
        &html,
        &ToMarkdownOptions {
            work_items: ids,
            ..ToMarkdownOptions::default()
        },
        &sources,
    );
    assert_eq!(back, text);
}

#[test]
fn only_blank_lines_next_to_headings_are_the_known_heading_limitation() {
    let html = "<span style=\"font-weight: bold; font-size: 11pt; line-height: 1.5;\">Head</span>\
                <br/>Text<br/><span style=\"font-weight: bold; font-size: 11pt; line-height: 1.5;\">\
                Next</span><br/>More";
    let options = ToHtmlOptions::default();
    let markdown = md::to_markdown(html, &ToMarkdownOptions::default());
    let back = md::to_html(&markdown, &options);
    assert_ne!(visual::render(html), visual::render(&back), "{back}");
    assert!(only_blank_lines_at_headings(html, &back), "{back}");
    // Any other difference is not the known limitation.
    let coloured = "<span style=\"color: #ff0000;\">Text</span>";
    assert!(!only_blank_lines_at_headings(coloured, "Text"));
    assert!(!only_blank_lines_at_headings("a<br/><br/>b", "a<br/>b"));
    assert!(!only_blank_lines_at_headings("a<br/>b", "a<br/><br/>b"));
}
