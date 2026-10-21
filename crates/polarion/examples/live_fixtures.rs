//! The bootstrap of the live suite: creates its fixtures once and prints the variables the live
//! tests need.
//!
//! ```sh
//! POLARION_URL=https://polarion.example.com/polarion \
//! POLARION_TOKEN=<personal access token> \
//! POLARION_PROJECT=<project of the fixtures> \
//! POLARION_LIVE_CREATE=1 \
//! POLARION_LIVE_WRITES=polarion.example.com \
//! cargo run -p polarion --example live_fixtures
//! ```
//!
//! The fixtures (see docs/testing.md):
//! - A: a `userstory` with one value for each custom field type the type defines;
//! - B: a `testcase`;
//! - R: test run `POLARION_RS_LIVE` from the template `Default` (`POLARION_LIVE_TEMPLATE`);
//! - D: document `polarion-rs live test fixture` in the space `_default`
//!   (`POLARION_LIVE_SPACE`), created without a title so no title heading work item is added.
//!
//! Their titles start with a marker (`[polarion-rs live test fixture A]`, ...), and their
//! descriptions say that they are kept on purpose.
//!
//! What it does:
//! - Nothing without `POLARION_LIVE_CREATE=1` and `POLARION_LIVE_WRITES` set to the host of
//!   `POLARION_URL`.
//! - With `POLARION_LIVE_ITEMS`, `POLARION_LIVE_TEST_RUN` or `POLARION_LIVE_DOCUMENT` set, it
//!   only verifies the fixtures they name and never creates anything.
//! - Otherwise it searches first, read-only: the fixtures it created before (its log
//!   `live-created.jsonl`, next to the baseline outside `target/`), work items by their title
//!   (and `author.id`, once the user is known) and, with a query that does not depend on how
//!   the server searches titles, the newest work items of each type with the marker matched in
//!   code; the test run id, the document location. More than one candidate for a fixture, or a
//!   fixture only one of the two work item searches finds, stops it.
//! - It creates only what is missing, with a client that may create exactly that many work
//!   items, test runs and documents in the project and write nothing else. After each create it
//!   waits until the index finds the new fixture.
//! - At the first error after a create was sent it stops: search for the fixtures before running
//!   it again (a create that timed out may have been applied).
//! - It lists D's work items (a work item created on the side is reported), verifies every
//!   fixture like the live tests' gate does, stores the baseline when there is none (only when
//!   the fixtures carry nothing a stopped run leaves behind; the live tests never store one),
//!   and prints the lines for the environment (`POLARION_LIVE_ITEMS`, `POLARION_LIVE_TEST_RUN`,
//!   `POLARION_LIVE_DOCUMENT`, `POLARION_LIVE_USER`: the author of A, the token's user).

#[macro_use]
#[path = "../tests/live_support.rs"]
mod live_support;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use live_support::*;
use polarion::{
    CreateBudget, Document, DocumentRef, Error, FieldDefinition, FieldValue, Fields, ItemRef,
    NewDocument, NewTestRun, NewWorkItem, Polarion, Query, TestRun, WorkItem,
};
use serde_json::{Value, json};

#[tokio::main]
async fn main() -> ExitCode {
    let settings = Settings::from_env();
    if env("POLARION_LIVE_CREATE").as_deref() != Some("1") {
        eprintln!(
            "POLARION_LIVE_CREATE is not set: set it to 1 to run the bootstrap (it creates the \
             fixtures that are missing)"
        );
        return ExitCode::FAILURE;
    }
    if let Err(e) = check_live_writes(&settings) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    let mut report = Report::default();
    let reader = reader(&settings);
    println!(
        "live fixtures in project {} on {}",
        settings.project, settings.host
    );
    let ids = if FixtureIds::any_set() {
        println!("POLARION_LIVE_* are set: verifying the fixtures they name, nothing is created");
        match FixtureIds::from_env() {
            Ok(ids) => Some(ids),
            Err(e) => {
                report.fail("POLARION_LIVE_*", e);
                None
            }
        }
    } else {
        match bootstrap(&mut report, &settings, &reader).await {
            Ok(ids) => Some(ids),
            Err(e) => {
                report.fail("bootstrap", e);
                None
            }
        }
    };
    if let Some(ids) = &ids {
        verify(&mut report, &settings, &reader, ids).await;
    }
    report.result("logout (reader)", reader.logout().await);
    if let Some(ids) = &ids {
        println!(
            "\nThe fixtures, for the environment of the live tests:\n{}",
            ids.env_lines()
        );
    }
    if report.summary("live fixtures") {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// What the search found (or the bootstrap created).
#[derive(Debug, Default)]
struct Found {
    user: Option<String>,
    a: Option<WorkItem>,
    b: Option<WorkItem>,
    r: Option<TestRun>,
    d: Option<Document>,
}

/// Where the fixtures go.
#[derive(Debug)]
struct Place {
    project: String,
    space: String,
    template: String,
}

/// Search, then create what is missing; the ids of all four fixtures.
async fn bootstrap(
    report: &mut Report,
    settings: &Settings,
    reader: &Polarion,
) -> Result<FixtureIds, String> {
    let place = Place {
        project: settings.project.clone(),
        space: env("POLARION_LIVE_SPACE").unwrap_or_else(|| DEFAULT_SPACE.to_owned()),
        template: env("POLARION_LIVE_TEMPLATE").unwrap_or_else(|| DEFAULT_TEMPLATE.to_owned()),
    };
    let log = created_log_path(settings)?;
    let mut found = search(report, reader, &place, &log).await?;
    let budget = CreateBudget {
        work_items: u32::from(found.a.is_none()) + u32::from(found.b.is_none()),
        test_runs: u32::from(found.r.is_none()),
        documents: u32::from(found.d.is_none()),
    };
    if budget == CreateBudget::default() {
        report.ok("all four fixtures exist: nothing to create");
    } else {
        println!(
            "creating in {}: {} work items, {} test run, {} document",
            place.project, budget.work_items, budget.test_runs, budget.documents
        );
        let writer = Polarion::builder(&settings.url, settings.auth())
            .danger_accept_invalid_certs(settings.insecure)
            .allow_creates(&place.project, budget)
            .build()
            .map_err(|e| format!("cannot build the creating client: {e}"))?;
        if !writer.writes_restricted()
            || writer.write_targets().next().is_some()
            || writer.writable_projects().next().is_some()
            || writer.global_writes_allowed()
        {
            return Err("the creating client is not restricted to its create budget".into());
        }
        let mut sent = false;
        let created =
            create_missing(report, reader, &writer, &place, &log, &mut found, &mut sent).await;
        report.result("logout (creating client)", writer.logout().await);
        if let Err(e) = created {
            return Err(if sent {
                format!(
                    "{e}\nA create was sent before this error and may have been applied. Search \
                     {} for work items titled `[polarion-rs live test fixture …`, the test run \
                     {RUN_ID} and the document {}/{DOCUMENT_NAME} before running the bootstrap \
                     again.",
                    place.project, place.space
                )
            } else {
                e
            });
        }
    }
    let id = |item: &Option<WorkItem>, label: &str| {
        item.as_ref()
            .and_then(|w| w.id.clone())
            .ok_or_else(|| format!("fixture {label} has no id"))
    };
    Ok(FixtureIds {
        a: id(&found.a, "A")?,
        b: id(&found.b, "B")?,
        run: RUN_ID.to_owned(),
        space: place.space.clone(),
        name: DOCUMENT_NAME.to_owned(),
        user: found
            .user
            .clone()
            .ok_or("the user (the author of A) is unknown")?,
    })
}

// ---------------------------------------------------------------- search

/// The log of what earlier runs created ([`created_log_path`]), one JSON object per line
/// (`{"fixture", "uri"}`): the search finds these by URI even when the index does not.
fn read_created_log(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let mut log = BTreeMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let entry: Value = serde_json::from_str(line)
            .map_err(|e| format!("{}: not a log line ({e}): {line}", path.display()))?;
        match (entry["fixture"].as_str(), entry["uri"].as_str()) {
            (Some(fixture), Some(uri)) => {
                if let Some(earlier) = log.insert(fixture.to_owned(), uri.to_owned()) {
                    return Err(format!(
                        "{} names two {fixture}: {earlier} and {uri}; keep one, set \
                         POLARION_LIVE_* to it",
                        path.display()
                    ));
                }
            }
            _ => return Err(format!("{}: not a log line: {line}", path.display())),
        }
    }
    Ok(log)
}

/// Append a created fixture to the log, right after the create returned.
fn log_created(path: &Path, fixture: &str, uri: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let line = json!({"fixture": fixture, "uri": uri, "created": iso(now())});
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| writeln!(file, "{line}"))
        .map_err(|e| format!("cannot append to {}: {e}", path.display()))
}

/// A Lucene term: bare if simple, quoted otherwise.
fn lucene_term(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        value.to_owned()
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// The title search of the fixtures of `kind`.
fn title_lucene(kind: &str) -> String {
    format!("type:{kind} AND title:\"polarion-rs live test fixture\"")
}

/// The work items of `kind` the title search finds, whose title starts with `marker`.
async fn by_title(
    reader: &Polarion,
    project: &str,
    kind: &str,
    marker: &str,
) -> Result<Vec<WorkItem>, String> {
    marked_items(reader, project, &title_lucene(kind), marker).await
}

/// The work items of `kind` found by `lucene` whose title starts with `marker` (matched in code).
async fn marked_items(
    reader: &Polarion,
    project: &str,
    lucene: &str,
    marker: &str,
) -> Result<Vec<WorkItem>, String> {
    let query = Query::new(lucene)
        .project(project)
        .limit(100)
        .fields(Fields::new(["id", "title", "type", "author", "created"]));
    let candidates = reader
        .items()
        .query(&query)
        .await
        .map_err(|e| format!("searching `{lucene}`: {e}"))?;
    if candidates.len() >= 100 {
        return Err(format!("`{lucene}` finds 100 or more work items"));
    }
    Ok(candidates
        .into_iter()
        .filter(|w| w.title.as_deref().is_some_and(|t| t.starts_with(marker)))
        .collect())
}

/// The newest 100 work items of `kind` (by `author.id` when the user is known), whose title
/// starts with `marker` (matched in code): a search that does not depend on how the server
/// searches titles.
async fn newest_marked(
    reader: &Polarion,
    project: &str,
    kind: &str,
    marker: &str,
    user: Option<&str>,
) -> Result<Vec<WorkItem>, String> {
    let lucene = match user {
        Some(user) => format!("type:{kind} AND author.id:{}", lucene_term(user)),
        None => format!("type:{kind}"),
    };
    let query = Query::new(lucene.as_str())
        .project(project)
        .sort("~created")
        .limit(100)
        .fields(Fields::new(["id", "title", "type", "author", "created"]));
    Ok(reader
        .items()
        .query(&query)
        .await
        .map_err(|e| format!("searching `{lucene}`: {e}"))?
        .into_iter()
        .filter(|w| w.title.as_deref().is_some_and(|t| t.starts_with(marker)))
        .collect())
}

/// The work item of `kind` whose title is exactly the fixture's title (`None`: there is none),
/// matched in code, by `author.id` when the user is known. A work item whose title only starts
/// with the marker (a run that stopped in a step may have left it changed), one by another
/// author, one that only the search of the newest work items ([`newest_marked`]) finds, or
/// more than one candidate stop the bootstrap, so no second fixture is created.
async fn find_item(
    reader: &Polarion,
    project: &str,
    kind: &str,
    marker: &str,
    user: Option<&str>,
) -> Result<Option<WorkItem>, String> {
    let title = fixture_title(marker);
    let all = by_title(reader, project, kind, marker).await?;
    let lucene = title_lucene(kind);
    // The title search may find nothing because the server reads the phrase differently: the
    // newest work items of the type must not hold a fixture it missed.
    let newest = newest_marked(reader, project, kind, marker, user).await?;
    let missed: Vec<(Option<String>, Option<String>)> = newest
        .iter()
        .filter(|n| !all.iter().any(|w| w.id == n.id))
        .map(|w| (w.id.clone(), w.title.clone()))
        .collect();
    if !missed.is_empty() {
        return Err(format!(
            "work items with a title starting with `{marker}` that `{lucene}` does not find: \
             {missed:?}; check them, and set POLARION_LIVE_* to the fixtures instead of creating \
             another one"
        ));
    }
    let candidates = match user {
        Some(user) => {
            let by_user = format!("{lucene} AND author.id:{}", lucene_term(user));
            let mine = marked_items(reader, project, &by_user, marker).await?;
            // The author term only narrows the search: what it leaves out is no fixture to
            // create again either.
            if mine.len() != all.len() {
                return Err(format!(
                    "work items with a title starting with `{marker}` not found by `author.id:\
                     {user}`: {:?}; check them before running the bootstrap again",
                    all.iter()
                        .filter(|w| !mine.iter().any(|m| m.id == w.id))
                        .map(|w| (w.id.clone(), w.author.clone()))
                        .collect::<Vec<_>>()
                ));
            }
            mine
        }
        None => all,
    };
    let (mut exact, marked): (Vec<WorkItem>, Vec<WorkItem>) = candidates
        .into_iter()
        .partition(|w| w.title.as_deref() == Some(title.as_str()));
    let ids = |list: &[WorkItem]| list.iter().map(|w| w.id.clone()).collect::<Vec<_>>();
    match (exact.len(), marked.len()) {
        (1, 0) => Ok(exact.pop()),
        (0, 0) => Ok(None),
        (0, 1) => Err(format!(
            "{:?} starts with `{marker}` but is not titled `{title}` (a run that stopped in a \
             step?): set POLARION_LIVE_* to the fixtures instead of creating another one",
            ids(&marked)
        )),
        _ => Err(format!(
            "more than one work item has a title starting with `{marker}`: {:?} {:?}; keep one, \
             and set POLARION_LIVE_* to the fixtures",
            ids(&exact),
            ids(&marked)
        )),
    }
}

/// The fixture test run (`None`: it does not exist).
async fn find_run(reader: &Polarion, project: &str) -> Result<Option<TestRun>, String> {
    let query = Query::new(format!("id:{RUN_ID}"))
        .project(project)
        .limit(10);
    let mut exact: Vec<TestRun> = reader
        .tests()
        .search(&query)
        .await
        .map_err(|e| format!("searching test run {RUN_ID}: {e}"))?
        .into_iter()
        .filter(|r| r.id.as_deref() == Some(RUN_ID))
        .collect();
    if exact.len() > 1 {
        return Err(format!("{} test runs have the id {RUN_ID}", exact.len()));
    }
    if let Some(run) = exact.pop() {
        return Ok(Some(run));
    }
    // Not in the index (yet): by id.
    match reader.tests().get(project, RUN_ID, None).await {
        Ok(run) => Ok(Some(run)),
        Err(Error::NotFound { .. }) => Ok(None),
        Err(e) => Err(format!("looking up test run {RUN_ID}: {e}")),
    }
}

/// The fixture document (`None`: it does not exist).
async fn find_document(reader: &Polarion, place: &Place) -> Result<Option<Document>, String> {
    match reader
        .documents()
        .get(&place.project, &place.space, DOCUMENT_NAME, true)
        .await
    {
        Ok(doc) => Ok(Some(doc)),
        Err(Error::NotFound { .. }) => Ok(None),
        Err(e) => Err(format!(
            "looking up document {}/{DOCUMENT_NAME}: {e}",
            place.space
        )),
    }
}

/// Search every fixture (read-only), the logged ones by URI.
async fn search(
    report: &mut Report,
    reader: &Polarion,
    place: &Place,
    log: &Path,
) -> Result<Found, String> {
    let log = read_created_log(log)?;
    let project = place.project.as_str();
    let mut found = Found {
        user: env("POLARION_LIVE_USER"),
        ..Found::default()
    };
    found.a = match log.get("A") {
        Some(uri) => Some(logged_item(reader, uri, MARKER_A).await?),
        None => find_item(reader, project, TYPE_A, MARKER_A, found.user.as_deref()).await?,
    };
    if found.user.is_none() {
        found.user = found.a.as_ref().and_then(|a| a.author.clone());
    }
    found.b = match log.get("B") {
        Some(uri) => Some(logged_item(reader, uri, MARKER_B).await?),
        None => find_item(reader, project, TYPE_B, MARKER_B, found.user.as_deref()).await?,
    };
    found.r = match log.get("R") {
        Some(uri) => Some(
            reader
                .tests()
                .get_by_uri(uri, None)
                .await
                .map_err(|e| format!("R, logged as created ({uri}): {e}"))?,
        ),
        None => find_run(reader, project).await?,
    };
    found.d = match log.get("D") {
        Some(uri) => Some(
            reader
                .documents()
                .get_by_ref(&DocumentRef::from_uri(uri.as_str()), None)
                .await
                .map_err(|e| format!("D, logged as created ({uri}): {e}"))?,
        ),
        None => find_document(reader, place).await?,
    };
    let state = |exists: bool, what: String| {
        if exists {
            format!("found {what}")
        } else {
            "missing".to_owned()
        }
    };
    report.info(format!(
        "A: {}\nB: {}\nR: {}\nD: {}\nuser: {:?}",
        state(
            found.a.is_some(),
            format!("{:?}", found.a.as_ref().and_then(|w| w.id.clone()))
        ),
        state(
            found.b.is_some(),
            format!("{:?}", found.b.as_ref().and_then(|w| w.id.clone()))
        ),
        state(found.r.is_some(), RUN_ID.to_owned()),
        state(
            found.d.is_some(),
            format!("{}/{DOCUMENT_NAME}", place.space)
        ),
        found.user
    ));
    Ok(found)
}

/// A work item the log names: it must exist and carry the fixture's marker.
async fn logged_item(reader: &Polarion, uri: &str, marker: &str) -> Result<WorkItem, String> {
    let item = reader
        .items()
        .get(&ItemRef::from_uri(uri))
        .await
        .map_err(|e| format!("{marker}, logged as created ({uri}): {e}"))?;
    if !item.title.as_deref().is_some_and(|t| t.starts_with(marker)) {
        return Err(format!(
            "{uri}, logged as created, is titled {:?}, without `{marker}`",
            item.title
        ));
    }
    Ok(item)
}

// ---------------------------------------------------------------- create

/// The values A is created with: one for each custom field type `userstory` defines (fields
/// that depend on another field and types without a safe value are skipped), and every required
/// field.
async fn creation_values(
    report: &mut Report,
    reader: &Polarion,
    project: &str,
) -> Result<BTreeMap<String, FieldValue>, String> {
    let defs: BTreeMap<String, FieldDefinition> = reader
        .schema()
        .custom_fields(project, TYPE_A)
        .await
        .map_err(|e| format!("custom fields of {TYPE_A}: {e}"))?
        .into_iter()
        .map(|d| (d.id.clone(), d))
        .collect();
    let mut wanted: Vec<&FieldDefinition> = Vec::new();
    for def in representatives(&defs) {
        if def.depends_on.is_some() && !def.required {
            report.skip(
                format!("A at creation: {} ({})", def.id, type_key(def)),
                "it depends on another field",
            );
        } else {
            wanted.push(def);
        }
    }
    for def in defs.values() {
        if def.required && !wanted.iter().any(|w| w.id == def.id) {
            wanted.push(def);
        }
    }
    let mut values = BTreeMap::new();
    for def in wanted {
        let label = format!("A at creation: {} ({})", def.id, type_key(def));
        match test_value(reader, project, def, None, None, "bootstrap").await? {
            Some(value) => {
                report.info(format!("{label} = {}", short(&json!(value))));
                values.insert(def.id.clone(), value);
            }
            None => report.skip(label, "no safe value for this type"),
        }
    }
    Ok(values)
}

/// Poll until `probe` finds the fixture; a timeout is an error.
async fn wait_until<F, Fut>(what: &str, probe: F) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool, String>>,
{
    match wait_for(INDEX_TIMEOUT, probe).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!(
            "{what}: not found after {} s",
            INDEX_TIMEOUT.as_secs()
        )),
        Err(e) => Err(format!("{what}: {e}")),
    }
}

/// Create the missing fixtures, in the order A, B, R, D. `sent` is set before the first create
/// is sent.
async fn create_missing(
    report: &mut Report,
    reader: &Polarion,
    writer: &Polarion,
    place: &Place,
    log: &Path,
    found: &mut Found,
    sent: &mut bool,
) -> Result<(), String> {
    let project = place.project.as_str();
    if found.a.is_none() {
        let values = creation_values(report, reader, project).await?;
        let new = NewWorkItem {
            description: Some(html(fixture_description(MARKER_A, "work item"))),
            custom_fields: values.clone(),
            ..NewWorkItem::new(project, TYPE_A, fixture_title(MARKER_A))
        };
        *sent = true;
        let created = writer
            .items()
            .create(&new)
            .await
            .map_err(|e| format!("creating A: {e}"))?;
        log_created(log, "A", &created.uri)?;
        report.ok(format!("created A: {created} ({})", created.uri));
        let id = created.id.clone();
        let id = id.as_deref();
        wait_until("the index finds A by its title", move || async move {
            Ok(by_title(reader, project, TYPE_A, MARKER_A)
                .await?
                .iter()
                .any(|w| w.id.as_deref() == id))
        })
        .await?;
        report.ok("the index finds A by its title");
        let a = reader
            .items()
            .get(&created)
            .await
            .map_err(|e| format!("reading A: {e}"))?;
        match &found.user {
            Some(user) if a.author.as_deref() != Some(user.as_str()) => {
                return Err(format!(
                    "A's author is {:?}, POLARION_LIVE_USER is {user}",
                    a.author
                ));
            }
            Some(_) => {}
            None => found.user = a.author.clone(),
        }
        // createWorkItem honours the custom fields given at creation.
        let fields = values
            .keys()
            .fold(Fields::new(["id"]), |fields, key| fields.with_custom(key));
        let stored = reader
            .items()
            .get_fields(&created, &fields)
            .await
            .map_err(|e| format!("reading A's custom fields: {e}"))?
            .custom_fields;
        for (key, value) in &values {
            check!(
                report,
                same_value(value, stored.get(key)),
                format!("A: the custom field {key} given at creation is stored"),
                "given {}, stored {}",
                short(&json!(value)),
                short(&json!(stored.get(key)))
            );
        }
        found.a = Some(a);
    }
    if found.b.is_none() {
        let new = NewWorkItem {
            description: Some(html(fixture_description(MARKER_B, "work item"))),
            ..NewWorkItem::new(project, TYPE_B, fixture_title(MARKER_B))
        };
        *sent = true;
        let created = writer
            .items()
            .create(&new)
            .await
            .map_err(|e| format!("creating B: {e}"))?;
        log_created(log, "B", &created.uri)?;
        report.ok(format!("created B: {created} ({})", created.uri));
        let id = created.id.clone();
        let id = id.as_deref();
        wait_until("the index finds B by its title", move || async move {
            Ok(by_title(reader, project, TYPE_B, MARKER_B)
                .await?
                .iter()
                .any(|w| w.id.as_deref() == id))
        })
        .await?;
        report.ok("the index finds B by its title");
        found.b = Some(
            reader
                .items()
                .get(&created)
                .await
                .map_err(|e| format!("reading B: {e}"))?,
        );
    }
    if found.r.is_none() {
        *sent = true;
        let uri = writer
            .tests()
            .create(&NewTestRun {
                title: Some(fixture_title(MARKER_R)),
                ..NewTestRun::new(project, RUN_ID, &place.template)
            })
            .await
            .map_err(|e| format!("creating R: {e}"))?;
        log_created(log, "R", &uri)?;
        report.ok(format!("created R: {uri}"));
        let query = Query::new(format!("id:{RUN_ID}"))
            .project(project)
            .limit(10);
        let query = &query;
        wait_until("the index finds R", move || async move {
            Ok(reader
                .tests()
                .search(query)
                .await
                .map_err(|e| e.to_string())?
                .iter()
                .any(|r| r.id.as_deref() == Some(RUN_ID)))
        })
        .await?;
        report.ok("the index finds R");
        let run = reader
            .tests()
            .get_by_uri(&uri, None)
            .await
            .map_err(|e| format!("reading R: {e}"))?;
        check!(
            report,
            run.title.as_deref() == Some(fixture_title(MARKER_R).as_str()),
            "R: the title given at creation is stored",
            "title {:?}",
            run.title
        );
        found.r = Some(run);
    }
    if found.d.is_none() {
        let new = NewDocument {
            project: project.to_owned(),
            space: Some(place.space.clone()),
            name: DOCUMENT_NAME.to_owned(),
            // No title: no title heading work item is created.
            title: None,
            allowed_types: vec![TYPE_A.to_owned()],
            structure_link_role: "parent".to_owned(),
            body: Some(fixture_description(MARKER_D, "document")),
            ..NewDocument::default()
        };
        *sent = true;
        let created = writer
            .documents()
            .create(&new)
            .await
            .map_err(|e| format!("creating D: {e}"))?;
        log_created(log, "D", &created.uri)?;
        report.ok(format!("created D: {created} ({})", created.uri));
        let uri = created.uri.as_str();
        // The 20 most recently updated documents (the sort the read tier uses), not every
        // document of the project.
        let query = Query::new("").project(project).sort("~updated").limit(20);
        let query = &query;
        wait_until("the index finds D", move || async move {
            Ok(reader
                .documents()
                .query_refs(query)
                .await
                .map_err(|e| e.to_string())?
                .iter()
                .any(|d| base_uri(&d.uri) == base_uri(uri)))
        })
        .await?;
        report.ok("the index finds D");
        found.d = Some(
            reader
                .documents()
                .get_by_ref(&created, None)
                .await
                .map_err(|e| format!("reading D: {e}"))?,
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- verify

/// The checks of the live tests' gate (read-only), D's work items, and the baseline (stored when
/// there is none).
async fn verify(report: &mut Report, settings: &Settings, reader: &Polarion, ids: &FixtureIds) {
    let f = match Fixtures::resolve(reader, &settings.project, ids).await {
        Ok(f) => f,
        Err(e) => {
            report.fail("fixtures", e);
            return;
        }
    };
    report.info(f.describe());
    match reader.documents().work_item_refs(&f.d, None, true).await {
        Ok(list) if list.is_empty() => report.ok("D holds no work items"),
        Ok(list) => report.fail(
            "D holds work items (created on the side?)",
            format!(
                "{:?}",
                list.iter().map(ToString::to_string).collect::<Vec<_>>()
            ),
        ),
        Err(e) => report.fail("D's work items", e),
    }
    let identical = check_identity(report, reader, &f).await;
    match back_links(reader, &f).await {
        Ok((_, foreign)) => {
            check!(
                report,
                foreign.is_empty(),
                "the back links of A and B come from fixtures only",
                "other items link to the fixtures: {foreign:?}"
            );
        }
        Err(e) => report.fail("back links", e),
    }
    if !identical {
        report.skip("baseline", "the fixtures did not pass the checks");
        return;
    }
    match load_baseline(settings, &f) {
        Ok(Some(_)) => report.ok(format!(
            "a baseline exists: {}",
            baseline_path(settings).map_or_else(|e| e, |p| p.display().to_string())
        )),
        Ok(None) => {
            let stored = match definitions(reader, &f).await {
                Ok(defs) => match snapshot(reader, &f, &defs).await {
                    Ok(state) => store_baseline(settings, &f, &state),
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            };
            match stored {
                Ok(path) => report.ok(format!("baseline stored in {}", path.display())),
                Err(e) => report.fail("baseline", e),
            }
        }
        Err(e) => report.fail("baseline", e),
    }
}
