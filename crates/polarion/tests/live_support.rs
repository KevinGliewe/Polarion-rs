//! Shared code of the live suite against a real server: the tiers of `tests/live.rs`, the
//! bootstrap `examples/live_fixtures.rs` and the front-end tiers of `polarion-cli` and
//! `polarion mcp`, which include this file with `#[path]` (it is no test of its own).
//!
//! The suite writes only to four fixtures that the bootstrap creates once: work items A
//! (`userstory`) and B (`testcase`), test run R and document D. Before anything is written,
//! [`write_gate`] checks, in this order:
//! 1. `POLARION_LIVE_WRITES` is the host of `POLARION_URL`;
//! 2. (after reading the fixtures) the writer is restricted to the URIs the server returned for
//!    A, B, R and D, all in `POLARION_PROJECT`, and nothing else: no project writes, no creates;
//! 3. each fixture is what the bootstrap made: its title marker, `author` is
//!    `POLARION_LIVE_USER`, its assignees and approvers are only that user, A and B are in no
//!    plan and no document, and D holds no work items;
//! 4. the back links of A and B come from fixtures only (compared again by the audit);
//! 5. the canary: the writer does not allow the newest other work item, allows each fixture,
//!    and refuses the harmless `removeHyperlink` on the canary before anything is sent;
//! 6. the stored baseline ([`baseline_path`], outside `target/`; stored only by the bootstrap,
//!    after checking that the fixtures carry nothing a run leaves behind, and never
//!    overwritten) exists, and every difference from it can be undone by the suite;
//! 7. the identity probe, the first write: A's title is changed and restored, and A's history
//!    must show both changes by `POLARION_LIVE_USER`, so the token belongs to that user before
//!    any write names the user. The date of the first change is the run's start for the audit
//!    (the server's clock, not this machine's);
//! 8. the fixtures are brought back to the baseline, so an aborted run repairs itself.
//!
//! Reads always go through a separate read-only client; the writer's `soap()` is never called.
//! Results are printed as `OK`, `SKIP` and `FAIL` lines.

#![allow(dead_code, unused_macros)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::future::Future;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use polarion::{
    Auth, Change, DocumentRef, DocumentUpdate, EnumOption, Error, FieldDefinition, FieldValue,
    Fields, ItemRef, Polarion, Query, TestRunUpdate, Text, WorkItem, WorkItemUpdate,
};
use serde_json::{Value, json};

/// An `OK` or `FAIL` line for a condition, with the reason formatted like `format!`; the value
/// of the condition.
macro_rules! check {
    ($report:expr, $cond:expr, $label:expr, $($why:tt)+) => {
        $report.check($label, $cond, format_args!($($why)+))
    };
}

// ---------------------------------------------------------------- fixtures

/// Title markers: every fixture's title starts with one, and a test that changes a title keeps it.
pub const MARKER_A: &str = "[polarion-rs live test fixture A]";
pub const MARKER_B: &str = "[polarion-rs live test fixture B]";
pub const MARKER_R: &str = "[polarion-rs live test fixture R]";
/// D has no title (so creating it adds no title heading work item); its body carries the marker.
pub const MARKER_D: &str = "[polarion-rs live test fixture D]";
const TITLE_REST: &str = " automated tests, do not edit";

pub const TYPE_A: &str = "userstory";
pub const TYPE_B: &str = "testcase";
pub const RUN_ID: &str = "POLARION_RS_LIVE";
pub const DEFAULT_TEMPLATE: &str = "Default";
pub const DEFAULT_SPACE: &str = "_default";
pub const DOCUMENT_NAME: &str = "polarion-rs live test fixture";
/// The link role of the link A → B.
pub const LINK_ROLE: &str = "relates_to";
/// Hyperlinks the suite adds start with this; restoring removes them.
pub const TEST_URL: &str = "https://example.invalid/polarion-rs-live/";
/// The URL of the only write attempted on the canary (it must be refused before it is sent).
pub const CANARY_URL: &str = "https://example.invalid/polarion-rs-canary";
/// File names of the attachments the suite adds start with this; restoring deletes them.
pub const FILE_PREFIX: &str = "polarion-rs-live-";
/// Text of the values the suite writes.
pub const TEST_DATA: &str = "polarion-rs live test data";
/// The title of the diagram attachments the Markdown writes add.
pub const DIAGRAM_TITLE: &str = "Mermaid diagram";

/// The title a fixture is created with.
pub fn fixture_title(marker: &str) -> String {
    format!("{marker}{TITLE_REST}")
}

/// The description a fixture is created with: what it is, and that it is kept on purpose.
pub fn fixture_description(marker: &str, what: &str) -> String {
    format!(
        "<p>{marker} This {what} is a fixture of the polarion-rs live test suite. The suite \
         changes it and restores it on every run. It is kept on purpose: work items, test runs \
         and comments cannot be deleted. Please do not edit, assign, link, plan or transition \
         it.</p>"
    )
}

pub fn html(content: impl Into<String>) -> Text {
    Text {
        content_type: "text/html".into(),
        content: content.into(),
    }
}

pub fn plain(content: impl Into<String>) -> Text {
    Text {
        content_type: "text/plain".into(),
        content: content.into(),
    }
}

/// Work item A or B.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    A,
    B,
}

impl Which {
    pub fn label(self) -> &'static str {
        match self {
            Which::A => "A",
            Which::B => "B",
        }
    }

    pub fn marker(self) -> &'static str {
        match self {
            Which::A => MARKER_A,
            Which::B => MARKER_B,
        }
    }

    /// The key of its state in a snapshot.
    fn key(self) -> &'static str {
        match self {
            Which::A => "a",
            Which::B => "b",
        }
    }
}

// ---------------------------------------------------------------- environment

/// A variable's value, trimmed; `None` when it is not set or blank.
pub fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// A variable that must be set: panics with what to set.
pub fn require(name: &str, what: &str) -> String {
    env(name).unwrap_or_else(|| panic!("{name} is not set: set it to {what}"))
}

/// The connection: `POLARION_URL`, `POLARION_TOKEN`, `POLARION_PROJECT`, `POLARION_INSECURE`.
pub struct Settings {
    pub url: String,
    /// The host of `POLARION_URL`, lower case.
    pub host: String,
    token: String,
    pub project: String,
    pub insecure: bool,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("url", &self.url)
            .field("project", &self.project)
            .field("insecure", &self.insecure)
            .finish_non_exhaustive()
    }
}

impl Settings {
    /// From the environment; panics with what to set when a variable is missing.
    pub fn from_env() -> Self {
        let url = require(
            "POLARION_URL",
            "the server's base URL, e.g. https://polarion.example.com/polarion",
        );
        let host = host_of(&url).unwrap_or_else(|| panic!("POLARION_URL `{url}` has no host name"));
        Self {
            token: require(
                "POLARION_TOKEN",
                "a personal access token of the user the suite runs as",
            ),
            project: require("POLARION_PROJECT", "the project of the fixtures"),
            insecure: env("POLARION_INSECURE").as_deref() == Some("1"),
            url,
            host,
        }
    }

    pub fn auth(&self) -> Auth {
        Auth::Token(self.token.clone())
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

/// The host of a URL, lower case (`https://user@Host:8443/x` → `host`).
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next()?,
        None => host.split(':').next()?,
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The value of `POLARION_LIVE_PERMANENT` that allows the permanent tier today (UTC) for these
/// fixtures: `YYYY-MM-DD:A-ID` (the read tier prints it), so a variable left set allows no
/// later run.
pub fn permanent_key(ids: &FixtureIds) -> String {
    format!("{}:{}", date(now()), ids.a)
}

/// Gate 1: writes only when `POLARION_LIVE_WRITES` names the host of `POLARION_URL`.
pub fn check_live_writes(settings: &Settings) -> Result<(), String> {
    match env("POLARION_LIVE_WRITES") {
        Some(host) if host.eq_ignore_ascii_case(&settings.host) => Ok(()),
        Some(host) => Err(format!(
            "POLARION_LIVE_WRITES is `{host}`, not the host of POLARION_URL (`{}`): nothing is \
             written",
            settings.host
        )),
        None => Err(format!(
            "POLARION_LIVE_WRITES is not set: set it to {} (the host of POLARION_URL) to allow \
             writes to the fixtures",
            settings.host
        )),
    }
}

/// The fixtures as the bootstrap prints them: `POLARION_LIVE_ITEMS=A-ID,B-ID`,
/// `POLARION_LIVE_TEST_RUN`, `POLARION_LIVE_DOCUMENT=SPACE/NAME`, `POLARION_LIVE_USER`.
#[derive(Debug, Clone)]
pub struct FixtureIds {
    pub a: String,
    pub b: String,
    pub run: String,
    pub space: String,
    pub name: String,
    pub user: String,
}

const FIXTURE_VARS: [&str; 3] = [
    "POLARION_LIVE_ITEMS",
    "POLARION_LIVE_TEST_RUN",
    "POLARION_LIVE_DOCUMENT",
];

impl FixtureIds {
    /// Whether any of the fixture variables is set.
    pub fn any_set() -> bool {
        FIXTURE_VARS.iter().any(|v| env(v).is_some())
    }

    pub fn from_env() -> Result<Self, String> {
        let missing = |name: &str| {
            format!(
                "{name} is not set: run the bootstrap (`cargo run -p polarion --example \
                 live_fixtures`, see docs/testing.md) and set the variables it prints"
            )
        };
        let items = env("POLARION_LIVE_ITEMS").ok_or_else(|| missing("POLARION_LIVE_ITEMS"))?;
        let (a, b) = items
            .split_once(',')
            .map(|(a, b)| (a.trim().to_owned(), b.trim().to_owned()))
            .filter(|(a, b)| !a.is_empty() && !b.is_empty() && !b.contains(','))
            .ok_or_else(|| format!("POLARION_LIVE_ITEMS must be `A-ID,B-ID`, not `{items}`"))?;
        let run = env("POLARION_LIVE_TEST_RUN").ok_or_else(|| missing("POLARION_LIVE_TEST_RUN"))?;
        let document =
            env("POLARION_LIVE_DOCUMENT").ok_or_else(|| missing("POLARION_LIVE_DOCUMENT"))?;
        let (space, name) = document
            .rsplit_once('/')
            .filter(|(s, n)| !s.is_empty() && !n.is_empty())
            .ok_or_else(|| {
                format!("POLARION_LIVE_DOCUMENT must be `SPACE/NAME`, not `{document}`")
            })?;
        let user = env("POLARION_LIVE_USER").ok_or_else(|| missing("POLARION_LIVE_USER"))?;
        Ok(Self {
            a,
            b,
            run,
            space: space.to_owned(),
            name: name.to_owned(),
            user,
        })
    }

    /// The lines to put in the environment.
    pub fn env_lines(&self) -> String {
        format!(
            "POLARION_LIVE_ITEMS={},{}\nPOLARION_LIVE_TEST_RUN={}\nPOLARION_LIVE_DOCUMENT={}/{}\n\
             POLARION_LIVE_USER={}",
            self.a, self.b, self.run, self.space, self.name, self.user
        )
    }
}

/// The fixtures, by the URIs the server returned for them (without `%revision`).
#[derive(Debug, Clone)]
pub struct Fixtures {
    pub project: String,
    /// `POLARION_LIVE_USER`: the user the token belongs to (the author of the fixtures).
    pub user: String,
    pub a: ItemRef,
    pub b: ItemRef,
    /// The URI of R.
    pub r: String,
    pub r_id: String,
    pub d: DocumentRef,
}

impl Fixtures {
    /// Read the fixtures named by `ids` (read-only) and keep the URIs the server returns.
    pub async fn resolve(
        reader: &Polarion,
        project: &str,
        ids: &FixtureIds,
    ) -> Result<Self, String> {
        let mut items = Vec::new();
        for (label, id) in [("A", &ids.a), ("B", &ids.b)] {
            let item = reader
                .items()
                .get(&ItemRef::new(project, id))
                .await
                .map_err(|e| format!("fixture {label} ({id}): {e}"))?;
            let uri = item
                .uri
                .ok_or_else(|| format!("fixture {label} ({id}): the server returned no URI"))?;
            items.push(ItemRef::from_uri(base_uri(&uri)));
        }
        let run = reader
            .tests()
            .get(project, &ids.run, None)
            .await
            .map_err(|e| format!("fixture R ({}): {e}", ids.run))?;
        let r = run
            .uri
            .ok_or("fixture R: the server returned no URI")
            .map(|u| base_uri(&u).to_owned())?;
        let document = reader
            .documents()
            .get(project, &ids.space, &ids.name, true)
            .await
            .map_err(|e| format!("fixture D ({}/{}): {e}", ids.space, ids.name))?;
        let d = document
            .uri
            .ok_or("fixture D: the server returned no URI")
            .map(|u| DocumentRef::from_uri(base_uri(&u)))?;
        let b = items.pop().expect("B");
        let a = items.pop().expect("A");
        Ok(Self {
            project: project.to_owned(),
            user: ids.user.clone(),
            a,
            b,
            r,
            r_id: ids.run.clone(),
            d,
        })
    }

    pub fn uris(&self) -> [&str; 4] {
        [
            self.a.uri.as_str(),
            self.b.uri.as_str(),
            self.r.as_str(),
            self.d.uri.as_str(),
        ]
    }

    pub fn get(&self, which: Which) -> &ItemRef {
        match which {
            Which::A => &self.a,
            Which::B => &self.b,
        }
    }

    /// Whether `uri` (without `%revision`) is one of the fixtures.
    pub fn is_fixture(&self, uri: &str) -> bool {
        self.uris().contains(&base_uri(uri))
    }

    /// Which fixture work item `item` is, by URI or by project and id.
    pub fn item(&self, item: &ItemRef) -> Option<Which> {
        let same = |f: &ItemRef| {
            base_uri(&item.uri) == f.uri
                || (item.project.is_some() && item.project == f.project && item.id == f.id)
        };
        if same(&self.a) {
            Some(Which::A)
        } else if same(&self.b) {
            Some(Which::B)
        } else {
            None
        }
    }

    pub fn describe(&self) -> String {
        format!(
            "A {}\nB {}\nR {}\nD {}\nuser {}",
            self.a.uri, self.b.uri, self.r, self.d.uri, self.user
        )
    }
}

/// `uri` without a `%revision` suffix.
pub fn base_uri(uri: &str) -> &str {
    match uri.rsplit_once('%') {
        Some((head, rev)) if !rev.is_empty() && rev.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => uri,
    }
}

// ---------------------------------------------------------------- report

/// `OK`, `SKIP` and `FAIL` lines, counted.
#[derive(Debug, Default)]
pub struct Report {
    failures: Vec<String>,
    oks: usize,
    skips: usize,
}

impl Report {
    pub fn ok(&mut self, label: impl Display) {
        println!("OK    {label}");
        self.oks += 1;
    }

    pub fn skip(&mut self, label: impl Display, why: impl Display) {
        println!("SKIP  {label}: {why}");
        self.skips += 1;
    }

    pub fn fail(&mut self, label: impl Display, why: impl Display) {
        let line = format!("{label}: {why}");
        println!("FAIL  {line}");
        self.failures.push(line);
    }

    /// Indented detail lines.
    pub fn info(&self, text: impl Display) {
        for line in text.to_string().lines() {
            println!("      {line}");
        }
    }

    /// `OK label` when `ok`, else `FAIL label: why`; returns `ok`.
    pub fn check(&mut self, label: impl Display, ok: bool, why: impl Display) -> bool {
        if ok {
            self.ok(label);
        } else {
            self.fail(label, why);
        }
        ok
    }

    /// `OK label` and the value, or `FAIL label: error`.
    pub fn result<T, E: Display>(
        &mut self,
        label: impl Display,
        result: Result<T, E>,
    ) -> Option<T> {
        match result {
            Ok(value) => {
                self.ok(label);
                Some(value)
            }
            Err(e) => {
                self.fail(label, e);
                None
            }
        }
    }

    pub fn failed(&self) -> bool {
        !self.failures.is_empty()
    }

    pub fn failures(&self) -> usize {
        self.failures.len()
    }

    /// Print the counts; `true` when nothing failed.
    pub fn summary(&self, name: &str) -> bool {
        println!(
            "== {name}: {} OK, {} SKIP, {} FAIL",
            self.oks,
            self.skips,
            self.failures.len()
        );
        for failure in &self.failures {
            println!("   FAIL {failure}");
        }
        self.failures.is_empty()
    }

    /// Print the counts and panic when anything failed.
    pub fn finish(self, name: &str) {
        if !self.summary(name) {
            panic!("{name}: {} checks failed", self.failures.len());
        }
    }
}

/// JSON, with long strings shortened, as indented lines.
pub fn show(value: &Value) {
    fn shorten(v: &mut Value) {
        match v {
            Value::String(s) if s.chars().count() > 200 => {
                let total = s.chars().count();
                *s = format!(
                    "{}… ({total} chars)",
                    s.chars().take(200).collect::<String>()
                );
            }
            Value::Array(items) => items.iter_mut().for_each(shorten),
            Value::Object(map) => map.values_mut().for_each(shorten),
            _ => {}
        }
    }
    let mut v = value.clone();
    shorten(&mut v);
    let text = serde_json::to_string_pretty(&v).unwrap_or_default();
    for line in text.lines() {
        println!("      {line}");
    }
}

/// A value as compact JSON, cut at 300 characters.
pub fn short(value: &Value) -> String {
    cut(&value.to_string())
}

/// Text cut at 300 characters.
pub fn cut(text: &str) -> String {
    let count = text.chars().count();
    if count <= 300 {
        text.to_owned()
    } else {
        format!(
            "{}… ({count} chars)",
            text.chars().take(300).collect::<String>()
        )
    }
}

/// Where two texts first differ.
pub fn first_difference(expected: &str, actual: &str) -> String {
    let at = expected
        .chars()
        .zip(actual.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let excerpt = |s: &str| -> String { s.chars().skip(at.saturating_sub(40)).take(120).collect() };
    format!(
        "at character {at}: expected …{:?}, actual …{:?}",
        excerpt(expected),
        excerpt(actual)
    )
}

static TIER: AtomicBool = AtomicBool::new(false);

/// Held while a tier runs: two tiers never run at the same time in one process.
#[derive(Debug)]
pub struct OneTier;

pub fn one_tier(name: &str) -> OneTier {
    assert!(
        !TIER.swap(true, Ordering::SeqCst),
        "{name}: another live tier is running in this process; run one tier at a time"
    );
    println!("== live tier: {name}");
    OneTier
}

impl Drop for OneTier {
    fn drop(&mut self) {
        TIER.store(false, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------- clients

fn builder(settings: &Settings) -> polarion::PolarionBuilder {
    Polarion::builder(&settings.url, settings.auth()).danger_accept_invalid_certs(settings.insecure)
}

/// The read-only client every read of the suite goes through (the SOAP read-only allowlist
/// applies).
pub fn reader(settings: &Settings) -> Polarion {
    let reader = builder(settings)
        .build()
        .unwrap_or_else(|e| panic!("cannot build the read-only client: {e}"));
    assert!(
        reader.soap().is_read_only()
            && !reader.writes_restricted()
            && reader.writable_projects().next().is_none()
            && !reader.global_writes_allowed(),
        "the reader must be read-only"
    );
    reader
}

/// Gate 1 and 2: the writer, restricted to the URIs of A, B, R and D and nothing else (no project
/// writes, no creates).
pub fn writer(settings: &Settings, fixtures: &Fixtures) -> Result<Polarion, String> {
    check_live_writes(settings)?;
    let uris = fixtures.uris();
    let distinct: BTreeSet<&str> = uris.iter().copied().collect();
    if distinct.len() != uris.len()
        || uris
            .iter()
            .any(|u| !u.starts_with("subterra:data-service:objects:/") || base_uri(u) != *u)
    {
        return Err(format!(
            "the fixture URIs are not four distinct object URIs: {uris:?}"
        ));
    }
    // The audit covers this project only: no target may be in another one.
    if let Some(other) = uris
        .iter()
        .find(|u| polarion::uri::project_of(u) != Some(settings.project.as_str()))
    {
        return Err(format!(
            "the fixture {other} is not in POLARION_PROJECT ({})",
            settings.project
        ));
    }
    let writer = builder(settings)
        .restrict_writes(uris)
        .build()
        .map_err(|e| format!("cannot build the writer: {e}"))?;
    let restricted = {
        let targets: BTreeSet<&str> = writer.write_targets().collect();
        writer.writes_restricted()
            && targets == distinct
            && writer.writable_projects().next().is_none()
            && !writer.global_writes_allowed()
    };
    if !restricted {
        return Err("the writer is not restricted to exactly the fixtures".into());
    }
    Ok(writer)
}

// ---------------------------------------------------------------- gate 3, 4, 5

/// The fields gate 3 checks, read by key: `get()` without a field list might leave some out,
/// and then a check would pass on nothing.
pub const IDENTITY_FIELDS: [&str; 8] = [
    "title",
    "type",
    "author",
    "assignee",
    "approvals",
    "plannedIn",
    "linkedWorkItems",
    "module",
];

/// What gate 3 checks of a work item, as JSON (to compare `get()` with the fields by key).
pub fn identity_of(item: &WorkItem) -> Value {
    json!({
        "title": item.title,
        "type": item.kind,
        "author": item.author,
        "assignees": item.assignees,
        "approvals": serde_json::to_value(&item.approvals).unwrap_or(Value::Null),
        "plannedIn": item.planned_in,
        "links": serde_json::to_value(&item.links).unwrap_or(Value::Null),
        "document": item.document.as_ref().map(|d| d.uri.clone()),
    })
}

/// Gate 3: each fixture is what the bootstrap made. `true` when every check passed. Watchers
/// cannot be read through the API: check them in the browser (docs/testing.md).
pub async fn check_identity(report: &mut Report, reader: &Polarion, f: &Fixtures) -> bool {
    let before = report.failures();
    for (which, kind) in [(Which::A, TYPE_A), (Which::B, TYPE_B)] {
        match reader
            .items()
            .get_fields(f.get(which), &Fields::new(IDENTITY_FIELDS))
            .await
        {
            Ok(item) => check_item(report, f, which, kind, &item),
            Err(e) => report.fail(format!("fixture {}", which.label()), e),
        }
    }
    match reader.tests().get_by_uri(&f.r, None).await {
        Ok(run) => {
            check!(
                report,
                run.title
                    .as_deref()
                    .is_some_and(|t| t.starts_with(MARKER_R)),
                "fixture R: title marker",
                "title is {:?}",
                run.title
            );
            check!(
                report,
                run.id.as_deref() == Some(f.r_id.as_str()) && !run.template,
                format!("fixture R: test run {} (no template)", f.r_id),
                "id {:?}, template {}",
                run.id,
                run.template
            );
            check!(
                report,
                run.author.as_deref() == Some(f.user.as_str()),
                "fixture R: author is POLARION_LIVE_USER",
                "author is {:?}, POLARION_LIVE_USER is {}",
                run.author,
                f.user
            );
        }
        Err(e) => report.fail("fixture R", e),
    }
    match reader.documents().get_by_ref(&f.d, None).await {
        Ok(doc) => {
            check!(
                report,
                doc.name.as_deref() == Some(DOCUMENT_NAME),
                "fixture D: name",
                "name is {:?}",
                doc.name
            );
            check!(
                report,
                doc.body
                    .as_ref()
                    .is_some_and(|b| b.content.contains(MARKER_D)),
                "fixture D: body marker",
                "the body does not contain {MARKER_D}"
            );
            check!(
                report,
                doc.author.as_deref() == Some(f.user.as_str()),
                "fixture D: author is POLARION_LIVE_USER",
                "author is {:?}, POLARION_LIVE_USER is {}",
                doc.author,
                f.user
            );
        }
        Err(e) => report.fail("fixture D", e),
    }
    match reader.documents().work_item_refs(&f.d, None, true).await {
        Ok(list) => {
            check!(
                report,
                list.is_empty(),
                "fixture D: holds no work items",
                "it holds {:?}",
                list.iter().map(ToString::to_string).collect::<Vec<_>>()
            );
        }
        Err(e) => report.fail("fixture D: work items", e),
    }
    report.failures() == before
}

fn check_item(report: &mut Report, f: &Fixtures, which: Which, kind: &str, item: &WorkItem) {
    let label = |what: &str| format!("fixture {}: {what}", which.label());
    check!(
        report,
        item.title
            .as_deref()
            .is_some_and(|t| t.starts_with(which.marker())),
        label("title marker"),
        "title is {:?}",
        item.title
    );
    check!(
        report,
        item.kind.as_deref() == Some(kind),
        label(&format!("type {kind}")),
        "type is {:?}",
        item.kind
    );
    check!(
        report,
        item.author.as_deref() == Some(f.user.as_str()),
        label("author is POLARION_LIVE_USER"),
        "author is {:?}, POLARION_LIVE_USER is {}",
        item.author,
        f.user
    );
    let assignees: Vec<&String> = item.assignees.iter().filter(|a| **a != f.user).collect();
    check!(
        report,
        assignees.is_empty(),
        label("assigned to no one but the user"),
        "also assigned to {assignees:?}"
    );
    let approvers: Vec<&str> = item
        .approvals
        .iter()
        .filter_map(|a| a.user.as_deref())
        .filter(|u| *u != f.user)
        .collect();
    check!(
        report,
        approvers.is_empty(),
        label("no approver but the user"),
        "approvers {approvers:?}"
    );
    check!(
        report,
        item.planned_in.is_empty(),
        label("in no plan"),
        "planned in {:?}",
        item.planned_in
    );
    // In a document, every title and description write would change what the document shows.
    check!(
        report,
        item.document.is_none(),
        label("in no document"),
        "in the document {:?}",
        item.document.as_ref().map(ToString::to_string)
    );
    let foreign: Vec<String> = item
        .links
        .iter()
        .filter(|l| f.item(&l.item).is_none())
        .map(|l| l.item.to_string())
        .collect();
    check!(
        report,
        foreign.is_empty(),
        label("links only to fixtures"),
        "links to {foreign:?}"
    );
}

/// Gate 4: the back links of A and B as `X <- role SOURCE` lines, and those whose source is no
/// fixture.
pub async fn back_links(
    reader: &Polarion,
    f: &Fixtures,
) -> Result<(BTreeSet<String>, Vec<String>), String> {
    let mut all = BTreeSet::new();
    let mut foreign = Vec::new();
    for which in [Which::A, Which::B] {
        let links = reader
            .items()
            .back_links(f.get(which))
            .await
            .map_err(|e| format!("back links of {}: {e}", which.label()))?;
        for link in links {
            let line = format!(
                "{} <- {} {}",
                which.label(),
                link.role.as_deref().unwrap_or("?"),
                link.item
            );
            if f.item(&link.item).is_none() {
                foreign.push(line.clone());
            }
            all.insert(line);
        }
    }
    Ok((all, foreign))
}

/// Gate 5: the canary, the newest work item that is no fixture. The writer must not allow it and
/// must allow each fixture (pure checks), and the only write attempted on it, the harmless
/// `removeHyperlink` of an `example.invalid` URL, must be refused before anything is sent.
pub async fn canary(
    report: &mut Report,
    reader: &Polarion,
    writer: &Polarion,
    f: &Fixtures,
) -> Option<ItemRef> {
    let query = Query::new("")
        .project(&f.project)
        .sort("~created")
        .limit(10);
    let refs = match reader.items().query_refs(&query).await {
        Ok(refs) => refs,
        Err(e) => {
            report.fail("gate 5: the newest work items", e);
            return None;
        }
    };
    let Some(canary) = refs.into_iter().find(|r| f.item(r).is_none()) else {
        report.fail(
            "gate 5",
            "no work item other than the fixtures found for the canary",
        );
        return None;
    };
    report.info(format!("canary: {canary} ({})", canary.uri));
    if !check!(
        report,
        !writer.write_allowed(&canary.uri),
        "gate 5: write_allowed is false for the canary",
        "the writer would write to {}",
        canary.uri
    ) {
        return None;
    }
    for uri in f.uris() {
        if !check!(
            report,
            writer.write_allowed(uri),
            format!("gate 5: write_allowed is true for {uri}"),
            "the writer refuses a fixture"
        ) {
            return None;
        }
    }
    match writer.items().remove_hyperlink(&canary, CANARY_URL).await {
        Err(Error::WriteNotAllowed { .. }) => {
            report.ok("gate 5: removeHyperlink on the canary is refused before it is sent");
            Some(canary)
        }
        Err(e) => {
            report.fail(
                "gate 5: removeHyperlink on the canary",
                format!("expected WriteNotAllowed, got: {e}"),
            );
            None
        }
        Ok(changed) => {
            report.fail(
                "gate 5: removeHyperlink on the canary",
                format!(
                    "THE REQUEST WAS SENT (changed: {changed}): the guard is broken, stop and \
                     investigate"
                ),
            );
            None
        }
    }
}

// ---------------------------------------------------------------- snapshots and the baseline

/// The custom field definitions of A and B, by key.
#[derive(Debug, Clone, Default)]
pub struct Definitions {
    pub a: BTreeMap<String, FieldDefinition>,
    pub b: BTreeMap<String, FieldDefinition>,
}

impl Definitions {
    pub fn get(&self, which: Which) -> &BTreeMap<String, FieldDefinition> {
        match which {
            Which::A => &self.a,
            Which::B => &self.b,
        }
    }
}

pub async fn definitions(reader: &Polarion, f: &Fixtures) -> Result<Definitions, String> {
    let mut out = Definitions::default();
    for which in [Which::A, Which::B] {
        let list = reader
            .schema()
            .custom_fields_of(f.get(which))
            .await
            .map_err(|e| format!("custom fields of {}: {e}", which.label()))?;
        let map = list.into_iter().map(|d| (d.id.clone(), d)).collect();
        match which {
            Which::A => out.a = map,
            Which::B => out.b = map,
        }
    }
    Ok(out)
}

/// The state of the fixtures, as JSON: A and B with their custom fields and planning
/// constraints, A's localized title (default language; `null` when it cannot be read), B's
/// test steps, R and its report, D with its body.
pub async fn snapshot(
    reader: &Polarion,
    f: &Fixtures,
    defs: &Definitions,
) -> Result<Value, String> {
    let a = item_state(reader, &f.a, &defs.a).await?;
    let a_localized = reader
        .items()
        .localized_title(&f.a, None)
        .await
        .map_or(Value::Null, Value::String);
    let b = item_state(reader, &f.b, &defs.b).await?;
    let steps = reader
        .tests()
        .steps(&f.b)
        .await
        .map_err(|e| format!("test steps of B: {e}"))?;
    let run = reader
        .tests()
        .get_by_uri(&f.r, None)
        .await
        .map_err(|e| format!("R: {e}"))?;
    let report = reader
        .tests()
        .report(&f.r)
        .await
        .map_err(|e| format!("report of R: {e}"))?;
    let document = reader
        .documents()
        .get_by_ref(&f.d, None)
        .await
        .map_err(|e| format!("D: {e}"))?;
    Ok(json!({
        "a": a,
        "aLocalizedTitle": a_localized,
        "b": b,
        "bSteps": serde_json::to_value(&steps).map_err(|e| e.to_string())?,
        "r": serde_json::to_value(&run).map_err(|e| e.to_string())?,
        "rReport": serde_json::to_value(&report).map_err(|e| e.to_string())?,
        "d": serde_json::to_value(&document).map_err(|e| e.to_string())?,
    }))
}

async fn item_state(
    reader: &Polarion,
    item: &ItemRef,
    defs: &BTreeMap<String, FieldDefinition>,
) -> Result<Value, String> {
    let full = reader
        .items()
        .get(item)
        .await
        .map_err(|e| format!("{item}: {e}"))?;
    let mut value = serde_json::to_value(&full).map_err(|e| e.to_string())?;
    // Custom fields by key: the key `customFields` alone returns none of them.
    let custom = if defs.is_empty() {
        BTreeMap::new()
    } else {
        let fields = defs
            .keys()
            .fold(Fields::new(["id"]), |fields, key| fields.with_custom(key));
        reader
            .items()
            .get_fields(item, &fields)
            .await
            .map_err(|e| format!("custom fields of {item}: {e}"))?
            .custom_fields
    };
    // Values of types this library does not map (tables, test steps, ...) carry no content to
    // compare; the suite writes none of them (test steps are compared as `bSteps`).
    let custom: BTreeMap<String, FieldValue> = custom
        .into_iter()
        .filter(|(_, v)| !matches!(v, FieldValue::Other { .. }))
        .collect();
    value["customFields"] = serde_json::to_value(&custom).map_err(|e| e.to_string())?;
    value["planningConstraints"] = json!(planning_constraints(reader, item).await?);
    Ok(value)
}

/// The planning constraints of a work item as `(date, constraint)`, sorted. The domain type has
/// none, so they are read with the read-only client's SOAP client.
pub async fn planning_constraints(
    reader: &Polarion,
    item: &ItemRef,
) -> Result<Vec<(String, String)>, String> {
    let w = reader
        .soap()
        .tracker()
        .get_work_item_by_uri_with_fields(&item.uri, &["planningConstraints".to_owned()])
        .await
        .map_err(|e| format!("planning constraints of {item}: {e}"))?;
    let mut list: Vec<(String, String)> = w
        .planning_constraints
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            (
                c.date.unwrap_or_default(),
                c.constraint.and_then(|e| e.id).unwrap_or_default(),
            )
        })
        .collect();
    list.sort();
    Ok(list)
}

/// Fields that change with every write, or cannot be restored (comments).
const ITEM_VOLATILE: &[&str] = &["updated", "comments"];
/// R's status follows its records (accepted).
const RUN_VOLATILE: &[&str] = &["updated", "status", "finishedOn", "comments"];
const DOC_VOLATILE: &[&str] = &["updated", "updatedBy", "comments"];

/// The parts of a snapshot and the fields they do not compare.
const PARTS: [(&str, &[&str]); 7] = [
    ("a", ITEM_VOLATILE),
    ("aLocalizedTitle", &[]),
    ("b", ITEM_VOLATILE),
    ("bSteps", &[]),
    ("r", RUN_VOLATILE),
    ("rReport", &[]),
    ("d", DOC_VOLATILE),
];

/// A part of a snapshot as it is compared: without its volatile fields, without the `url` and
/// `updated` of attachments, without empty values.
fn comparable(value: &Value, volatile: &[&str]) -> Value {
    fn clean(v: &mut Value) {
        match v {
            Value::Object(map) => {
                if let Some(Value::Array(list)) = map.get_mut("attachments") {
                    for a in list.iter_mut() {
                        if let Value::Object(a) = a {
                            a.remove("url");
                            a.remove("updated");
                        }
                    }
                }
                for child in map.values_mut() {
                    clean(child);
                }
                map.retain(|_, child| !is_empty(child));
            }
            Value::Array(list) => list.iter_mut().for_each(clean),
            _ => {}
        }
    }
    let mut value = value.clone();
    if let Value::Object(map) = &mut value {
        for key in volatile {
            map.remove(*key);
        }
    }
    clean(&mut value);
    value
}

fn is_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// The differences between two snapshots, field by field (`part.field: baseline …, now …`).
pub fn differences(baseline: &Value, current: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for (part, volatile) in PARTS {
        let base = comparable(&baseline[part], volatile);
        let cur = comparable(&current[part], volatile);
        match (&base, &cur) {
            (Value::Object(_), Value::Object(_)) => {
                for key in keys(&base, &cur) {
                    let (b, c) = (&base[key.as_str()], &cur[key.as_str()]);
                    if b != c {
                        out.push(format!(
                            "{part}.{key}: baseline {}, now {}",
                            short(b),
                            short(c)
                        ));
                    }
                }
            }
            _ if base != cur => {
                out.push(format!(
                    "{part}: baseline {}, now {}",
                    short(&base),
                    short(&cur)
                ));
            }
            _ => {}
        }
    }
    out
}

fn keys(a: &Value, b: &Value) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for v in [a, b] {
        if let Value::Object(map) = v {
            keys.extend(map.keys().cloned());
        }
    }
    keys
}

fn list(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

/// The elements of `a` that are not in `b`.
fn minus(a: &[Value], b: &[Value]) -> Vec<Value> {
    a.iter().filter(|x| !b.contains(x)).cloned().collect()
}

/// A rich text from its JSON form `{"type", "content"}`.
pub fn text_of(v: &Value) -> Option<Text> {
    Some(Text {
        content_type: v["type"].as_str().unwrap_or("text/html").to_owned(),
        content: v["content"].as_str()?.to_owned(),
    })
}

/// Whether `path` is `dir` or inside it, compared by components (made absolute, `.` and `..`
/// resolved without the file system; on Windows ignoring case).
pub fn inside(path: &Path, dir: &Path) -> bool {
    fn parts(path: &Path) -> Vec<String> {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let mut out: Vec<String> = Vec::new();
        for c in absolute.components() {
            match c {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                c => {
                    let s = c.as_os_str().to_string_lossy().into_owned();
                    out.push(if cfg!(windows) {
                        s.to_ascii_lowercase()
                    } else {
                        s
                    });
                }
            }
        }
        out
    }
    let (path, dir) = (parts(path), parts(dir));
    path.len() >= dir.len() && path[..dir.len()] == dir[..]
}

/// A host name or project id as a folder name.
fn folder_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The file of the baseline: `POLARION_LIVE_BASELINE`, by default
/// `~/.polarion-rs-live/<host>/<project>/baseline.json` (the home folder from `HOME` or
/// `USERPROFILE`). The suite's other state files are next to it: the bootstrap's log of the
/// fixtures it created ([`created_log_path`]) and the log of permanent runs
/// ([`permanent_log_path`]). Never inside the workspace's `target` folder, which `cargo clean`
/// deletes: without the baseline nothing can be restored, and without the log the bootstrap
/// could create a fixture twice.
pub fn baseline_path(settings: &Settings) -> Result<PathBuf, String> {
    let path = match env("POLARION_LIVE_BASELINE") {
        Some(path) => PathBuf::from(path),
        None => {
            let home = env("HOME").or_else(|| env("USERPROFILE")).ok_or(
                "neither HOME nor USERPROFILE is set: set POLARION_LIVE_BASELINE to the file \
                 of the baseline (outside target/)",
            )?;
            PathBuf::from(home)
                .join(".polarion-rs-live")
                .join(folder_name(&settings.host))
                .join(folder_name(&settings.project))
                .join("baseline.json")
        }
    };
    let target = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target"));
    if inside(&path, &target) {
        return Err(format!(
            "{} is inside the workspace's target folder, which `cargo clean` deletes: set \
             POLARION_LIVE_BASELINE to a file elsewhere",
            path.display()
        ));
    }
    Ok(path)
}

/// The bootstrap's log of the fixtures it created (`live-created.jsonl`, next to the baseline).
pub fn created_log_path(settings: &Settings) -> Result<PathBuf, String> {
    Ok(baseline_path(settings)?.with_file_name("live-created.jsonl"))
}

/// The log of permanent runs (`live-permanent.jsonl`, next to the baseline).
pub fn permanent_log_path(settings: &Settings) -> Result<PathBuf, String> {
    Ok(baseline_path(settings)?.with_file_name("live-permanent.jsonl"))
}

fn baseline_header(f: &Fixtures, host: &str) -> Value {
    json!({
        "host": host,
        "project": f.project,
        "user": f.user,
        "fixtures": {"a": f.a.uri, "b": f.b.uri, "r": f.r, "d": f.d.uri},
    })
}

/// Texts a run writes and restores, never written at creation: in a state, a run stopped before
/// it restored them.
fn run_texts() -> [String; 6] {
    [
        format!("{TEST_DATA} prs"),
        "by the live suite".to_owned(),
        "polarion-rs live test of".to_owned(),
        "index probe prs".to_owned(),
        "identity probe prs".to_owned(),
        "localized title prs".to_owned(),
    ]
}

/// Every string in `value`, with its path.
fn strings(value: &Value, path: &str, found: &mut dyn FnMut(&str, &str)) {
    match value {
        Value::String(s) => found(path, s),
        Value::Array(list) => {
            for (i, v) in list.iter().enumerate() {
                strings(v, &format!("{path}[{i}]"), found);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                strings(v, &format!("{path}.{k}"), found);
            }
        }
        _ => {}
    }
}

/// Whether `name` is a file the suite uploaded: it starts with [`FILE_PREFIX`], or (test record
/// and step attachments, which the server stores as `<test case id>_<file name>` and
/// `<test case id>_<step>_<file name>`) an id prefix is followed by `_` and [`FILE_PREFIX`].
pub fn is_suite_file(name: &str) -> bool {
    name.starts_with(FILE_PREFIX)
        || name
            .split_once(&format!("_{FILE_PREFIX}"))
            .is_some_and(|(head, _)| {
                !head.is_empty()
                    && head
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
            })
}

/// Whether `stored` is the server's name of the file the suite uploaded as `file`: the same, or
/// `file` after an id prefix and `_` (see [`is_suite_file`]).
pub fn server_name_of(stored: &str, file: &str) -> bool {
    stored == file || (stored.ends_with(&format!("_{file}")) && is_suite_file(stored))
}

/// After a test record or step attachment was deleted: the run's own attachment list must not
/// keep the file either. If it does, it is deleted from the run (R is a target, the file is the
/// suite's) and the deletion is checked.
pub async fn run_attachment_gone(report: &mut Report, g: &Gate, file: &str) -> Result<(), String> {
    let f = &g.fixtures;
    let listed = |list: &[polarion::Attachment]| {
        list.iter()
            .filter_map(|a| a.file_name.clone())
            .find(|n| server_name_of(n, file))
    };
    let list = g
        .reader
        .tests()
        .attachments(&f.r)
        .await
        .map_err(|e| format!("R: attachments: {e}"))?;
    let Some(stored) = listed(&list) else {
        report.ok(format!("R: {file} is not among the run's attachments"));
        return Ok(());
    };
    report.info(format!(
        "R: the run still lists {stored} after the delete; deleting it from the run"
    ));
    g.writer
        .tests()
        .delete_attachment(&f.r, &stored)
        .await
        .map_err(|e| format!("R: deleteTestRunAttachment {stored}: {e}"))?;
    let list = g
        .reader
        .tests()
        .attachments(&f.r)
        .await
        .map_err(|e| format!("R: attachments: {e}"))?;
    check!(
        report,
        listed(&list).is_none(),
        format!("R: {stored} is gone from the run"),
        "it is still listed"
    );
    Ok(())
}

/// An attachment (JSON) the suite adds and deletes: its file name prefix, or a diagram of the
/// Markdown writes.
fn is_suite_attachment(a: &Value) -> bool {
    let name = a["fileName"].as_str().unwrap_or_default();
    is_suite_file(name)
        || (name.starts_with("mermaid-")
            && name.ends_with(".svg")
            && a["title"].as_str() == Some(DIAGRAM_TITLE))
}

/// A custom field value the suite writes over the value the bootstrap gave A
/// ([`test_value`] of the bootstrap's value: 8, 3.5, 13.5, the second date and time).
fn second_test_value(v: &Value) -> bool {
    match v {
        Value::Number(n) => n
            .as_f64()
            .is_some_and(|x| (x - 8.0).abs() < 1e-9 || (x - 3.5).abs() < 1e-9),
        Value::String(s) => {
            s.starts_with("2001-02-04") || s == "05:06:07" || s == "13.5" || s == "13.50"
        }
        Value::Array(list) => list.iter().any(second_test_value),
        _ => false,
    }
}

/// What a run leaves behind when it stops before restoring, found in a state of the fixtures:
/// the reasons not to store it as the baseline (empty: as the bootstrap made it, as far as can
/// be seen). Checked: the creation titles and descriptions, the texts a run writes, the suite's
/// hyperlinks, attachments and planning constraint, work records, approvals and assignments of
/// the user, links between the fixtures, records on R, and A's custom fields holding the
/// suite's second test values.
pub fn baseline_problems(f: &Fixtures, state: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let texts = run_texts();
    for (part, volatile) in PARTS {
        let value = comparable(&state[part], volatile);
        strings(&value, part, &mut |path, s| {
            if let Some(t) = texts.iter().find(|t| s.contains(t.as_str())) {
                out.push(format!("{path} is {:?} (a run writes {t:?})", cut(s)));
            }
        });
    }
    for (part, marker) in [("a", MARKER_A), ("b", MARKER_B), ("r", MARKER_R)] {
        let title = state[part]["title"].as_str();
        if title != Some(fixture_title(marker).as_str()) {
            out.push(format!(
                "{part}.title is {title:?}, not {:?}",
                fixture_title(marker)
            ));
        }
    }
    let user = Some(f.user.as_str());
    for (part, marker) in [("a", MARKER_A), ("b", MARKER_B)] {
        let item = &state[part];
        let description = text_only(item["description"]["content"].as_str().unwrap_or_default());
        if description != text_only(&fixture_description(marker, "work item")) {
            out.push(format!(
                "{part}.description is {:?}, not the description it was created with",
                cut(&description)
            ));
        }
        for h in list(&item["hyperlinks"]) {
            if h["url"]
                .as_str()
                .is_some_and(|u| u.starts_with("https://example.invalid/"))
            {
                out.push(format!("{part}.hyperlinks: {} is the suite's", short(&h)));
            }
        }
        for a in list(&item["attachments"]) {
            if is_suite_attachment(&a) {
                out.push(format!("{part}.attachments: {} is the suite's", short(&a)));
            }
        }
        for r in list(&item["workRecords"]) {
            if r["user"].as_str() == user {
                out.push(format!("{part}.workRecords: {} is the user's", short(&r)));
            }
        }
        for a in list(&item["approvals"]) {
            if a["user"].as_str() == user {
                out.push(format!(
                    "{part}.approvals: the user approves ({})",
                    short(&a)
                ));
            }
        }
        if list(&item["assignees"]).iter().any(|a| a.as_str() == user) {
            out.push(format!("{part}.assignees: the user is assigned"));
        }
        for l in list(&item["links"]) {
            out.push(format!(
                "{part}.links: {} (the bootstrap links nothing)",
                short(&l)
            ));
        }
        for c in list(&item["planningConstraints"]) {
            if c[0].as_str().is_some_and(|d| d.starts_with("2001-02-03")) {
                out.push(format!(
                    "{part}.planningConstraints: {} is the suite's",
                    short(&c)
                ));
            }
        }
    }
    if let Value::Object(fields) = &state["a"]["customFields"] {
        for (key, value) in fields {
            if second_test_value(value) {
                out.push(format!(
                    "a.customFields.{key} is {}, the value a run writes over the bootstrap's",
                    short(value)
                ));
            }
        }
    }
    if !list(&state["r"]["records"]).is_empty() {
        out.push(format!(
            "r.records: R has records (the bootstrap adds none): {}",
            short(&state["r"]["records"])
        ));
    }
    for a in list(&state["r"]["attachments"]) {
        if a["fileName"].as_str().is_some_and(is_suite_file) {
            out.push(format!("r.attachments: {} is the suite's", short(&a)));
        }
    }
    let body = text_only(state["d"]["body"]["content"].as_str().unwrap_or_default());
    if !body.contains(&text_only(&fixture_description(MARKER_D, "document"))) {
        out.push(format!(
            "d.body is {:?}, without the text D was created with",
            cut(&body)
        ));
    }
    out
}

/// The stored baseline state of these fixtures; `None` when there is no baseline file. A file
/// for another server, project or other fixtures is an error.
pub fn load_baseline(settings: &Settings, f: &Fixtures) -> Result<Option<Value>, String> {
    let host = settings.host.as_str();
    let path = baseline_path(settings)?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let stored: Value = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not a baseline: {e}", path.display()))?;
    let expected = baseline_header(f, host);
    for key in ["host", "project", "user", "fixtures"] {
        if stored[key] != expected[key] {
            return Err(format!(
                "{} belongs to other fixtures (`{key}` is {}, expected {}); move it away only \
                 if the fixtures were recreated",
                path.display(),
                short(&stored[key]),
                short(&expected[key])
            ));
        }
    }
    if !stored["state"].is_object() {
        return Err(format!("{} has no state", path.display()));
    }
    Ok(Some(stored["state"].clone()))
}

/// Store the baseline state (only the bootstrap does); an existing file is never overwritten,
/// and a state with [`baseline_problems`] is never stored: the suite would restore the
/// fixtures to what a stopped run left behind.
pub fn store_baseline(settings: &Settings, f: &Fixtures, state: &Value) -> Result<PathBuf, String> {
    let problems = baseline_problems(f, state);
    if !problems.is_empty() {
        return Err(format!(
            "the fixtures are not as the bootstrap made them, so their state is not stored as \
             the baseline; fix them in the browser and run the bootstrap again: {}",
            problems.join("; ")
        ));
    }
    let path = baseline_path(settings)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let mut doc = baseline_header(f, &settings.host);
    doc["taken"] = json!(iso(now()));
    doc["state"] = state.clone();
    let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| {
            format!(
                "cannot create {} (it is never overwritten): {e}",
                path.display()
            )
        })?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

// ---------------------------------------------------------------- restoring

/// One write that brings a fixture back to the baseline.
#[derive(Debug, Clone)]
pub enum Restore {
    Title(Which, String),
    Description(Which, Option<Text>),
    CustomField(Which, String, Option<FieldValue>),
    RemoveHyperlink(Which, String),
    AddHyperlink(Which, String, String),
    RemoveCategory(Which, String),
    AddCategory(Which, String),
    RemoveLink(Which, String, Which),
    AddLink(Which, String, Which),
    RemoveAssignee(Which),
    AddAssignee(Which),
    RemoveApprovee(Which),
    DeleteAttachment(Which, String),
    DeleteWorkRecord(Which, String),
    RemoveConstraint(Which, String, String),
    AddConstraint(Which, String, String),
    /// `timeSpent` or `remainingEstimate` (none: cleared).
    Estimate(Which, &'static str, Option<String>),
    /// A's localized title in the default language (none: cleared).
    LocalizedTitle(Option<String>),
    Steps(Vec<Vec<Text>>),
    RunTitle(String),
    RunReport(Text),
    RemoveRecord(u32),
    DeleteRunAttachment(String),
    Body(Text),
}

impl Restore {
    fn describe(&self) -> String {
        match self {
            Restore::Title(w, t) => format!("{}: title {t:?}", w.label()),
            Restore::Description(w, _) => format!("{}: description", w.label()),
            Restore::CustomField(w, k, v) => {
                format!("{}: custom field {k} = {}", w.label(), short(&json!(v)))
            }
            Restore::RemoveHyperlink(w, u) => format!("{}: remove hyperlink {u}", w.label()),
            Restore::AddHyperlink(w, u, _) => format!("{}: add hyperlink {u}", w.label()),
            Restore::RemoveCategory(w, c) => format!("{}: remove category {c}", w.label()),
            Restore::AddCategory(w, c) => format!("{}: add category {c}", w.label()),
            Restore::RemoveLink(w, r, t) => {
                format!("{}: remove link {r} {}", w.label(), t.label())
            }
            Restore::AddLink(w, r, t) => format!("{}: add link {r} {}", w.label(), t.label()),
            Restore::RemoveAssignee(w) => format!("{}: remove the user as assignee", w.label()),
            Restore::AddAssignee(w) => format!("{}: add the user as assignee", w.label()),
            Restore::RemoveApprovee(w) => format!("{}: remove the user as approver", w.label()),
            Restore::DeleteAttachment(w, id) => format!("{}: delete attachment {id}", w.label()),
            Restore::DeleteWorkRecord(w, u) => format!("{}: delete work record {u}", w.label()),
            Restore::RemoveConstraint(w, d, c) => {
                format!("{}: remove planning constraint {c} {d}", w.label())
            }
            Restore::AddConstraint(w, d, c) => {
                format!("{}: add planning constraint {c} {d}", w.label())
            }
            Restore::Estimate(w, key, v) => format!("{}: {key} {v:?}", w.label()),
            Restore::LocalizedTitle(t) => format!("A: localized title {t:?}"),
            Restore::Steps(s) => format!("B: {} test steps", s.len()),
            Restore::RunTitle(t) => format!("R: title {t:?}"),
            Restore::RunReport(_) => "R: report".into(),
            Restore::RemoveRecord(i) => format!("R: remove record {i}"),
            Restore::DeleteRunAttachment(n) => format!("R: delete attachment {n}"),
            Restore::Body(_) => "D: body".into(),
        }
    }
}

/// The writes that bring `current` back to `baseline`, and the differences no write of the suite
/// can undo (then nothing may be written).
fn plan_restore(
    f: &Fixtures,
    defs: &Definitions,
    baseline: &Value,
    current: &Value,
) -> (Vec<Restore>, Vec<String>) {
    let mut plan = Vec::new();
    let mut manual = Vec::new();
    for which in [Which::A, Which::B] {
        let base = comparable(&baseline[which.key()], ITEM_VOLATILE);
        let cur = comparable(&current[which.key()], ITEM_VOLATILE);
        plan_item(
            f,
            which,
            defs.get(which),
            &base,
            &cur,
            &mut plan,
            &mut manual,
        );
    }
    // After A's title: a localized title in the default language may follow the title.
    let (base, cur) = (&baseline["aLocalizedTitle"], &current["aLocalizedTitle"]);
    if base != cur {
        match (base, cur) {
            (Value::String(b), Value::String(_)) => {
                plan.push(Restore::LocalizedTitle((!b.is_empty()).then(|| b.clone())))
            }
            _ => manual.push(format!(
                "aLocalizedTitle: baseline {}, now {} (unreadable, or a baseline taken without it: \
                 store a new one with the bootstrap)",
                short(base),
                short(cur)
            )),
        }
    }
    if comparable(&baseline["bSteps"], &[]) != comparable(&current["bSteps"], &[]) {
        let steps: Option<Vec<Vec<Text>>> = list(&baseline["bSteps"]["steps"])
            .iter()
            .map(|row| list(row).iter().map(text_of).collect::<Option<Vec<Text>>>())
            .collect();
        match steps {
            Some(steps) => plan.push(Restore::Steps(steps)),
            None => manual.push("bSteps: the baseline's steps cannot be read".into()),
        }
    }
    let base = comparable(&baseline["r"], RUN_VOLATILE);
    let cur = comparable(&current["r"], RUN_VOLATILE);
    for key in keys(&base, &cur) {
        let (b, c) = (&base[key.as_str()], &cur[key.as_str()]);
        if b == c {
            continue;
        }
        match key.as_str() {
            "title" => match b.as_str() {
                Some(t) if t.starts_with(MARKER_R) => plan.push(Restore::RunTitle(t.to_owned())),
                _ => manual.push(format!("r.title: the baseline title {b} has no marker")),
            },
            "records" => {
                let (bl, cl) = (list(b), list(c));
                // Only records the suite added (its comment), after those of the baseline.
                let ours = |r: &Value| {
                    r["comment"]["content"]
                        .as_str()
                        .is_some_and(|c| c.contains(TEST_DATA))
                };
                if cl.len() > bl.len()
                    && cl[..bl.len()] == bl[..]
                    && cl[bl.len()..].iter().all(ours)
                {
                    for i in (bl.len()..cl.len()).rev() {
                        plan.push(Restore::RemoveRecord(u32::try_from(i).unwrap_or(u32::MAX)));
                    }
                } else {
                    manual.push(format!(
                        "r.records: baseline {}, now {}",
                        short(b),
                        short(c)
                    ));
                }
            }
            "attachments" => {
                let (bl, cl) = (list(b), list(c));
                for a in minus(&cl, &bl) {
                    match a["fileName"].as_str() {
                        Some(name) if is_suite_file(name) => {
                            plan.push(Restore::DeleteRunAttachment(name.to_owned()));
                        }
                        _ => {
                            manual.push(format!("r.attachments: {} is not the suite's", short(&a)))
                        }
                    }
                }
                if !minus(&bl, &cl).is_empty() {
                    manual.push("r.attachments: attachments of the baseline are gone".into());
                }
            }
            // The report: restored from `rReport` below, compared again after the restore.
            "body" => {}
            other => manual.push(format!(
                "r.{other}: baseline {}, now {}",
                short(b),
                short(c)
            )),
        }
    }
    if comparable(&baseline["rReport"], &[]) != comparable(&current["rReport"], &[]) {
        plan.push(Restore::RunReport(
            text_of(&baseline["rReport"]).unwrap_or_else(|| html("")),
        ));
    }
    let base = comparable(&baseline["d"], DOC_VOLATILE);
    let cur = comparable(&current["d"], DOC_VOLATILE);
    for key in keys(&base, &cur) {
        let (b, c) = (&base[key.as_str()], &cur[key.as_str()]);
        if b == c {
            continue;
        }
        match key.as_str() {
            "body" => plan.push(Restore::Body(text_of(b).unwrap_or_else(|| html("")))),
            other => manual.push(format!(
                "d.{other}: baseline {}, now {}",
                short(b),
                short(c)
            )),
        }
    }
    (plan, manual)
}

fn plan_item(
    f: &Fixtures,
    which: Which,
    defs: &BTreeMap<String, FieldDefinition>,
    base: &Value,
    cur: &Value,
    plan: &mut Vec<Restore>,
    manual: &mut Vec<String>,
) {
    let w = which;
    let records_match = base["workRecords"] == cur["workRecords"];
    for key in keys(base, cur) {
        let (b, c) = (&base[key.as_str()], &cur[key.as_str()]);
        if b == c {
            continue;
        }
        let mut note = |what: String| manual.push(format!("{}.{key}: {what}", w.key()));
        let (bl, cl) = (list(b), list(c));
        match key.as_str() {
            "title" => match b.as_str() {
                Some(t) if t.starts_with(w.marker()) => plan.push(Restore::Title(w, t.to_owned())),
                _ => note(format!("the baseline title {b} has no marker")),
            },
            "description" => plan.push(Restore::Description(w, text_of(b))),
            "customFields" => {
                for k in keys(b, c) {
                    let (bv, cv) = (&b[k.as_str()], &c[k.as_str()]);
                    if bv == cv {
                        continue;
                    }
                    match defs.get(&k) {
                        None => note(format!("custom field {k} has no definition")),
                        Some(_) if bv.is_null() => plan.push(Restore::CustomField(w, k, None)),
                        Some(def) => match FieldValue::coerce(bv, def) {
                            Ok(value) => plan.push(Restore::CustomField(w, k, value)),
                            Err(e) => note(format!("custom field {k}: {e}")),
                        },
                    }
                }
            }
            "hyperlinks" => {
                for h in minus(&cl, &bl) {
                    match h["url"].as_str() {
                        Some(url) if url.starts_with("https://example.invalid/") => {
                            plan.push(Restore::RemoveHyperlink(w, url.to_owned()));
                        }
                        _ => note(format!("hyperlink {} is not the suite's", short(&h))),
                    }
                }
                for h in minus(&bl, &cl) {
                    match h["url"].as_str() {
                        Some(url) => plan.push(Restore::AddHyperlink(
                            w,
                            url.to_owned(),
                            h["role"].as_str().unwrap_or("ref_ext").to_owned(),
                        )),
                        None => note(format!("hyperlink {} has no URL", short(&h))),
                    }
                }
            }
            "categories" => {
                for x in minus(&cl, &bl) {
                    match x.as_str() {
                        Some(id) => plan.push(Restore::RemoveCategory(w, id.to_owned())),
                        None => note(format!("category {x}")),
                    }
                }
                for x in minus(&bl, &cl) {
                    match x.as_str() {
                        Some(id) => plan.push(Restore::AddCategory(w, id.to_owned())),
                        None => note(format!("category {x}")),
                    }
                }
            }
            "links" => {
                for l in minus(&cl, &bl) {
                    match (link_target(f, &l), l["role"].as_str()) {
                        (Some(t), Some(role)) => plan.push(Restore::RemoveLink(w, role.into(), t)),
                        _ => note(format!("link {} does not go to a fixture", short(&l))),
                    }
                }
                for l in minus(&bl, &cl) {
                    match (link_target(f, &l), l["role"].as_str()) {
                        (Some(t), Some(role)) => plan.push(Restore::AddLink(w, role.into(), t)),
                        _ => note(format!("link {} does not go to a fixture", short(&l))),
                    }
                }
            }
            "assignees" => {
                for x in minus(&cl, &bl) {
                    if x.as_str() == Some(f.user.as_str()) {
                        plan.push(Restore::RemoveAssignee(w));
                    } else {
                        note(format!("assignee {x} is not the user"));
                    }
                }
                for x in minus(&bl, &cl) {
                    if x.as_str() == Some(f.user.as_str()) {
                        plan.push(Restore::AddAssignee(w));
                    } else {
                        note(format!("assignee {x} is not the user"));
                    }
                }
            }
            "approvals" => {
                let extra = minus(&cl, &bl);
                if !minus(&bl, &cl).is_empty() {
                    note("approvals of the baseline are gone or changed".into());
                } else if extra
                    .iter()
                    .all(|a| a["user"].as_str() == Some(f.user.as_str()))
                {
                    plan.push(Restore::RemoveApprovee(w));
                } else {
                    note(format!(
                        "approvals {} are not the user's",
                        short(&json!(extra))
                    ));
                }
            }
            "attachments" => {
                for a in minus(&cl, &bl) {
                    let ours = is_suite_attachment(&a);
                    match a["id"].as_str() {
                        Some(id) if ours => plan.push(Restore::DeleteAttachment(w, id.to_owned())),
                        _ => note(format!("attachment {} is not the suite's", short(&a))),
                    }
                }
                if !minus(&bl, &cl).is_empty() {
                    note("attachments of the baseline are gone or changed".into());
                }
            }
            "workRecords" => {
                for r in minus(&cl, &bl) {
                    match (r["uri"].as_str(), r["user"].as_str()) {
                        (Some(uri), Some(user)) if user == f.user => {
                            plan.push(Restore::DeleteWorkRecord(w, uri.to_owned()));
                        }
                        _ => note(format!("work record {} is not the user's", short(&r))),
                    }
                }
                if !minus(&bl, &cl).is_empty() {
                    note("work records of the baseline are gone or changed".into());
                }
            }
            "planningConstraints" => {
                for x in minus(&cl, &bl) {
                    match (x[0].as_str(), x[1].as_str()) {
                        // The date as the suite writes it (`YYYY-MM-DD`), not as the server
                        // returns it (possibly a date and time).
                        (Some(d), Some(k)) => {
                            plan.push(Restore::RemoveConstraint(w, day_of(d).into(), k.into()));
                        }
                        _ => note(format!("planning constraint {x}")),
                    }
                }
                for x in minus(&bl, &cl) {
                    match (x[0].as_str(), x[1].as_str()) {
                        (Some(d), Some(k)) => {
                            plan.push(Restore::AddConstraint(w, d.into(), k.into()))
                        }
                        _ => note(format!("planning constraint {x}")),
                    }
                }
            }
            // They follow the work records: restored only when those already match (otherwise
            // deleting the suite's work record restores them; compared again afterwards).
            "timeSpent" | "remainingEstimate" if records_match => {
                let key = if key == "timeSpent" {
                    "timeSpent"
                } else {
                    "remainingEstimate"
                };
                match b {
                    Value::Null => plan.push(Restore::Estimate(w, key, None)),
                    Value::String(v) => plan.push(Restore::Estimate(w, key, Some(v.clone()))),
                    _ => note(format!("baseline {}, now {}", short(b), short(c))),
                }
            }
            "timeSpent" | "remainingEstimate" => {}
            _ => note(format!("baseline {}, now {}", short(b), short(c))),
        }
    }
}

/// The day of a date or date and time (`2001-02-03T00:00:00.000+01:00` → `2001-02-03`).
pub fn day_of(date: &str) -> &str {
    let day = date.get(..10).unwrap_or(date);
    let b = day.as_bytes();
    let is_day = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    if is_day { day } else { date }
}

/// Which fixture a link (JSON) goes to.
fn link_target(f: &Fixtures, link: &Value) -> Option<Which> {
    let item = &link["item"];
    let target = match (
        item["project"].as_str(),
        item["id"].as_str(),
        item["uri"].as_str(),
    ) {
        (Some(project), Some(id), _) => ItemRef::new(project, id),
        (_, _, Some(uri)) => ItemRef::from_uri(uri),
        _ => return None,
    };
    f.item(&target)
}

async fn apply(writer: &Polarion, f: &Fixtures, step: &Restore) -> polarion::Result<()> {
    let items = writer.items();
    let tests = writer.tests();
    let user = f.user.as_str();
    match step {
        Restore::Title(w, title) => {
            items
                .update(
                    f.get(*w),
                    &WorkItemUpdate {
                        title: Some(title.clone()),
                        ..WorkItemUpdate::default()
                    },
                )
                .await
        }
        Restore::Description(w, Some(text)) => {
            items
                .update(
                    f.get(*w),
                    &WorkItemUpdate {
                        description: Some(text.clone()),
                        ..WorkItemUpdate::default()
                    },
                )
                .await
        }
        Restore::Description(w, None) => {
            items
                .clear_fields(f.get(*w), &["description".to_owned()])
                .await
        }
        Restore::CustomField(w, key, value) => {
            items.set_custom_field(f.get(*w), key, value.as_ref()).await
        }
        Restore::RemoveHyperlink(w, url) => {
            items.remove_hyperlink(f.get(*w), url).await.map(|_| ())
        }
        Restore::AddHyperlink(w, url, role) => {
            items.add_hyperlink(f.get(*w), url, role).await.map(|_| ())
        }
        Restore::RemoveCategory(w, c) => items.remove_category(f.get(*w), c).await.map(|_| ()),
        Restore::AddCategory(w, c) => items.add_category(f.get(*w), c).await.map(|_| ()),
        Restore::RemoveLink(w, role, t) => items
            .remove_link(f.get(*w), role, f.get(*t))
            .await
            .map(|_| ()),
        Restore::AddLink(w, role, t) => {
            items.add_link(f.get(*w), role, f.get(*t)).await.map(|_| ())
        }
        Restore::RemoveAssignee(w) => items.remove_assignee(f.get(*w), user).await.map(|_| ()),
        Restore::AddAssignee(w) => items.add_assignee(f.get(*w), user).await.map(|_| ()),
        Restore::RemoveApprovee(w) => items.remove_approvee(f.get(*w), user).await,
        Restore::DeleteAttachment(w, id) => items.delete_attachment(f.get(*w), id).await,
        Restore::DeleteWorkRecord(w, uri) => items.delete_work_record(f.get(*w), uri).await,
        Restore::RemoveConstraint(w, date, c) => items
            .remove_planning_constraint(f.get(*w), date, c)
            .await
            .map(|_| ()),
        Restore::AddConstraint(w, date, c) => items
            .add_planning_constraint(f.get(*w), date, c)
            .await
            .map(|_| ()),
        Restore::Estimate(w, key, Some(value)) => {
            let mut update = WorkItemUpdate::default();
            if *key == "timeSpent" {
                update.time_spent = Some(value.clone());
            } else {
                update.remaining_estimate = Some(value.clone());
            }
            items.update(f.get(*w), &update).await
        }
        Restore::Estimate(w, key, None) => {
            items.clear_fields(f.get(*w), &[(*key).to_owned()]).await
        }
        Restore::LocalizedTitle(title) => {
            items
                .set_localized_title(&f.a, None, title.as_deref())
                .await
        }
        Restore::Steps(steps) => set_steps_of_b(writer, f, steps).await,
        Restore::RunTitle(title) => {
            tests
                .update(
                    &f.r,
                    &TestRunUpdate {
                        title: Some(title.clone()),
                        ..TestRunUpdate::default()
                    },
                )
                .await
        }
        Restore::RunReport(text) => tests.set_report(&f.r, text).await,
        Restore::RemoveRecord(index) => tests.remove_record(&f.r, *index).await,
        Restore::DeleteRunAttachment(name) => tests.delete_attachment(&f.r, name).await,
        Restore::Body(text) => {
            writer
                .documents()
                .update(
                    &f.d,
                    &DocumentUpdate {
                        body: Some(text.clone()),
                        ..DocumentUpdate::default()
                    },
                )
                .await
        }
    }
}

/// Set B's test steps. No steps: an empty `setTestSteps`, or when the server refuses that,
/// clearing the field `testSteps`.
pub async fn set_steps_of_b(
    writer: &Polarion,
    f: &Fixtures,
    steps: &[Vec<Text>],
) -> polarion::Result<()> {
    match writer.tests().set_steps(&f.b, steps).await {
        Err(e) if steps.is_empty() => writer
            .items()
            .clear_fields(&f.b, &["testSteps".to_owned()])
            .await
            .map_err(|_| e),
        other => other,
    }
}

/// The writes that bring the fixtures back to `baseline` (read-only): `None` when they match
/// it. Differences that no write of the suite can undo are an error, so nothing is written.
pub async fn restore_plan(
    report: &mut Report,
    reader: &Polarion,
    f: &Fixtures,
    defs: &Definitions,
    baseline: &Value,
) -> Result<Option<Vec<Restore>>, String> {
    let current = snapshot(reader, f, defs).await?;
    let diffs = differences(baseline, &current);
    if diffs.is_empty() {
        return Ok(None);
    }
    report.info(format!("{} differences from the baseline:", diffs.len()));
    for d in &diffs {
        report.info(format!("  {d}"));
    }
    let (plan, manual) = plan_restore(f, defs, baseline, &current);
    if !manual.is_empty() {
        return Err(format!(
            "differences the suite cannot undo (nothing was written; check the fixtures in the \
             browser): {}",
            manual.join("; ")
        ));
    }
    Ok(Some(plan))
}

/// Apply a plan of [`restore_plan`]; afterwards the fixtures must compare equal to the
/// baseline field by field.
pub async fn apply_restore(
    report: &mut Report,
    reader: &Polarion,
    writer: &Polarion,
    f: &Fixtures,
    defs: &Definitions,
    baseline: &Value,
    plan: Option<Vec<Restore>>,
) -> Result<(), String> {
    let Some(plan) = plan else {
        report.ok("the fixtures match the baseline");
        return Ok(());
    };
    for step in &plan {
        apply(writer, f, step)
            .await
            .map_err(|e| format!("restoring ({}): {e}", step.describe()))?;
        report.info(format!("restored: {}", step.describe()));
    }
    let after = snapshot(reader, f, defs).await?;
    let left = differences(baseline, &after);
    if left.is_empty() {
        report.ok(format!(
            "the fixtures were restored to the baseline ({} writes)",
            plan.len()
        ));
        Ok(())
    } else {
        Err(format!(
            "still different from the baseline after restoring: {}",
            left.join("; ")
        ))
    }
}

/// The repair after a step: bring the fixtures back to `baseline` ([`restore_plan`], then
/// [`apply_restore`]).
pub async fn normalize(
    report: &mut Report,
    reader: &Polarion,
    writer: &Polarion,
    f: &Fixtures,
    defs: &Definitions,
    baseline: &Value,
) -> Result<(), String> {
    let plan = restore_plan(report, reader, f, defs, baseline).await?;
    apply_restore(report, reader, writer, f, defs, baseline, plan).await
}

// ---------------------------------------------------------------- the write gate

/// Everything a writing tier needs, after every gate passed.
#[derive(Debug)]
pub struct Gate {
    pub settings: Settings,
    pub reader: Polarion,
    pub writer: Polarion,
    pub fixtures: Fixtures,
    pub defs: Definitions,
    /// The stored baseline state.
    pub baseline: Value,
    /// The back links of A and B before the run (gate 4).
    pub back_links: BTreeSet<String>,
    pub canary: ItemRef,
    /// The run's start, seconds since the epoch: the server's date of the identity probe's
    /// first change (gate 7), a few seconds earlier.
    pub start: i64,
}

/// Gates 1 to 8 (see the module documentation); `None` when one failed (it printed why).
/// Nothing is written unless gates 1 to 6 pass; gate 7 writes A's title twice, gate 8 restores.
pub async fn write_gate(report: &mut Report) -> Option<Gate> {
    let local_start = now() - 60;
    let settings = Settings::from_env();
    match check_live_writes(&settings) {
        Ok(()) => report.ok(format!(
            "gate 1: POLARION_LIVE_WRITES is the host of POLARION_URL ({})",
            settings.host
        )),
        Err(e) => {
            report.fail("gate 1", e);
            return None;
        }
    }
    let ids = FixtureIds::from_env().unwrap_or_else(|e| panic!("{e}"));
    let reader = reader(&settings);
    let fixtures = match Fixtures::resolve(&reader, &settings.project, &ids).await {
        Ok(f) => f,
        Err(e) => {
            report.fail("fixtures", e);
            let _ = reader.logout().await;
            return None;
        }
    };
    report.info(fixtures.describe());
    let writer = match gate_reads(report, &settings, &reader, &fixtures).await {
        Some(writer) => writer,
        None => {
            let _ = reader.logout().await;
            return None;
        }
    };
    let rest = gate_writes(report, &settings, &reader, &writer, &fixtures, local_start).await;
    match rest {
        Some((defs, baseline, back_links, canary, start)) => Some(Gate {
            settings,
            reader,
            writer,
            fixtures,
            defs,
            baseline,
            back_links,
            canary,
            start,
        }),
        None => {
            let _ = writer.logout().await;
            let _ = reader.logout().await;
            None
        }
    }
}

/// Gates 3, 4 and 2: the fixtures, their back links, and then the writer.
async fn gate_reads(
    report: &mut Report,
    settings: &Settings,
    reader: &Polarion,
    fixtures: &Fixtures,
) -> Option<Polarion> {
    if !check_identity(report, reader, fixtures).await {
        report.fail("gate 3", "the fixtures are not what the bootstrap made");
        return None;
    }
    match back_links(reader, fixtures).await {
        Ok((_, foreign)) => {
            if !check!(
                report,
                foreign.is_empty(),
                "gate 4: the back links of A and B come from fixtures only",
                "other items link to the fixtures: {foreign:?}"
            ) {
                return None;
            }
        }
        Err(e) => {
            report.fail("gate 4", e);
            return None;
        }
    }
    match writer(settings, fixtures) {
        Ok(writer) => {
            report.ok("gate 2: the writer is restricted to the URIs of A, B, R and D");
            Some(writer)
        }
        Err(e) => {
            report.fail("gate 2", e);
            None
        }
    }
}

/// Gates 5 to 8: the canary, the baseline and what restoring it takes (read-only), the
/// identity probe, and then the restore.
async fn gate_writes(
    report: &mut Report,
    settings: &Settings,
    reader: &Polarion,
    writer: &Polarion,
    fixtures: &Fixtures,
    local_start: i64,
) -> Option<(Definitions, Value, BTreeSet<String>, ItemRef, i64)> {
    let canary = canary(report, reader, writer, fixtures).await?;
    let defs = match definitions(reader, fixtures).await {
        Ok(defs) => defs,
        Err(e) => {
            report.fail("custom field definitions", e);
            return None;
        }
    };
    let baseline = match load_baseline(settings, fixtures) {
        Ok(Some(stored)) => stored,
        Ok(None) => {
            report.fail(
                "gate 6",
                format!(
                    "there is no baseline ({}): run the bootstrap (`cargo run -p polarion \
                     --example live_fixtures`, see docs/testing.md), which checks the fixtures \
                     and stores it",
                    baseline_path(settings).map_or_else(|e| e, |p| p.display().to_string())
                ),
            );
            return None;
        }
        Err(e) => {
            report.fail("gate 6", e);
            return None;
        }
    };
    let plan = match restore_plan(report, reader, fixtures, &defs, &baseline).await {
        Ok(plan) => {
            report.ok(format!(
                "gate 6: the baseline exists, {}",
                match &plan {
                    None => "and the fixtures match it".to_owned(),
                    Some(plan) => format!("and {} writes restore it", plan.len()),
                }
            ));
            plan
        }
        Err(e) => {
            report.fail("gate 6", e);
            return None;
        }
    };
    let start = identity_probe(report, reader, writer, fixtures, local_start).await?;
    if let Err(e) = apply_restore(report, reader, writer, fixtures, &defs, &baseline, plan).await {
        report.fail("gate 8", e);
        return None;
    }
    report.ok("gate 8: the fixtures are at the baseline");
    // Gate 4 again, now as the state the audit compares with.
    let links = match back_links(reader, fixtures).await {
        Ok((all, _)) => all,
        Err(e) => {
            report.fail("back links", e);
            return None;
        }
    };
    Some((defs, baseline, links, canary, start))
}

/// Gate 7, the identity probe and the run's first write: A's title gets a token and is
/// restored; A's history must then show these changes, all by `POLARION_LIVE_USER`, dated now.
/// Only then may a write name the user (assignee, approver, work records, `@user` fields), and
/// only then does the audit, which looks for the user's changes, see what the token writes.
/// The start of the run (the server's date of the first change, 5 seconds earlier), or `None`
/// when the probe failed (no further write).
pub async fn identity_probe(
    report: &mut Report,
    reader: &Polarion,
    writer: &Polarion,
    f: &Fixtures,
    local_start: i64,
) -> Option<i64> {
    let label = "gate 7: the token belongs to POLARION_LIVE_USER";
    let title = match reader
        .items()
        .get_fields(&f.a, &Fields::new(["title"]))
        .await
    {
        Ok(item) => item.title.unwrap_or_default(),
        Err(e) => {
            report.fail(format!("{label}: A's title"), e);
            return None;
        }
    };
    if !title.starts_with(MARKER_A) {
        report.fail(label, format!("A's title {title:?} has no marker"));
        return None;
    }
    let before = match reader.items().history(&f.a).await {
        Ok(history) => history.len(),
        Err(e) => {
            report.fail(format!("{label}: A's history"), e);
            return None;
        }
    };
    let token = run_token();
    let probe = format!("{MARKER_A} identity probe {token}");
    let changed = writer.items().update(&f.a, &title_update(&probe)).await;
    // Restored even when the change failed: it may have been applied.
    let restored = writer.items().update(&f.a, &title_update(&title)).await;
    if let Err(e) = changed {
        report.fail(format!("{label}: A's title with the probe"), e);
        return None;
    }
    if let Err(e) = restored {
        report.fail(format!("{label}: A's title restored"), e);
        return None;
    }
    let history = match reader.items().history(&f.a).await {
        Ok(history) => history,
        Err(e) => {
            report.fail(format!("{label}: A's history"), e);
            return None;
        }
    };
    let new = history.get(before..).unwrap_or_default();
    let user = f.user.as_str();
    let by_user = |c: &&Change| {
        c.user
            .as_deref()
            .is_some_and(|u| u.eq_ignore_ascii_case(user))
    };
    let dates: Vec<Option<i64>> = new
        .iter()
        .map(|c| c.date.as_deref().and_then(parse_iso))
        .collect();
    let first = dates.iter().flatten().min().copied();
    // A change of the probe's title, when the history lists field changes at all.
    let lists_fields = new.iter().any(|c| !c.changes.is_empty());
    let shows_probe = new.iter().any(|c| {
        c.changes.iter().any(|fc| {
            fc.field == "title"
                && serde_json::to_string(&fc.after).is_ok_and(|after| after.contains(&token))
        })
    });
    let problem = if new.is_empty() {
        Some("A's history shows no change after the probe: the audit could not see the run".into())
    } else if let Some(c) = new.iter().find(|c| !by_user(c)) {
        Some(format!(
            "A's history shows a change by {:?}, not by POLARION_LIVE_USER ({user}): the token \
             belongs to someone else, or the history names users differently",
            c.user
        ))
    } else if dates.iter().any(Option::is_none) {
        Some(format!(
            "A's history has dates that do not parse: {:?}",
            new.iter().map(|c| c.date.clone()).collect::<Vec<_>>()
        ))
    } else if first.is_some_and(|t| t < local_start - 600) {
        Some(format!(
            "the probe's change is dated {:?}, more than 10 minutes before this machine's time: \
             the clocks differ, or the history is not oldest first",
            new.first().and_then(|c| c.date.clone())
        ))
    } else if lists_fields && !shows_probe {
        Some("the new changes in A's history do not show the probe's title".to_owned())
    } else {
        None
    };
    match (problem, first) {
        (None, Some(first)) => {
            report.ok(format!(
                "{label}: A's history shows the probe ({} changes) by {user} at {:?}",
                new.len(),
                new.first().and_then(|c| c.date.clone())
            ));
            Some(first - 5)
        }
        (problem, _) => {
            report.fail(
                label,
                format!(
                    "{}; nothing else is written",
                    problem.unwrap_or_else(|| "no date".into())
                ),
            );
            None
        }
    }
}

impl Gate {
    /// After a step: FAIL with `result`'s error, make sure no transaction is left open, then
    /// compare the fixtures with the baseline field by field; on a difference FAIL and restore
    /// them. `false` when a transaction stays open or the fixtures could not be restored: no
    /// further writes.
    pub async fn after(&self, report: &mut Report, step: &str, result: Result<(), String>) -> bool {
        if let Err(e) = result {
            report.fail(step, e);
        }
        if !self.no_transaction(report, step).await {
            return false;
        }
        match snapshot(&self.reader, &self.fixtures, &self.defs).await {
            Ok(current) => {
                let diffs = differences(&self.baseline, &current);
                if diffs.is_empty() {
                    report.ok(format!("{step}: the fixtures match the baseline"));
                    return true;
                }
                report.fail(
                    format!("{step}: the fixtures differ from the baseline"),
                    diffs.join("; "),
                );
            }
            Err(e) => report.fail(format!("{step}: reading the fixtures"), e),
        }
        match normalize(
            report,
            &self.reader,
            &self.writer,
            &self.fixtures,
            &self.defs,
            &self.baseline,
        )
        .await
        {
            Ok(()) => true,
            Err(e) => {
                report.fail(
                    format!("{step}: restoring the baseline"),
                    format!("{e}; no further writes"),
                );
                false
            }
        }
    }

    /// Whether the writer's session has no explicit transaction open (one left open is rolled
    /// back once): writes into an open transaction would be discarded at logout.
    pub async fn no_transaction(&self, report: &mut Report, step: &str) -> bool {
        match self.writer.in_transaction().await {
            Ok(false) => return true,
            Ok(true) => report.fail(
                format!("{step}: a transaction was left open"),
                "rolling it back",
            ),
            Err(e) => {
                report.fail(
                    format!("{step}: transactionExists"),
                    format!("{e}; no further writes"),
                );
                return false;
            }
        }
        let rolled_back = self.writer.rollback().await;
        match self.writer.in_transaction().await {
            Ok(false) => true,
            other => {
                report.fail(
                    format!("{step}: the transaction is still open"),
                    format!(
                        "rollback: {rolled_back:?}, transactionExists: {other:?}; no further writes"
                    ),
                );
                false
            }
        }
    }

    /// Log both sessions out.
    pub async fn close(self, report: &mut Report) {
        report.result("logout (writer)", self.writer.logout().await);
        report.result("logout (reader)", self.reader.logout().await);
    }
}

/// What the audit can see of a change of A: its `updated` and its history.
#[derive(Debug, Clone)]
pub struct Mark {
    updated: Option<String>,
    history: usize,
    changes: Vec<Change>,
}

/// A's [`Mark`] before a write of `kind` (attachments, work records, approvals, comments).
pub async fn mark(g: &Gate) -> Result<Mark, String> {
    let f = &g.fixtures;
    let updated = g
        .reader
        .items()
        .get_fields(&f.a, &Fields::new(["updated"]))
        .await
        .map_err(|e| format!("A's updated: {e}"))?
        .updated;
    let changes = g
        .reader
        .items()
        .history(&f.a)
        .await
        .map_err(|e| format!("A's history: {e}"))?;
    Ok(Mark {
        updated,
        history: changes.len(),
        changes,
    })
}

/// After a write of `kind` to A: whether it changed A's `updated` and history by the user. If
/// not, the audit (which finds work items by `updated` and reads their history) cannot see
/// such writes to other work items: a `SKIP` says so.
pub async fn audit_sees(report: &mut Report, g: &Gate, kind: &str, before: &Mark) {
    let label = format!("audit: {kind} written to A show in its `updated` and history");
    let after = match mark(g).await {
        Ok(after) => after,
        Err(e) => {
            report.fail(label, e);
            return;
        }
    };
    let user = g.fixtures.user.as_str();
    let by_user = after
        .changes
        .get(before.history..)
        .unwrap_or_default()
        .iter()
        .any(|c| by_user_since(c, user, g.start));
    if after.updated != before.updated && by_user {
        report.ok(label);
    } else {
        report.skip(
            label,
            format!(
                "A's updated {:?} -> {:?}, a new change by {user} in its history: {by_user}; the \
                 audit cannot see {kind} written to other work items",
                before.updated, after.updated
            ),
        );
    }
}

// ---------------------------------------------------------------- custom field values

/// A custom field's value type for grouping: the schema type, object enumerations such as
/// `@user` apart, `[]` for multi-valued fields.
pub fn type_key(def: &FieldDefinition) -> String {
    let kind = def.kind.as_deref().unwrap_or("string").to_ascii_lowercase();
    let objects = def
        .enum_id
        .as_deref()
        .filter(|e| e.starts_with('@'))
        .unwrap_or("");
    let multi = if def.multi { "[]" } else { "" };
    format!("{kind}{objects}{multi}")
}

/// One custom field per value type: the first by key, preferring fields that depend on no other.
pub fn representatives(defs: &BTreeMap<String, FieldDefinition>) -> Vec<&FieldDefinition> {
    let mut by_type: BTreeMap<String, &FieldDefinition> = BTreeMap::new();
    for def in defs.values() {
        let entry = by_type.entry(type_key(def)).or_insert(def);
        if entry.depends_on.is_some() && def.depends_on.is_none() {
            *entry = def;
        }
    }
    by_type.into_values().collect()
}

/// The option ids of a value (one, or a list).
fn option_ids(value: Option<&FieldValue>) -> Vec<String> {
    match value {
        Some(FieldValue::List(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        Some(v) => v.as_str().map(|s| vec![s.to_owned()]).unwrap_or_default(),
        None => Vec::new(),
    }
}

/// A safe test value for a custom field that differs from `current`: test data for text, an
/// existing option for enums, the user for `@user` fields (when known); `None` for types the
/// suite does not write (other object enumerations, tables, ...).
pub async fn test_value(
    reader: &Polarion,
    project: &str,
    def: &FieldDefinition,
    current: Option<&FieldValue>,
    user: Option<&str>,
    token: &str,
) -> Result<Option<FieldValue>, String> {
    let kind = def.kind.as_deref().unwrap_or("string").to_ascii_lowercase();
    let current_ids = option_ids(current);
    let toggle = |a: &str, b: &str| {
        if current
            .and_then(FieldValue::as_str)
            .is_some_and(|c| c.starts_with(a))
        {
            b.to_owned()
        } else {
            a.to_owned()
        }
    };
    let one = match kind.as_str() {
        "string" => FieldValue::String(format!("{TEST_DATA} {token}")),
        "text" => FieldValue::Text(html(format!("<p>{TEST_DATA} {token}</p>"))),
        "boolean" => FieldValue::Bool(!matches!(current, Some(FieldValue::Bool(true)))),
        "int" | "integer" | "long" | "short" => FieldValue::Int(match current {
            Some(FieldValue::Int(7)) => 8,
            _ => 7,
        }),
        "float" | "double" => FieldValue::Float(match current {
            Some(FieldValue::Float(x)) if (*x - 2.5).abs() < 1e-6 => 3.5,
            _ => 2.5,
        }),
        "decimal" | "currency" => FieldValue::Decimal(toggle("12.5", "13.5")),
        "date" => FieldValue::Date(toggle("2001-02-03", "2001-02-04")),
        "datetime" => FieldValue::DateTime(toggle("2001-02-03T04:05:06Z", "2001-02-04T04:05:06Z")),
        "time" => FieldValue::Time(toggle("04:05:06", "05:06:07")),
        "enumoptionid" => {
            let pick = match def.enum_id.as_deref() {
                Some("@user") => match user {
                    Some(u) if !current_ids.iter().any(|c| c == u) => u.to_owned(),
                    _ => return Ok(None),
                },
                // References to other objects (documents, plans, test runs, ...).
                Some(e) if e.starts_with('@') => return Ok(None),
                Some(e) => {
                    let options = reader
                        .schema()
                        .enum_options(project, e)
                        .await
                        .map_err(|err| format!("options of {e}: {err}"))?;
                    let mut usable: Vec<&EnumOption> = options
                        .iter()
                        .filter(|o| !o.hidden && !o.id.is_empty() && !current_ids.contains(&o.id))
                        .collect();
                    usable.sort_by_key(|o| (!o.default, o.sequence));
                    match usable.first() {
                        Some(o) => o.id.clone(),
                        None => return Ok(None),
                    }
                }
                None => return Ok(None),
            };
            FieldValue::Enum(pick)
        }
        _ => return Ok(None),
    };
    Ok(Some(if def.multi {
        FieldValue::List(vec![one])
    } else {
        one
    }))
}

/// Text without tags, the common character references resolved, white space (a no-break space
/// included) collapsed.
pub fn text_only(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace('\u{a0}', " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether the value read back is the value written (dates by their date, numbers by value,
/// rich text by its text, lists as sets).
pub fn same_value(written: &FieldValue, read: Option<&FieldValue>) -> bool {
    let Some(read) = read else {
        return false;
    };
    let number = |v: &FieldValue| match v {
        FieldValue::Int(i) => Some(*i as f64),
        FieldValue::Float(x) => Some(*x),
        FieldValue::Decimal(s) | FieldValue::String(s) => s.trim().parse().ok(),
        _ => None,
    };
    match written {
        FieldValue::Float(_) | FieldValue::Int(_) | FieldValue::Decimal(_) => {
            match (number(written), number(read)) {
                (Some(a), Some(b)) => (a - b).abs() < 1e-6,
                _ => false,
            }
        }
        FieldValue::Date(d) => read.as_str().is_some_and(|r| r.starts_with(d.as_str())),
        FieldValue::DateTime(d) => match (parse_iso(d), read.as_str().and_then(parse_iso)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        },
        FieldValue::Text(t) => match read {
            FieldValue::Text(r) => text_only(&r.content) == text_only(&t.content),
            _ => false,
        },
        FieldValue::List(_) => {
            let a: BTreeSet<String> = option_ids(Some(written)).into_iter().collect();
            let b: BTreeSet<String> = option_ids(Some(read)).into_iter().collect();
            a == b
        }
        FieldValue::Enum(e) | FieldValue::String(e) | FieldValue::Time(e) => {
            read.as_str() == Some(e.as_str())
        }
        other => other == read,
    }
}

// ---------------------------------------------------------------- comment and work record URIs

/// URIs of existing comments and work records with the URI of their owner, to check
/// `uri::owner_of` against.
#[derive(Debug, Default)]
pub struct OwnerSamples {
    pub comments: Vec<(String, String)>,
    pub work_records: Vec<(String, String)>,
    pub document_comments: Vec<(String, String)>,
}

/// Samples from the 20 most recently updated work items and documents (read-only).
pub async fn owner_samples(reader: &Polarion, project: &str) -> Result<OwnerSamples, String> {
    let mut samples = OwnerSamples::default();
    let items = reader
        .items()
        .query(
            &Query::new("")
                .project(project)
                .sort("~updated")
                .limit(20)
                .fields(Fields::new(["id", "comments", "workRecords"])),
        )
        .await
        .map_err(|e| format!("work items with comments: {e}"))?;
    for item in items {
        let owner = match (&item.uri, &item.id) {
            (Some(uri), _) => base_uri(uri).to_owned(),
            (None, Some(id)) => ItemRef::new(project, id).uri,
            (None, None) => continue,
        };
        let owner = owner.as_str();
        for c in &item.comments {
            if let Some(uri) = &c.uri {
                samples.comments.push((owner.to_owned(), uri.clone()));
            }
        }
        for r in &item.work_records {
            if let Some(uri) = &r.uri {
                samples.work_records.push((owner.to_owned(), uri.clone()));
            }
        }
    }
    let documents = reader
        .documents()
        .query(
            &Query::new("")
                .project(project)
                .sort("~updated")
                .limit(20)
                .fields(Fields::new([
                    "id",
                    "moduleFolder",
                    "moduleName",
                    "comments",
                ])),
        )
        .await
        .map_err(|e| format!("documents with comments: {e}"))?;
    for doc in documents {
        let owner = match (&doc.uri, &doc.space, &doc.name) {
            (Some(uri), _, _) => base_uri(uri).to_owned(),
            (None, Some(space), Some(name)) => DocumentRef::new(project, space, name).uri,
            _ => continue,
        };
        let owner = owner.as_str();
        for c in &doc.comments {
            if let Some(uri) = &c.uri {
                samples
                    .document_comments
                    .push((owner.to_owned(), uri.clone()));
            }
        }
    }
    Ok(samples)
}

/// `OK` when `uri::owner_of` gives the owner of every sample; `None` (a `SKIP`) without samples.
pub fn check_owner_samples(
    report: &mut Report,
    what: &str,
    samples: &[(String, String)],
) -> Option<bool> {
    let Some((owner, owned)) = samples.first() else {
        report.skip(format!("uri::owner_of on {what}"), "no sample found");
        return None;
    };
    report.info(format!("{what}, e.g. {owned} (owner {owner})"));
    let wrong: Vec<String> = samples
        .iter()
        .filter(|(owner, owned)| polarion::uri::owner_of(owned).as_deref() != Some(owner.as_str()))
        .map(|(owner, owned)| {
            format!(
                "{owned} (owner {owner}, parsed {:?})",
                polarion::uri::owner_of(owned)
            )
        })
        .collect();
    Some(check!(
        report,
        wrong.is_empty(),
        format!("uri::owner_of on {} {what}", samples.len()),
        "wrong owner: {wrong:?}"
    ))
}

// ---------------------------------------------------------------- the index

/// How long to wait for the index to show a change.
pub const INDEX_TIMEOUT: Duration = Duration::from_secs(180);
const POLL: Duration = Duration::from_secs(3);

/// Poll `probe` until it gives `true` (`Ok(true)`), or `timeout` passed (`Ok(false)`); an error
/// of the probe ends the wait.
pub async fn wait_for<F, Fut>(timeout: Duration, mut probe: F) -> Result<bool, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool, String>>,
{
    let start = Instant::now();
    loop {
        if probe().await? {
            return Ok(true);
        }
        if start.elapsed() > timeout {
            return Ok(false);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// The work items a Lucene query finds in the project (at most 10).
async fn found(reader: &Polarion, project: &str, lucene: &str) -> Result<Vec<ItemRef>, String> {
    reader
        .items()
        .query_refs(&Query::new(lucene).project(project).limit(10))
        .await
        .map_err(|e| format!("query `{lucene}`: {e}"))
}

fn title_update(title: &str) -> WorkItemUpdate {
    WorkItemUpdate {
        title: Some(title.to_owned()),
        ..WorkItemUpdate::default()
    }
}

/// The last change of every writing tier, so the audit sees the run: A's title gets a token,
/// the index must find A by it, the title is restored, and the index must no longer find it.
/// `true` when the index showed both changes.
pub async fn index_probe(report: &mut Report, g: &Gate) -> bool {
    let f = &g.fixtures;
    let title = match g
        .reader
        .items()
        .get_fields(&f.a, &Fields::new(["title"]))
        .await
    {
        Ok(item) => item.title.unwrap_or_default(),
        Err(e) => {
            report.fail("index probe: A's title", e);
            return false;
        }
    };
    if !title.starts_with(MARKER_A) {
        report.fail("index probe", format!("A's title {title:?} has no marker"));
        return false;
    }
    let token = run_token();
    let lucene = format!("title:{token}");
    let (reader, project, a, query) = (&g.reader, f.project.as_str(), &f.a, lucene.as_str());
    let probe = format!("{MARKER_A} index probe {token}");
    if let Err(e) = g.writer.items().update(a, &title_update(&probe)).await {
        report.fail("index probe: A's title with the token", e);
        return false;
    }
    let seen = wait_for(INDEX_TIMEOUT, move || async move {
        let refs = found(reader, project, query).await?;
        Ok(refs.len() == 1 && refs.iter().any(|r| base_uri(&r.uri) == a.uri))
    })
    .await;
    let restored = g.writer.items().update(a, &title_update(&title)).await;
    let mut ok = match seen {
        Ok(true) => {
            report.ok("index probe: the index finds A by its new title");
            true
        }
        Ok(false) => {
            report.fail(
                "index probe",
                format!("the index does not find A by its new title after {INDEX_TIMEOUT:?}"),
            );
            false
        }
        Err(e) => {
            report.fail("index probe", e);
            false
        }
    };
    if let Err(e) = restored {
        report.fail("index probe: A's title restored", e);
        return false;
    }
    let gone = wait_for(INDEX_TIMEOUT, move || async move {
        Ok(found(reader, project, query).await?.is_empty())
    })
    .await;
    ok &= match gone {
        Ok(true) => {
            report.ok("index probe: the index shows A's restored title (the run's last change)");
            true
        }
        Ok(false) => {
            report.fail(
                "index probe",
                format!("the index still finds A by the token after {INDEX_TIMEOUT:?}"),
            );
            false
        }
        Err(e) => {
            report.fail("index probe", e);
            false
        }
    };
    ok
}

// ---------------------------------------------------------------- the audit

/// At most this many updated work items (and documents, and test runs) are audited; more fail
/// the audit.
pub const AUDIT_LIMIT: u32 = 500;

/// At most this many revisions of a document or test run since the run's start are read.
const REVISION_LIMIT: usize = 50;

/// Whether a change is by `user` and not older than `start` (an unreadable date counts as newer).
fn by_user_since(change: &Change, user: &str, start: i64) -> bool {
    change
        .user
        .as_deref()
        .is_some_and(|u| u.eq_ignore_ascii_case(user))
        && change
            .date
            .as_deref()
            .is_none_or(|d| parse_iso(d).is_none_or(|t| t >= start))
}

/// Whether an object's `updated` is not older than `start` (unreadable or missing: it might
/// be).
fn updated_since(updated: Option<&str>, start: i64) -> bool {
    updated.is_none_or(|t| parse_iso(t).is_none_or(|t| t >= start))
}

/// The revisions of the object `uri` since `start` that are by `user` (read-only:
/// `getRevisions`, then each revision from the newest back to `start`).
pub async fn revisions_by_since(
    reader: &Polarion,
    uri: &str,
    user: &str,
    start: i64,
) -> Result<Vec<String>, String> {
    let revisions = reader
        .soap()
        .tracker()
        .get_revisions(uri)
        .await
        .map_err(|e| format!("the revisions of {uri}: {e}"))?;
    let mut by_user = Vec::new();
    for (n, name) in revisions.iter().rev().enumerate() {
        if n == REVISION_LIMIT {
            return Err(format!(
                "{uri} has more than {REVISION_LIMIT} revisions since the run's start"
            ));
        }
        let revision = reader
            .revisions()
            .get("default", name)
            .await
            .map_err(|e| format!("revision {name} of {uri}: {e}"))?;
        match revision.created.as_deref().and_then(parse_iso) {
            Some(t) if t < start => break,
            Some(_) => {}
            None => {
                return Err(format!(
                    "revision {name} of {uri} has no readable date ({:?})",
                    revision.created
                ));
            }
        }
        if revision
            .author
            .as_deref()
            .is_some_and(|a| a.eq_ignore_ascii_case(user))
        {
            by_user.push(format!("revision {name} at {:?}", revision.created));
        }
    }
    Ok(by_user)
}

/// The post-run audit (read-only), of `POLARION_PROJECT` only: gate 2 made sure every write
/// target is in it, so the guard and the SOAP write filter refuse writes to other projects
/// (the report says so with a `SKIP`). Since the run's start (the server's date of the identity
/// probe, gate 7):
/// - no work item but A and B was changed by `POLARION_LIVE_USER` (the history of every work
///   item `updated:[<start day - 1> TO 99991231]` finds);
/// - no document but D, and no test run but R, was changed by the user (their `updatedBy` and
///   revisions, for those updated since the start);
/// - the back links of A and B and D's work items are as before.
///
/// An empty, failing or truncated query is a FAIL. `wrote_d`: the tier changed D, so the
/// document query must see it.
pub async fn audit(report: &mut Report, g: &Gate, wrote_d: bool) {
    let (f, reader) = (&g.fixtures, &g.reader);
    let since = lucene_date(g.start - 86_400);
    let lucene = format!("updated:[{since} TO 99991231]");
    report.skip(
        "audit: other projects",
        format!(
            "the audit covers {} only; every write target is in it, so the guard and the SOAP \
             write filter refuse writes to other projects",
            f.project
        ),
    );
    audit_items(report, g, &lucene).await;
    audit_documents(report, g, &lucene, wrote_d).await;
    audit_runs(report, g, &lucene).await;
    match back_links(reader, f).await {
        Ok((all, _)) => {
            check!(
                report,
                all == g.back_links,
                "audit: the back links of A and B are as before the run",
                "before {:?}, now {all:?}",
                g.back_links
            );
        }
        Err(e) => report.fail("audit: back links", e),
    }
    match reader.documents().work_item_refs(&f.d, None, true).await {
        Ok(list) => {
            check!(
                report,
                list.is_empty(),
                "audit: D holds no work items",
                "it holds {:?}",
                list.iter().map(ToString::to_string).collect::<Vec<_>>()
            );
        }
        Err(e) => report.fail("audit: D's work items", e),
    }
}

async fn audit_items(report: &mut Report, g: &Gate, lucene: &str) {
    let (f, reader, user) = (&g.fixtures, &g.reader, g.fixtures.user.as_str());
    let query = Query::new(lucene).project(&f.project);
    let count = match reader.items().count(&query).await {
        Err(e) => {
            report.fail("audit: the count of updated work items", e);
            return;
        }
        Ok(0) => {
            report.fail(
                "audit",
                format!("`{lucene}` finds no work item, not even the fixtures"),
            );
            return;
        }
        Ok(count) if count > AUDIT_LIMIT => {
            report.fail(
                "audit",
                format!("`{lucene}` finds {count} work items, more than {AUDIT_LIMIT} to audit"),
            );
            return;
        }
        Ok(count) => count,
    };
    // Items updated between the count and the query must not push others out of the result.
    let limit = count + 10;
    let refs = match reader.items().query_refs(&query.clone().limit(limit)).await {
        Ok(refs) => refs,
        Err(e) => {
            report.fail("audit: the updated work items", e);
            return;
        }
    };
    if refs.len() >= limit as usize {
        report.fail(
            "audit",
            format!(
                "`{lucene}` gives {} work items, the limit of the query: the result may be cut",
                refs.len()
            ),
        );
        return;
    }
    // Every writing tier changes A (at least the identity probe does).
    check!(
        report,
        refs.iter().any(|r| f.item(r) == Some(Which::A)),
        "audit: the query sees the change of A",
        "`{lucene}` does not find A: the index lags or the query is wrong"
    );
    let mut checked = 0;
    let mut changed = Vec::new();
    for item in refs.iter().filter(|r| f.item(r).is_none()) {
        match reader.items().history(item).await {
            Ok(history) => {
                checked += 1;
                for c in history.iter().filter(|c| by_user_since(c, user, g.start)) {
                    changed.push(format!("{item} revision {:?} at {:?}", c.revision, c.date));
                }
            }
            Err(e) => report.fail(format!("audit: the history of {item}"), e),
        }
    }
    check!(
        report,
        changed.is_empty(),
        format!(
            "audit: none of the {checked} other work items found by `{lucene}` was changed by \
             {user} during the run"
        ),
        "changed by {user}: {changed:?}"
    );
}

async fn audit_documents(report: &mut Report, g: &Gate, lucene: &str, wrote_d: bool) {
    let (f, reader, user) = (&g.fixtures, &g.reader, g.fixtures.user.as_str());
    let fields = Fields::new([
        "id",
        "title",
        "moduleFolder",
        "moduleName",
        "updated",
        "updatedBy",
        "project",
    ]);
    let query = Query::new(lucene)
        .project(&f.project)
        .limit(AUDIT_LIMIT + 1)
        .fields(fields);
    let docs = match reader.documents().query(&query).await {
        Ok(docs) => docs,
        Err(e) => {
            report.fail("audit: the query of updated documents", e);
            return;
        }
    };
    if docs.len() > AUDIT_LIMIT as usize {
        report.fail(
            "audit",
            format!("`{lucene}` finds more than {AUDIT_LIMIT} documents to audit"),
        );
        return;
    }
    let is_d = |uri: Option<&str>| uri.map(base_uri) == Some(f.d.uri.as_str());
    if wrote_d {
        // The detector itself: D shows the user as the one who changed it.
        let d = docs.iter().find(|d| is_d(d.uri.as_deref()));
        check!(
            report,
            d.is_some_and(|d| d
                .updated_by
                .as_deref()
                .is_some_and(|u| u.eq_ignore_ascii_case(user))),
            format!("audit: the document query sees D's change by {user}"),
            "`{lucene}` gives D as {:?} (updatedBy {:?})",
            d.map(|d| &d.uri),
            d.map(|d| &d.updated_by)
        );
    }
    let mut checked = 0;
    let mut changed = Vec::new();
    for doc in docs.iter().filter(|d| !is_d(d.uri.as_deref())) {
        if !updated_since(doc.updated.as_deref(), g.start) {
            continue;
        }
        checked += 1;
        let name = format!("{:?}/{:?}", doc.space, doc.name);
        if doc
            .updated_by
            .as_deref()
            .is_some_and(|u| u.eq_ignore_ascii_case(user))
        {
            changed.push(format!("{name}: updated by {user} at {:?}", doc.updated));
        }
        // Someone else may have changed it after the user did: its revisions since the start.
        let Some(uri) = doc.uri.as_deref() else {
            report.fail(
                format!("audit: the document {name}"),
                "it has no URI: its revisions cannot be read",
            );
            continue;
        };
        match revisions_by_since(reader, uri, user, g.start).await {
            Ok(list) => changed.extend(list.into_iter().map(|r| format!("{name}: {r}"))),
            Err(e) => report.fail(format!("audit: the document {name}"), e),
        }
    }
    check!(
        report,
        changed.is_empty(),
        format!(
            "audit: none of the {checked} other documents updated since the run's start was \
             changed by {user}"
        ),
        "changed by {user}: {changed:?}"
    );
}

async fn audit_runs(report: &mut Report, g: &Gate, lucene: &str) {
    let (f, reader, user) = (&g.fixtures, &g.reader, g.fixtures.user.as_str());
    let query = Query::new(lucene)
        .project(&f.project)
        .limit(AUDIT_LIMIT + 1)
        .fields(Fields::new(["id", "title", "updated", "author"]));
    let runs = match reader.tests().search(&query).await {
        Ok(runs) => runs,
        Err(e) => {
            report.fail("audit: the query of updated test runs", e);
            return;
        }
    };
    if runs.len() > AUDIT_LIMIT as usize {
        report.fail(
            "audit",
            format!("`{lucene}` finds more than {AUDIT_LIMIT} test runs to audit"),
        );
        return;
    }
    let mut checked = 0;
    let mut changed = Vec::new();
    for run in &runs {
        let name = format!("{:?}", run.id);
        let Some(uri) = run.uri.as_deref() else {
            report.fail(
                format!("audit: the test run {name}"),
                "it has no URI: its revisions cannot be read",
            );
            continue;
        };
        if base_uri(uri) == f.r || !updated_since(run.updated.as_deref(), g.start) {
            continue;
        }
        checked += 1;
        match revisions_by_since(reader, uri, user, g.start).await {
            Ok(list) => changed.extend(list.into_iter().map(|r| format!("{name}: {r}"))),
            Err(e) => report.fail(format!("audit: the test run {name}"), e),
        }
    }
    check!(
        report,
        changed.is_empty(),
        format!(
            "audit: none of the {checked} other test runs updated since the run's start was \
             changed by {user}"
        ),
        "changed by {user}: {changed:?}"
    );
}

/// Wait (read-only) until the index shows A as it is now: the query by A's id returns the
/// `updated` A has. Before the audit when a tier stops early, without the index probe's
/// writes; `true` when it did.
pub async fn wait_for_index(report: &mut Report, g: &Gate) -> bool {
    let f = &g.fixtures;
    let updated = match g
        .reader
        .items()
        .get_fields(&f.a, &Fields::new(["updated"]))
        .await
    {
        Ok(item) => item.updated,
        Err(e) => {
            report.fail("index: A's updated", e);
            return false;
        }
    };
    let Some(id) = f.a.id.clone() else {
        report.fail("index", "A's URI has no id");
        return false;
    };
    let query = Query::new(format!("id:{id}"))
        .project(&f.project)
        .limit(1)
        .fields(Fields::new(["id", "updated"]));
    let (reader, query, updated, id) = (&g.reader, &query, &updated, id.as_str());
    let seen = wait_for(INDEX_TIMEOUT, move || async move {
        let found = reader
            .items()
            .query(query)
            .await
            .map_err(|e| format!("query `id:{id}`: {e}"))?;
        Ok(found.first().is_some_and(|w| w.updated == *updated))
    })
    .await;
    report.check(
        "index: the query shows A's last change (before the audit)",
        seen == Ok(true),
        format!("{seen:?} after {INDEX_TIMEOUT:?}: the audit may miss the last changes"),
    )
}

// ---------------------------------------------------------------- front ends

/// A front-end binary with every `POLARION_*` variable of this process removed (the write
/// settings, the MCP config and tools included), then the connection and
/// `POLARION_WRITE_TARGETS=targets`.
pub fn front_end(program: &str, settings: &Settings, targets: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("POLARION_")
        {
            cmd.env_remove(&name);
        }
    }
    cmd.env("POLARION_URL", &settings.url)
        .env("POLARION_TOKEN", &settings.token)
        .env("POLARION_PROJECT", &settings.project)
        .env("POLARION_WRITE_TARGETS", targets);
    if settings.insecure {
        cmd.env("POLARION_INSECURE", "1");
    }
    cmd
}

// ---------------------------------------------------------------- time

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// A token for this run made of letters only, so it is one search term.
pub fn run_token() -> String {
    let mut n = now().unsigned_abs();
    let mut letters = String::new();
    while n > 0 {
        letters.insert(0, char::from(b'a' + u8::try_from(n % 26).unwrap_or(0)));
        n /= 26;
    }
    format!("prs{letters}")
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let yoe = year - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `YYYY-MM-DD` (UTC).
pub fn date(epoch: i64) -> String {
    let (y, m, d) = civil_from_days(epoch.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYYMMDD` (UTC), for Lucene date ranges.
pub fn lucene_date(epoch: i64) -> String {
    let (y, m, d) = civil_from_days(epoch.div_euclid(86_400));
    format!("{y:04}{m:02}{d:02}")
}

/// `YYYY-MM-DDTHH:MM:SSZ`.
pub fn iso(epoch: i64) -> String {
    let secs = epoch.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        date(epoch),
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// Seconds since the epoch of an ISO 8601 date or date and time (`2024-01-31`,
/// `2024-01-31T12:34:56.789+02:00`, `…Z`; without a zone: UTC).
pub fn parse_iso(text: &str) -> Option<i64> {
    let b = text.trim().as_bytes();
    let digits = |from: usize, len: usize| -> Option<i64> {
        let part = b.get(from..from + len)?;
        part.iter()
            .all(u8::is_ascii_digit)
            .then(|| part.iter().fold(0, |n, d| n * 10 + i64::from(d - b'0')))
    };
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    let (year, month, day) = (digits(0, 4)?, digits(5, 2)?, digits(8, 2)?);
    let mut seconds = days_from_civil(year, month, day) * 86_400;
    let mut i = 10;
    if matches!(b.get(i), Some(b'T' | b' ')) {
        if b.get(i + 3) != Some(&b':') {
            return None;
        }
        seconds += digits(i + 1, 2)? * 3600 + digits(i + 4, 2)? * 60;
        i += 6;
        if b.get(i) == Some(&b':') {
            seconds += digits(i + 1, 2)?;
            i += 3;
        }
        if b.get(i) == Some(&b'.') {
            i += 1;
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
    }
    match b.get(i) {
        None => Some(seconds),
        Some(b'Z') if i + 1 == b.len() => Some(seconds),
        Some(sign @ (b'+' | b'-')) => {
            let hours = digits(i + 1, 2)?;
            let minutes = match b.get(i + 3) {
                Some(b':') => digits(i + 4, 2)?,
                _ => digits(i + 3, 2).unwrap_or(0),
            };
            let offset = hours * 3600 + minutes * 60;
            Some(if *sign == b'+' {
                seconds - offset
            } else {
                seconds + offset
            })
        }
        _ => None,
    }
}
