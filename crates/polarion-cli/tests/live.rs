//! The front-end tier of the live suite for the `polarion` binary, against a real server (see
//! docs/testing.md and `crates/polarion/tests/live.rs`).
//!
//! ```sh
//! cargo test -p polarion-cli --test live -- --ignored --nocapture
//! ```
//!
//! The environment of the write tier (`POLARION_URL`, `POLARION_TOKEN`, `POLARION_PROJECT`,
//! `POLARION_LIVE_WRITES`, `POLARION_LIVE_ITEMS`, `POLARION_LIVE_TEST_RUN`,
//! `POLARION_LIVE_DOCUMENT`, `POLARION_LIVE_USER`). After the gates of the write tier, the binary
//! runs with `POLARION_WRITE_TARGETS` set to A's URI and every other `POLARION_*` variable
//! removed: a query, `items set-description-markdown` on A (restored afterwards through the
//! library), and the canary `items remove-hyperlink`, which must be refused with `readOnly`.
//! Then the fixtures are compared with the baseline, and the audit runs.

#[macro_use]
#[path = "../../polarion/tests/live_support.rs"]
mod live_support;

use live_support::*;
use polarion::{Fields, WorkItemUpdate};
use serde_json::Value;

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Run {
    fn describe(&self) -> String {
        format!(
            "exit {:?}, stdout {}, stderr {}",
            self.code,
            cut(&self.stdout),
            cut(&self.stderr)
        )
    }
}

/// The binary with `args`, allowed to write to A only.
async fn polarion(g: &Gate, args: &[&str]) -> Result<Run, String> {
    let mut cmd = front_end(
        env!("CARGO_BIN_EXE_polarion"),
        &g.settings,
        &g.fixtures.a.uri,
    );
    cmd.args(args);
    let out = tokio::task::spawn_blocking(move || cmd.output())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("cannot run polarion: {e}"))?;
    Ok(Run {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: writes to fixture A on the server in POLARION_URL (see docs/testing.md)"]
async fn front_end_cli() {
    let _tier = one_tier("front end (CLI)");
    let mut report = Report::default();
    if let Some(g) = write_gate(&mut report).await {
        let result = steps(&mut report, &g).await;
        if g.after(&mut report, "CLI", result).await {
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
    report.finish("front end (CLI)");
}

async fn steps(report: &mut Report, g: &Gate) -> Result<(), String> {
    let f = &g.fixtures;
    let a_id = f.a.id.clone().ok_or("A's URI has no id")?;

    let lucene = format!("id:{a_id}");
    let run = polarion(
        g,
        &[
            "--json", "-p", &f.project, "items", "query", &lucene, "--limit", "1",
        ],
    )
    .await?;
    let listed: Value = serde_json::from_str(&run.stdout).unwrap_or(Value::Null);
    check!(
        report,
        run.code == Some(0)
            && listed
                .as_array()
                .is_some_and(|l| l.len() == 1 && l[0]["id"] == a_id.as_str()),
        "CLI: items query finds A",
        "{}",
        run.describe()
    );

    let before = g
        .reader
        .items()
        .get_fields(&f.a, &Fields::new(["description"]))
        .await
        .map_err(|e| format!("A's description: {e}"))?
        .description;
    let token = run_token();
    let markdown = format!("polarion-rs live test of the CLI ({token}).");
    let run = polarion(
        g,
        &[
            "--json",
            "items",
            "set-description-markdown",
            &f.a.uri,
            "--markdown",
            &markdown,
        ],
    )
    .await?;
    let out: Value = serde_json::from_str(&run.stdout).unwrap_or(Value::Null);
    check!(
        report,
        run.code == Some(0) && out["updated"] == true,
        "CLI: items set-description-markdown on A",
        "{}",
        run.describe()
    );
    let written = g
        .reader
        .items()
        .get_fields(&f.a, &Fields::new(["description"]))
        .await
        .map_err(|e| format!("A's description: {e}"))?
        .description;
    check!(
        report,
        written.as_ref().is_some_and(|d| d.content.contains(&token)),
        "CLI: A's description is the HTML of the Markdown",
        "description {:?}",
        written.as_ref().map(|d| cut(&d.content))
    );
    let items = g.writer.items();
    let restored = match &before {
        Some(text) => {
            items
                .update(
                    &f.a,
                    &WorkItemUpdate {
                        description: Some(text.clone()),
                        ..WorkItemUpdate::default()
                    },
                )
                .await
        }
        None => items.clear_fields(&f.a, &["description".to_owned()]).await,
    };
    restored.map_err(|e| format!("restoring A's description: {e}"))?;
    let after = g
        .reader
        .items()
        .get_fields(&f.a, &Fields::new(["description"]))
        .await
        .map_err(|e| format!("A's description: {e}"))?
        .description;
    check!(
        report,
        after == before,
        "CLI: A's description restored",
        "description {:?}",
        after.as_ref().map(|d| cut(&d.content))
    );

    let run = polarion(
        g,
        &[
            "--json",
            "items",
            "remove-hyperlink",
            &g.canary.uri,
            "--url",
            CANARY_URL,
        ],
    )
    .await?;
    let error: Value = serde_json::from_str(run.stderr.trim()).unwrap_or(Value::Null);
    check!(
        report,
        run.code == Some(1) && error["error"]["code"] == "readOnly",
        "CLI: items remove-hyperlink on the canary is refused (readOnly)",
        "{}",
        run.describe()
    );
    Ok(())
}
