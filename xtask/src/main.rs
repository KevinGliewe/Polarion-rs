//! Repository automation. Run `cargo xtask codegen` to regenerate the SOAP
//! client from `spec/soap/*.wsdl`, `--check` to verify it is current.

mod docs;
mod names;
mod soap;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result, bail};

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
    let (files, summary) = soap::generate(&root)?;

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
    println!("{summary}");
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
