//! Repository automation. Run `cargo xtask codegen` to regenerate the REST
//! client from `spec/polarion-openapi.json`, `--check` to verify it is current.

mod docs;
mod emit;
mod names;
mod ops;
mod soap;
mod spec;
mod types;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result, bail};

use crate::spec::{Spec, Verb, envelope};
use crate::types::{Registry, Ty};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("codegen") => codegen(args[1..].iter().any(|a| a == "--check")),
        _ => {
            eprintln!("usage: cargo xtask codegen [--check]");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root")
        .to_owned()
}

fn codegen(check: bool) -> Result<()> {
    let root = workspace_root();
    let spec = Spec::load(&root.join("spec/polarion-openapi.json"))?;
    let mut reg = Registry::new(&spec);

    // Resource attribute structs, merged across all envelope variants, plus
    // a round-trip test case per envelope schema.
    let schema_names: Vec<String> = spec
        .root
        .pointer("/components/schemas")
        .and_then(|s| s.as_object())
        .context("spec has no components.schemas")?
        .keys()
        .cloned()
        .collect();
    let mut tests = Vec::new();
    for name in &schema_names {
        let Some(env) = envelope(&spec, name) else {
            continue;
        };
        let schema = spec.schema(name).expect("listed above");
        match (&env.resource_type, env.data_item(&spec, schema)) {
            (Some(rt), Some(item)) => {
                let attrs = reg.register_resource(rt, item)?;
                let ty = match (env.request, env.list, env.verb) {
                    (_, _, Verb::Delete) => "Envelope<Vec<ResourceRef>>".to_owned(),
                    (false, true, _) => format!("ListEnvelope<{attrs}>"),
                    (false, false, _) => format!("SingleEnvelope<{attrs}>"),
                    (true, true, _) => format!("Envelope<Vec<Resource<{attrs}>>>"),
                    (true, false, _) => format!("Envelope<Resource<{attrs}>>"),
                };
                tests.push((name.clone(), ty));
            }
            _ if env.verb == Verb::Delete => {
                tests.push((name.clone(), "Envelope<Vec<ResourceRef>>".to_owned()))
            }
            _ => bail!("envelope schema `{name}` has no single resource type"),
        }
    }

    let ops = ops::collect(&spec, &mut reg)?;

    for (name, ty) in reg.plain_schemas() {
        if let Ty::Named(t) = ty {
            tests.push((name.clone(), t.clone()));
        }
    }
    for (name, ty) in [
        (
            "RelationshipDataSingleRequest",
            "Envelope<RelationshipData>",
        ),
        ("RelationshipDataListRequest", "Envelope<RelationshipData>"),
        (
            "RelationshipDataSingleResponse",
            "Envelope<Option<RelationshipData>>",
        ),
        (
            "RelationshipDataListResponse",
            "Envelope<Option<RelationshipData>>",
        ),
    ] {
        if spec.schema(name).is_some() {
            tests.push((name.to_owned(), ty.to_owned()));
        }
    }
    tests.sort();

    for warning in &reg.warnings {
        eprintln!("warning: {warning}");
    }

    let out_dir = root.join("crates/polarion-rest");
    let mut files = vec![
        (out_dir.join("src/generated/models.rs"), emit::models(&reg)?),
        (
            out_dir.join("src/generated/operations.rs"),
            emit::operations(&ops),
        ),
        (
            out_dir.join("tests/spec_examples.rs"),
            emit::spec_tests(&tests),
        ),
    ];
    let (soap_files, soap_summary) = soap::generate(&root)?;
    files.extend(soap_files);

    let mut stale = Vec::new();
    for (path, source) in &files {
        let formatted =
            rustfmt(source).with_context(|| format!("formatting {}", path.display()))?;
        let current = std::fs::read_to_string(path).unwrap_or_default();
        if current == formatted {
            continue;
        }
        if check {
            stale.push(
                path.strip_prefix(&root)
                    .unwrap_or(path)
                    .display()
                    .to_string(),
            );
        } else {
            std::fs::write(path, formatted)
                .with_context(|| format!("writing {}", path.display()))?;
            println!(
                "wrote {}",
                path.strip_prefix(&root).unwrap_or(path).display()
            );
        }
    }
    if !stale.is_empty() {
        bail!(
            "generated code is out of date, run `cargo xtask codegen`: {}",
            stale.join(", ")
        );
    }
    println!(
        "REST: {} operations, {} types, {} spec round-trip tests",
        ops.len(),
        reg.structs.len(),
        tests.len(),
    );
    println!("{soap_summary}");
    if check {
        println!("generated code is up to date");
    }
    Ok(())
}

fn rustfmt(source: &str) -> Result<String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running rustfmt (install it with `rustup component add rustfmt`)")?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(source.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "rustfmt failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8(output.stdout)?)
}
