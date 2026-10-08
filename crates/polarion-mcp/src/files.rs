//! Binary content over MCP: byte results become image or text blocks (never base64 for the
//! agent to read), and the optional exchange folder (`POLARION_MCP_FILES`) lets byte-returning
//! commands save files (`save_as`) and upload commands read them (`<param>-file`).

use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use polarion_commands::{CommandError, Entry, View};
use rmcp::model::ContentBlock;
use serde_json::{Map, Value, json};

/// Largest image returned as an image block.
const MAX_IMAGE: usize = 4 * 1024 * 1024;

/// Largest text returned as a text block.
const MAX_TEXT: usize = 100 * 1024;

/// The argument that saves a byte result in the exchange folder.
const SAVE_AS: &str = "save_as";

/// The folder files are exchanged through, by plain file name.
#[derive(Debug, Clone)]
pub struct Exchange {
    dir: PathBuf,
}

impl Exchange {
    /// `POLARION_MCP_FILES`, which must be an existing directory; `None` when unset.
    pub fn from_env() -> Result<Option<Self>, String> {
        let Some(dir) = std::env::var_os("POLARION_MCP_FILES").filter(|d| !d.is_empty()) else {
            return Ok(None);
        };
        let dir = Path::new(&dir);
        let dir = dir
            .canonicalize()
            .ok()
            .filter(|d| d.is_dir())
            .ok_or_else(|| {
                format!(
                    "POLARION_MCP_FILES: {} is not an existing directory",
                    dir.display()
                )
            })?;
        Ok(Some(Self { dir }))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `name` inside the folder: a plain file name, and not a symbolic link.
    fn path(&self, name: &str) -> Result<PathBuf, CommandError> {
        let mut components = Path::new(name).components();
        let plain = matches!(
            (components.next(), components.next()),
            (Some(Component::Normal(_)), None)
        ) && !name.contains(['/', '\\']);
        if !plain {
            return Err(CommandError::invalid(format!(
                "`{name}` is not a plain file name in the exchange folder"
            )));
        }
        let path = self.dir.join(name);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(CommandError::invalid(format!(
                "`{name}` is a symbolic link; only plain files are exchanged"
            )));
        }
        Ok(path)
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, CommandError> {
        let path = self.path(name)?;
        std::fs::read(&path)
            .map_err(|e| CommandError::invalid(format!("cannot read {}: {e}", path.display())))
    }

    fn write(&self, name: &str, data: &[u8]) -> Result<PathBuf, CommandError> {
        let path = self.path(name)?;
        std::fs::write(&path, data)
            .map_err(|e| CommandError::invalid(format!("cannot write {}: {e}", path.display())))?;
        Ok(path)
    }
}

/// The arguments that take binary content (base64 in the schema).
fn byte_arguments(schema: &Value) -> Vec<String> {
    schema["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, p)| p.get("contentEncoding").and_then(Value::as_str) == Some("base64"))
        .map(|(name, _)| name.clone())
        .collect()
}

/// The argument naming the exchange file for byte parameter `param`.
fn file_argument(param: &str) -> String {
    format!("{param}-file")
}

/// The command's input schema, plus the exchange arguments when the folder is set.
pub fn schema(command: &Entry, exchange: Option<&Exchange>) -> Value {
    let mut schema = command.input_schema();
    if exchange.is_none() {
        return schema;
    }
    let bytes = byte_arguments(&schema);
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        if matches!(command.view, View::Bytes) {
            properties.insert(
                SAVE_AS.into(),
                json!({"type": "string", "description": "Save the content as this file in the exchange folder instead of returning it"}),
            );
        }
        for name in &bytes {
            properties.insert(
                file_argument(name),
                json!({"type": "string", "description": format!("Read `{name}` from this file in the exchange folder (instead of base64)")}),
            );
        }
    }
    if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|r| !r.as_str().is_some_and(|r| bytes.iter().any(|b| b == r)));
    }
    schema
}

/// Resolve the exchange arguments: read `<param>-file` into `<param>` (base64), and take out
/// `save_as`, which is returned.
pub fn prepare(
    command: &Entry,
    arguments: Value,
    exchange: Option<&Exchange>,
) -> Result<(Value, Option<String>), CommandError> {
    let mut arguments = match arguments {
        Value::Object(map) => map,
        Value::Null => Map::new(),
        other => return Ok((other, None)),
    };
    let needs_folder = |argument: &str| {
        CommandError::invalid(format!(
            "`{argument}` needs the exchange folder (start the server with POLARION_MCP_FILES)"
        ))
    };
    let save_as = match arguments.remove(SAVE_AS) {
        None => None,
        Some(_) if !matches!(command.view, View::Bytes) => {
            return Err(CommandError::invalid(format!(
                "`{SAVE_AS}` only applies to commands that return file content"
            )));
        }
        Some(value) => {
            let exchange = exchange.ok_or_else(|| needs_folder(SAVE_AS))?;
            let Value::String(name) = value else {
                return Err(CommandError::invalid(format!(
                    "`{SAVE_AS}` must be a string"
                )));
            };
            // Refuse a bad name before anything is downloaded.
            exchange.path(&name)?;
            Some(name)
        }
    };
    for param in byte_arguments(&command.input_schema()) {
        let key = file_argument(&param);
        let Some(name) = arguments.remove(&key) else {
            continue;
        };
        let exchange = exchange.ok_or_else(|| needs_folder(&key))?;
        let Value::String(name) = name else {
            return Err(CommandError::invalid(format!("`{key}` must be a string")));
        };
        if arguments.contains_key(&param) {
            return Err(CommandError::invalid(format!(
                "give either `{param}` or `{key}`, not both"
            )));
        }
        let data = exchange.read(&name)?;
        arguments.insert(param, Value::from(BASE64.encode(data)));
    }
    Ok((Value::Object(arguments), save_as))
}

/// A byte result (`{"size", "base64"}`) as content for the agent.
pub fn present(
    value: &Value,
    save_as: Option<&str>,
    exchange: Option<&Exchange>,
) -> Result<Vec<ContentBlock>, CommandError> {
    let data = value
        .get("base64")
        .and_then(Value::as_str)
        .and_then(|b| BASE64.decode(b).ok())
        .ok_or_else(|| CommandError::invalid("the command returned no file content"))?;
    if let (Some(name), Some(exchange)) = (save_as, exchange) {
        let path = exchange.write(name, &data)?;
        let saved = json!({"savedTo": path.display().to_string(), "size": data.len()});
        return Ok(vec![ContentBlock::text(saved.to_string())]);
    }
    let kind = media_type(&data);
    if kind.starts_with("image/") && data.len() <= MAX_IMAGE {
        return Ok(vec![ContentBlock::image(BASE64.encode(&data), kind)]);
    }
    if let Ok(text) = std::str::from_utf8(&data)
        && kind == "application/octet-stream"
        && !text.contains('\0')
        && data.len() <= MAX_TEXT
    {
        return Ok(vec![ContentBlock::text(text)]);
    }
    let hint = if exchange.is_some() {
        format!("Pass `{SAVE_AS}` to save it in the exchange folder.")
    } else {
        "Start the server with POLARION_MCP_FILES (an exchange folder) to save such files."
            .to_owned()
    };
    let summary =
        json!({"size": data.len(), "type": kind, "note": format!("Content not shown. {hint}")});
    Ok(vec![ContentBlock::text(summary.to_string())])
}

/// The media type, from the first bytes; `application/octet-stream` when unknown.
fn media_type(data: &[u8]) -> &'static str {
    match data {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => "image/webp",
        [b'%', b'P', b'D', b'F', b'-', ..] => "application/pdf",
        [b'P', b'K', 3, 4, ..] => "application/zip",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_types_are_sniffed() {
        assert_eq!(media_type(b"\x89PNG\r\n\x1a\n...."), "image/png");
        assert_eq!(media_type(b"RIFF\0\0\0\0WEBPVP8 "), "image/webp");
        assert_eq!(media_type(b"%PDF-1.7"), "application/pdf");
        assert_eq!(media_type(b"PK\x03\x04"), "application/zip");
        assert_eq!(media_type(b"hello"), "application/octet-stream");
    }

    #[test]
    fn only_plain_file_names_are_exchanged() {
        let exchange = Exchange {
            dir: std::env::temp_dir(),
        };
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "/etc/passwd"] {
            assert!(exchange.path(bad).is_err(), "{bad:?}");
        }
        assert!(exchange.path("spec.pdf").is_ok());
    }
}
