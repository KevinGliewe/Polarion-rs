//! SOAP client generation from `spec/soap/*.wsdl`.

mod emit;
mod wsdl;

use std::path::{Path, PathBuf};

use anyhow::Result;

/// Generate the SOAP client sources: `(path, unformatted source)` pairs.
pub fn generate(root: &Path) -> Result<(Vec<(PathBuf, String)>, String)> {
    let model = wsdl::Model::load(&root.join("spec/soap"))?;
    let generator = emit::Gen::new(&model)?;
    let out = root.join("crates/polarion-soap");
    let files = vec![
        (out.join("src/generated/types.rs"), generator.types()?),
        (out.join("src/generated/services.rs"), generator.services()?),
        (out.join("tests/spec_examples.rs"), generator.tests()),
    ];
    let ops: usize = model.services.iter().map(|s| s.ops.len()).sum();
    let summary = format!(
        "SOAP: {} services, {ops} operations, {} types",
        model.services.len(),
        model.complex.len()
    );
    Ok((files, summary))
}
