//! WSDL-driven round-trip checks, independent of the generator: read the
//! schemas from `spec/soap/*.wsdl`, synthesize an example element for a type
//! (every optional element present), decode it into the Rust type, encode it
//! again and compare the two documents structurally (namespace URIs, local
//! names, attributes, text; prefixes ignored).

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::OnceLock;

use polarion_soap::NAMESPACES;
use polarion_soap::types::AnyValue;
use polarion_soap::xml::{FromXml, ToXml, XmlWriter};
use roxmltree::{Document, Node};

const XSD: &str = "http://www.w3.org/2001/XMLSchema";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

type QName = (String, String);

#[derive(Debug, Clone)]
enum TRef {
    Xsd(String),
    Named(QName),
    Any,
}

#[derive(Debug, Clone)]
struct Elem {
    name: String,
    ns: String,
    ty: TRef,
}

#[derive(Debug, Clone, Default)]
struct Complex {
    base: Option<QName>,
    elements: Vec<Elem>,
    attrs: Vec<(String, TRef)>,
}

#[derive(Debug, Default)]
struct Index {
    complex: BTreeMap<QName, Complex>,
    simple: BTreeMap<QName, String>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../spec/soap");
        let mut index = Index::default();
        for entry in std::fs::read_dir(dir).expect("spec/soap") {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "wsdl") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let doc = Document::parse(&text).unwrap();
            for schema in doc
                .descendants()
                .filter(|n| n.has_tag_name((XSD, "schema")))
            {
                let tns = schema.attribute("targetNamespace").unwrap();
                for node in schema.children().filter(Node::is_element) {
                    let Some(name) = node.attribute("name") else {
                        continue;
                    };
                    let key = (tns.to_owned(), name.to_owned());
                    match node.tag_name().name() {
                        "complexType" => {
                            index.complex.insert(key, parse_complex(node, tns));
                        }
                        "simpleType" => {
                            let base = node
                                .descendants()
                                .find(|n| n.has_tag_name((XSD, "restriction")))
                                .and_then(|r| r.attribute("base"))
                                .unwrap();
                            index.simple.insert(key, resolve(node, base).1);
                        }
                        _ => {}
                    }
                }
            }
        }
        index
    })
}

fn resolve(node: Node, value: &str) -> QName {
    let (prefix, local) = value
        .split_once(':')
        .map_or((None, value), |(p, l)| (Some(p), l));
    (
        node.lookup_namespace_uri(prefix)
            .unwrap_or_default()
            .to_owned(),
        local.to_owned(),
    )
}

fn tref(node: Node, value: Option<&str>) -> TRef {
    match value {
        None => TRef::Any,
        Some(v) => {
            let q = resolve(node, v);
            if q.0 == XSD {
                if q.1 == "anyType" {
                    TRef::Any
                } else {
                    TRef::Xsd(q.1)
                }
            } else {
                TRef::Named(q)
            }
        }
    }
}

fn parse_complex(ct: Node, tns: &str) -> Complex {
    let mut c = Complex::default();
    if let Some(ext) = ct
        .descendants()
        .find(|n| n.has_tag_name((XSD, "extension")))
    {
        c.base = Some(resolve(ext, ext.attribute("base").unwrap()));
    }
    for el in ct
        .descendants()
        .filter(|n| n.has_tag_name((XSD, "element")))
    {
        c.elements.push(Elem {
            name: el.attribute("name").unwrap().to_owned(),
            ns: tns.to_owned(),
            ty: tref(el, el.attribute("type")),
        });
    }
    for a in ct
        .descendants()
        .filter(|n| n.has_tag_name((XSD, "attribute")))
    {
        c.attrs.push((
            a.attribute("name").unwrap().to_owned(),
            tref(a, a.attribute("type")),
        ));
    }
    c
}

fn sample(xsd: &str) -> &'static str {
    match xsd {
        "boolean" => "true",
        "int" | "short" | "long" | "integer" => "7",
        "float" | "double" => "1.5",
        "decimal" => "1.25",
        "date" => "2024-01-31",
        "dateTime" => "2024-01-31T12:00:00.000Z",
        "time" => "12:00:00",
        "base64Binary" => "aGVsbG8=",
        "duration" => "P1D",
        _ => "text",
    }
}

struct Synth {
    prefixes: BTreeMap<String, String>,
    out: String,
}

impl Synth {
    fn new() -> Self {
        Self {
            prefixes: BTreeMap::new(),
            out: String::new(),
        }
    }

    fn prefix(&mut self, ns: &str) -> String {
        let next = format!("n{}", self.prefixes.len());
        self.prefixes.entry(ns.to_owned()).or_insert(next).clone()
    }

    fn scalar(&self, ty: &TRef) -> Option<String> {
        match ty {
            TRef::Xsd(x) => Some(sample(x).to_owned()),
            TRef::Named(q) => index().simple.get(q).map(|b| sample(b).to_owned()),
            TRef::Any => None,
        }
    }

    fn elements_of(q: &QName) -> (Vec<Elem>, Vec<(String, TRef)>) {
        let c = &index().complex[q];
        let (mut els, mut attrs) = match &c.base {
            Some(b) => Self::elements_of(b),
            None => (Vec::new(), Vec::new()),
        };
        els.extend(c.elements.iter().cloned());
        attrs.extend(c.attrs.iter().cloned());
        (els, attrs)
    }

    /// Write `<ns:name [xsi:type]>content</ns:name>` for a value of complex type `q`.
    fn complex(&mut self, ns: &str, name: &str, q: &QName, xsi_type: bool, stack: &mut Vec<QName>) {
        let p = self.prefix(ns);
        let _ = write!(self.out, "<{p}:{name}");
        if xsi_type {
            let tp = self.prefix(&q.0);
            let _ = write!(self.out, " xsi:type=\"{tp}:{}\"", q.1);
        }
        let (els, attrs) = Self::elements_of(q);
        for (a, ty) in &attrs {
            let _ = write!(self.out, " {a}=\"{}\"", self.scalar(ty).unwrap());
        }
        self.out.push('>');
        stack.push(q.clone());
        for el in &els {
            self.element(el, stack);
        }
        stack.pop();
        let _ = write!(self.out, "</{p}:{name}>");
    }

    fn element(&mut self, el: &Elem, stack: &mut Vec<QName>) {
        match &el.ty {
            TRef::Any => {
                let p = self.prefix(&el.ns);
                let _ = write!(
                    self.out,
                    "<{p}:{} xsi:type=\"xsd:string\">text</{p}:{}>",
                    el.name, el.name
                );
            }
            TRef::Named(q) if index().complex.contains_key(q) => {
                // Skip recursive elements so the example stays finite.
                if !stack.contains(q) {
                    self.complex(&el.ns, &el.name, q, false, stack);
                }
            }
            ty => {
                let p = self.prefix(&el.ns);
                let v = self
                    .scalar(ty)
                    .unwrap_or_else(|| panic!("no sample for {ty:?}"));
                let _ = write!(self.out, "<{p}:{}>{v}</{p}:{}>", el.name, el.name);
            }
        }
    }

    fn document(self) -> String {
        let mut decls = format!(" xmlns:xsi=\"{XSI}\" xmlns:xsd=\"{XSD}\"");
        for (ns, p) in &self.prefixes {
            let _ = write!(decls, " xmlns:{p}=\"{ns}\"");
        }
        format!("<wrap{decls}>{}</wrap>", self.out)
    }
}

/// Encode `value` as element `name` in namespace `ns` inside a wrapper declaring all prefixes.
fn encode<T: ToXml>(value: &T, ns: &str, name: &str) -> String {
    let prefix = NAMESPACES
        .iter()
        .find(|(_, uri)| *uri == ns)
        .map(|(p, _)| *p)
        .unwrap_or_else(|| panic!("no prefix for {ns}"));
    let mut w = XmlWriter::new();
    let mut open = format!("<wrap xmlns:xsi=\"{XSI}\" xmlns:xsd=\"{XSD}\"");
    for (p, uri) in NAMESPACES {
        let _ = write!(open, " xmlns:{p}=\"{uri}\"");
    }
    open.push('>');
    w.raw(&open);
    value.write_xml(&mut w, prefix, name);
    w.raw("</wrap>");
    w.into_string()
}

/// Structural form of an element: (ns, name, sorted attributes, text, children).
#[derive(Debug, PartialEq)]
struct Canon {
    ns: String,
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Canon>,
}

fn canon(node: Node) -> Canon {
    let mut attrs: Vec<(String, String)> = node
        .attributes()
        .map(|a| {
            let key = format!("{{{}}}{}", a.namespace().unwrap_or(""), a.name());
            let value = if a.namespace() == Some(XSI) && a.name() == "type" {
                let (ns, local) = resolve(node, a.value());
                format!("{{{ns}}}{local}")
            } else {
                a.value().to_owned()
            };
            (key, value)
        })
        .collect();
    attrs.sort();
    let children: Vec<Canon> = node
        .children()
        .filter(Node::is_element)
        .map(canon)
        .collect();
    let text = if children.is_empty() {
        node.children()
            .filter(Node::is_text)
            .filter_map(|t| t.text())
            .collect::<String>()
            .trim()
            .to_owned()
    } else {
        String::new()
    };
    Canon {
        ns: node.tag_name().namespace().unwrap_or("").to_owned(),
        name: node.tag_name().name().to_owned(),
        attrs,
        text,
        children,
    }
}

fn first_child<'a, 'i>(doc: &'a Document<'i>) -> Node<'a, 'i> {
    doc.root_element()
        .first_element_child()
        .expect("wrapped element")
}

/// Synthesize an example of complex type `{ns}name`, decode it as `T`, re-encode and compare.
pub fn roundtrip<T: FromXml + ToXml>(ns: &str, name: &str) {
    let q: QName = (ns.to_owned(), name.to_owned());
    let mut synth = Synth::new();
    synth.complex(ns, name, &q, false, &mut Vec::new());
    let input = synth.document();
    let in_doc = Document::parse(&input).expect("synthesized XML");
    let value = T::read_xml(first_child(&in_doc))
        .unwrap_or_else(|e| panic!("{name}: decode failed: {e}\n{input}"));
    let output = encode(&value, ns, name);
    let out_doc = Document::parse(&output)
        .unwrap_or_else(|e| panic!("{name}: re-encoded XML invalid: {e}\n{output}"));
    assert_eq!(
        canon(first_child(&out_doc)),
        canon(first_child(&in_doc)),
        "{name}: round-trip changed the document\ninput:  {input}\noutput: {output}"
    );
}

/// Same, as an `xsi:type`-tagged untyped value; `check` asserts the decoded variant.
pub fn roundtrip_any(ns: &str, name: &str, check: impl Fn(&AnyValue) -> bool) {
    let q: QName = (ns.to_owned(), name.to_owned());
    let mut synth = Synth::new();
    synth.complex(ns, "value", &q, true, &mut Vec::new());
    let input = synth.document();
    let in_doc = Document::parse(&input).expect("synthesized XML");
    let value = AnyValue::read_xml(first_child(&in_doc))
        .unwrap_or_else(|e| panic!("{name}: decode failed: {e}"));
    assert!(check(&value), "{name}: decoded as wrong variant: {value:?}");
    let output = encode(&value, ns, "value");
    let out_doc = Document::parse(&output).unwrap();
    assert_eq!(
        canon(first_child(&out_doc)),
        canon(first_child(&in_doc)),
        "{name}: AnyValue round-trip changed the document\ninput:  {input}\noutput: {output}"
    );
}
