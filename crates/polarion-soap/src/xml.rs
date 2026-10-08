//! Minimal XML (de)serialization used by the generated types.
//!
//! Writing goes through [`XmlWriter`]; all namespace prefixes are declared once
//! on the SOAP envelope, so element writers only emit `prefix:name`.
//! Reading works on a [`roxmltree`] DOM and matches child elements by local
//! name, which keeps decoding independent of the prefixes the server picks.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use roxmltree::Node;

use crate::error::{Error, Result};

pub(crate) const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
pub(crate) const XSD: &str = "http://www.w3.org/2001/XMLSchema";

/// String-backed XML writer.
#[derive(Debug, Default)]
pub struct XmlWriter {
    buf: String,
}

impl XmlWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_string(self) -> String {
        self.buf
    }

    /// `<prefix:name` (call [`XmlWriter::open_end`] or [`XmlWriter::empty_end`] next).
    pub fn open(&mut self, prefix: &str, name: &str) {
        self.buf.push('<');
        self.qname(prefix, name);
    }

    /// ` name="value"` inside an open start tag.
    pub fn attr(&mut self, name: &str, value: &str) {
        self.buf.push(' ');
        self.buf.push_str(name);
        self.buf.push_str("=\"");
        escape_into(&mut self.buf, value, true);
        self.buf.push('"');
    }

    pub fn open_end(&mut self) {
        self.buf.push('>');
    }

    pub fn empty_end(&mut self) {
        self.buf.push_str("/>");
    }

    pub fn close(&mut self, prefix: &str, name: &str) {
        self.buf.push_str("</");
        self.qname(prefix, name);
        self.buf.push('>');
    }

    pub fn text(&mut self, text: &str) {
        escape_into(&mut self.buf, text, false);
    }

    /// `<prefix:name [xsi:type]>text</prefix:name>`.
    pub fn leaf(&mut self, prefix: &str, name: &str, xsi_type: Option<&str>, text: &str) {
        self.open(prefix, name);
        if let Some(t) = xsi_type {
            self.attr("xsi:type", t);
        }
        self.open_end();
        self.text(text);
        self.close(prefix, name);
    }

    /// `<prefix:name xsi:nil="true"/>`.
    pub fn nil(&mut self, prefix: &str, name: &str) {
        self.open(prefix, name);
        self.attr("xsi:nil", "true");
        self.empty_end();
    }

    /// Append pre-serialized XML verbatim.
    pub fn raw(&mut self, xml: &str) {
        self.buf.push_str(xml);
    }

    fn qname(&mut self, prefix: &str, name: &str) {
        if !prefix.is_empty() {
            self.buf.push_str(prefix);
            self.buf.push(':');
        }
        self.buf.push_str(name);
    }
}

fn escape_into(buf: &mut String, text: &str, attribute: bool) {
    for c in text.chars() {
        match c {
            '&' => buf.push_str("&amp;"),
            '<' => buf.push_str("&lt;"),
            '>' => buf.push_str("&gt;"),
            '"' if attribute => buf.push_str("&quot;"),
            '\r' => buf.push_str("&#13;"),
            '\n' if attribute => buf.push_str("&#10;"),
            '\t' if attribute => buf.push_str("&#9;"),
            c => buf.push(c),
        }
    }
}

/// Serialize a value as an element.
pub trait ToXml {
    fn write_xml(&self, w: &mut XmlWriter, prefix: &str, name: &str) {
        self.write_xml_typed(w, prefix, name, None);
    }

    /// Like [`ToXml::write_xml`], with an `xsi:type` attribute (used for untyped values).
    fn write_xml_typed(&self, w: &mut XmlWriter, prefix: &str, name: &str, xsi_type: Option<&str>);
}

/// Deserialize a value from an element.
pub trait FromXml: Sized {
    fn read_xml(node: Node<'_, '_>) -> Result<Self>;
}

impl ToXml for str {
    fn write_xml_typed(&self, w: &mut XmlWriter, prefix: &str, name: &str, xsi_type: Option<&str>) {
        w.leaf(prefix, name, xsi_type, self);
    }
}

impl ToXml for String {
    fn write_xml_typed(&self, w: &mut XmlWriter, prefix: &str, name: &str, xsi_type: Option<&str>) {
        self.as_str().write_xml_typed(w, prefix, name, xsi_type);
    }
}

impl FromXml for String {
    fn read_xml(node: Node<'_, '_>) -> Result<Self> {
        Ok(text(node))
    }
}

impl ToXml for bool {
    fn write_xml_typed(&self, w: &mut XmlWriter, prefix: &str, name: &str, xsi_type: Option<&str>) {
        w.leaf(prefix, name, xsi_type, if *self { "true" } else { "false" });
    }
}

impl FromXml for bool {
    fn read_xml(node: Node<'_, '_>) -> Result<Self> {
        parse_bool(&text(node))
            .ok_or_else(|| Error::decode(node.tag_name().name(), "not a boolean"))
    }
}

macro_rules! number {
    ($($t:ty),*) => {$(
        impl ToXml for $t {
            fn write_xml_typed(&self, w: &mut XmlWriter, prefix: &str, name: &str, xsi_type: Option<&str>) {
                w.leaf(prefix, name, xsi_type, &self.to_string());
            }
        }

        impl FromXml for $t {
            fn read_xml(node: Node<'_, '_>) -> Result<Self> {
                let t = text(node);
                t.trim().parse().map_err(|_| {
                    Error::decode(node.tag_name().name(), format!("`{t}` is not a valid {}", stringify!($t)))
                })
            }
        }
    )*};
}
number!(i32, i64, f32, f64);

/// `xsd:base64Binary`.
impl ToXml for [u8] {
    fn write_xml_typed(&self, w: &mut XmlWriter, prefix: &str, name: &str, xsi_type: Option<&str>) {
        w.leaf(prefix, name, xsi_type, &BASE64.encode(self));
    }
}

impl FromXml for Vec<u8> {
    fn read_xml(node: Node<'_, '_>) -> Result<Self> {
        let t: String = text(node).split_whitespace().collect();
        BASE64
            .decode(t)
            .map_err(|e| Error::decode(node.tag_name().name(), format!("invalid base64: {e}")))
    }
}

impl<T: ToXml + ?Sized> ToXml for Box<T> {
    fn write_xml_typed(&self, w: &mut XmlWriter, prefix: &str, name: &str, xsi_type: Option<&str>) {
        (**self).write_xml_typed(w, prefix, name, xsi_type);
    }
}

impl<T: FromXml> FromXml for Box<T> {
    fn read_xml(node: Node<'_, '_>) -> Result<Self> {
        T::read_xml(node).map(Box::new)
    }
}

// ---- helpers used by generated code ----

/// Concatenated text content of an element.
pub fn text(node: Node<'_, '_>) -> String {
    node.children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect()
}

pub fn parse_bool(s: &str) -> Option<bool> {
    match s.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// `xsi:nil="true"`.
pub fn is_nil(node: Node<'_, '_>) -> bool {
    node.attribute((XSI, "nil")).and_then(parse_bool) == Some(true)
}

/// Decode an optional element: `xsi:nil` becomes `None`.
pub fn read_opt<T: FromXml>(node: Node<'_, '_>) -> Result<Option<T>> {
    if is_nil(node) {
        Ok(None)
    } else {
        T::read_xml(node).map(Some)
    }
}

/// Decode a required element: `xsi:nil` becomes `T::default()`.
pub fn read_req<T: FromXml + Default>(node: Node<'_, '_>) -> Result<T> {
    if is_nil(node) {
        Ok(T::default())
    } else {
        T::read_xml(node)
    }
}

/// Decode the items of an `ArrayOf…` wrapper element.
pub fn read_array<T: FromXml + Default>(node: Node<'_, '_>, item: &str) -> Result<Vec<T>> {
    node.children()
        .filter(|c| c.is_element() && c.tag_name().name() == item)
        .map(read_req)
        .collect()
}

/// Decode an optional `ArrayOf…` wrapper element.
pub fn read_opt_array<T: FromXml + Default>(
    node: Node<'_, '_>,
    item: &str,
) -> Result<Option<Vec<T>>> {
    if is_nil(node) {
        Ok(None)
    } else {
        read_array(node, item).map(Some)
    }
}

/// Write an `ArrayOf…` wrapper element with one child per item.
pub fn write_array<T: ToXml>(
    w: &mut XmlWriter,
    prefix: &str,
    name: &str,
    item_prefix: &str,
    item: &str,
    items: &[T],
) {
    w.open(prefix, name);
    w.open_end();
    for value in items {
        value.write_xml(w, item_prefix, item);
    }
    w.close(prefix, name);
}

/// Parse an attribute value.
pub fn attr<T: std::str::FromStr>(node: Node<'_, '_>, name: &str) -> Result<Option<T>> {
    match node.attribute(name) {
        None => Ok(None),
        Some(v) => v.trim().parse().map(Some).map_err(|_| {
            Error::decode(
                node.tag_name().name(),
                format!("invalid attribute {name}=`{v}`"),
            )
        }),
    }
}

/// Parse a boolean attribute value.
pub fn attr_bool(node: Node<'_, '_>, name: &str) -> Result<Option<bool>> {
    match node.attribute(name) {
        None => Ok(None),
        Some(v) => parse_bool(v).map(Some).ok_or_else(|| {
            Error::decode(
                node.tag_name().name(),
                format!("invalid attribute {name}=`{v}`"),
            )
        }),
    }
}

/// The `xsi:type` of an element as `(namespace URI, local name)`.
pub fn xsi_type<'a>(node: Node<'a, '_>) -> Option<(Option<&'a str>, &'a str)> {
    let value = node.attribute((XSI, "type"))?;
    match value.split_once(':') {
        Some((prefix, local)) => Some((node.lookup_namespace_uri(Some(prefix)), local)),
        None => Some((node.lookup_namespace_uri(None), value)),
    }
}

/// Whether the element has element children (i.e. is not a simple value).
pub fn has_element_children(node: Node<'_, '_>) -> bool {
    node.children().any(|c| c.is_element())
}

/// The original XML text of an element.
pub fn raw_xml(node: Node<'_, '_>) -> String {
    node.document().input_text()[node.range()].to_owned()
}
