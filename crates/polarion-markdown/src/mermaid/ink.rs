//! Legacy mermaid.ink image URLs, which carry the diagram source in base64.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD_PAD_INDIFFERENT, URL_SAFE_PAD_INDIFFERENT};

use crate::html::percent_decode;

/// The diagram source of a `https://mermaid.ink/img/…` or `…/svg/…` URL, if it can be decoded.
///
/// The payload (query and fragment ignored, percent-encoding and an optional `base64:` prefix
/// removed) is standard or URL-safe base64, padded or not, of UTF-8 text: either the source itself
/// or a JSON object whose string `code` is the source (the mermaid live editor's format).
/// Compressed `pako:` payloads, other hosts and anything that does not decode give `None`.
pub fn decode_mermaid_ink(url: &str) -> Option<String> {
    const ORIGIN: &str = "https://mermaid.ink/";
    let url = url.trim();
    let origin = url.get(..ORIGIN.len())?;
    if !origin.eq_ignore_ascii_case(ORIGIN) {
        return None;
    }
    let path = &url[ORIGIN.len()..];
    let payload = path
        .strip_prefix("img/")
        .or_else(|| path.strip_prefix("svg/"))?;
    let payload = payload.split(['?', '#']).next().unwrap_or_default();
    // base64's `+`, `/` and `=` may arrive percent-encoded.
    let payload = percent_decode(payload);
    if payload.starts_with("pako:") {
        return None;
    }
    let payload = payload.strip_prefix("base64:").unwrap_or(&payload);
    let bytes = STANDARD_PAD_INDIFFERENT
        .decode(payload)
        .or_else(|_| URL_SAFE_PAD_INDIFFERENT.decode(payload))
        .ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let source = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(serde_json::Value::Object(object)) => match object.get("code") {
            Some(serde_json::Value::String(code)) => code.clone(),
            _ => text,
        },
        _ => text,
    };
    (!source.trim().is_empty()).then_some(source)
}
