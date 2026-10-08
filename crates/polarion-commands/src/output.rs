//! Results that are not library types.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

/// Whether an operation changed anything: `{"changed": true}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Changed {
    pub changed: bool,
}

impl From<bool> for Changed {
    fn from(changed: bool) -> Self {
        Self { changed }
    }
}

/// Nothing to report (`null`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Done;

/// Binary content: `{"size": .., "base64": ..}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download(pub Vec<u8>);

impl Serialize for Download {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("size", &self.0.len())?;
        map.serialize_entry("base64", &BASE64.encode(&self.0))?;
        map.end()
    }
}
