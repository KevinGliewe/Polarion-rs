//! Helpers for command handlers.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Serialize;
use serde_json::{Value, json};

use crate::error::Result;

/// A serializable result as JSON.
pub(crate) fn out(value: impl Serialize) -> Result<Value> {
    Ok(serde_json::to_value(value).expect("results always serialize"))
}

/// Binary content as `{size, base64}`.
pub(crate) fn bytes(data: &[u8]) -> Result<Value> {
    Ok(json!({"size": data.len(), "base64": BASE64.encode(data)}))
}

/// A catalog entry: `command!("area", "name", write, output, "summary", [params], handler)`.
macro_rules! command {
    ($area:literal, $name:literal, $write:literal, $output:expr, $summary:literal, [$($p:expr),* $(,)?], $f:path $(,)?) => {
        $crate::catalog::Command {
            area: $area,
            name: $name,
            summary: $summary,
            params: &[$($p),*],
            write: $write,
            output: $output,
            run: |p, a| Box::pin($f(p, a)),
        }
    };
}
pub(crate) use command;

/// `{"changed": bool}` for operations that report whether anything changed.
pub(crate) fn changed(changed: bool) -> Result<Value> {
    Ok(json!({ "changed": changed }))
}
