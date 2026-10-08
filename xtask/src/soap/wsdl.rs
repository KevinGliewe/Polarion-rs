//! WSDL parsing into a small model.
//!
//! Supports exactly what Polarion's WSDLs use: SOAP 1.1 document/literal
//! wrapped operations, inline schemas, `sequence`, `complexContent/extension`,
//! attributes and simple-type restrictions. Anything else is an error, so a
//! new construct in a future Polarion version is noticed instead of ignored.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use roxmltree::{Document, Node};

pub const XSD: &str = "http://www.w3.org/2001/XMLSchema";
const WSDL: &str = "http://schemas.xmlsoap.org/wsdl/";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QName {
    pub ns: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeRef {
    /// A built-in XML Schema type (`string`, `int`, ...).
    Xsd(String),
    /// A named simple or complex type.
    Named(QName),
    /// No type given: any content, typed at runtime via `xsi:type`.
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElemDef {
    pub name: String,
    /// Namespace of the element itself (the declaring schema's target namespace).
    pub ns: String,
    pub ty: TypeRef,
    pub min: u32,
    pub unbounded: bool,
    pub nillable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrDef {
    pub name: String,
    pub ty: TypeRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplexDef {
    pub qname: QName,
    pub base: Option<QName>,
    pub elements: Vec<ElemDef>,
    pub attrs: Vec<AttrDef>,
}

#[derive(Debug, Clone)]
pub struct OpDef {
    pub name: String,
    pub doc: Option<String>,
    /// Namespace of the request/response wrapper elements.
    pub ns: String,
    pub params: Vec<ElemDef>,
    pub ret: Option<ElemDef>,
}

#[derive(Debug, Clone)]
pub struct Service {
    /// e.g. `TrackerWebService`.
    pub name: String,
    pub doc: Option<String>,
    pub ops: Vec<OpDef>,
}

#[derive(Debug, Default)]
pub struct Model {
    pub complex: BTreeMap<QName, ComplexDef>,
    /// Simple types → their XML Schema base type.
    pub simple: BTreeMap<QName, String>,
    pub services: Vec<Service>,
}

impl Model {
    pub fn load(dir: &Path) -> Result<Self> {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "wsdl"))
            .collect();
        files.sort();
        if files.is_empty() {
            bail!("no .wsdl files in {}", dir.display());
        }
        let mut model = Model::default();
        for path in files {
            let text = std::fs::read_to_string(&path)?;
            let doc =
                Document::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
            model
                .add(&doc)
                .with_context(|| format!("in {}", path.display()))?;
        }
        Ok(model)
    }

    fn add(&mut self, doc: &Document) -> Result<()> {
        let root = doc.root_element();
        let mut wrappers: BTreeMap<String, (String, Vec<ElemDef>)> = BTreeMap::new();

        for schema in children(root, WSDL, "types").flat_map(|t| children(t, XSD, "schema")) {
            let tns = schema
                .attribute("targetNamespace")
                .context("schema without targetNamespace")?;
            if schema.attribute("elementFormDefault") != Some("qualified") {
                bail!("schema {tns}: only elementFormDefault=\"qualified\" is supported");
            }
            for node in schema.children().filter(Node::is_element) {
                match node.tag_name().name() {
                    "complexType" => {
                        let def = parse_complex(node, tns)?;
                        match self.complex.get(&def.qname) {
                            Some(existing) if *existing != def => {
                                bail!("type {:?} is defined differently in two WSDLs", def.qname)
                            }
                            _ => {
                                self.complex.insert(def.qname.clone(), def);
                            }
                        }
                    }
                    "simpleType" => {
                        let name = node.attribute("name").context("anonymous simpleType")?;
                        let base = node
                            .children()
                            .find(|c| c.tag_name().name() == "restriction")
                            .and_then(|r| r.attribute("base"))
                            .context("simpleType without restriction")?;
                        let base = qname(node, base)?;
                        if base.ns != XSD {
                            bail!("simpleType {name} restricts non-XSD type {base:?}");
                        }
                        self.simple.insert(
                            QName {
                                ns: tns.to_owned(),
                                name: name.to_owned(),
                            },
                            base.name,
                        );
                    }
                    "element" => {
                        let name = node.attribute("name").context("element without name")?;
                        let ct = node
                            .children()
                            .find(|c| c.tag_name().name() == "complexType")
                            .with_context(|| {
                                format!("top-level element {name} without inline complexType")
                            })?;
                        let def = parse_complex(ct, tns)?;
                        wrappers.insert(name.to_owned(), (tns.to_owned(), def.elements));
                    }
                    "import" | "annotation" => {}
                    other => bail!("unsupported schema construct <{other}>"),
                }
            }
        }

        let messages: BTreeMap<&str, &str> = children(root, WSDL, "message")
            .filter_map(|m| {
                let part = children(m, WSDL, "part").next()?;
                Some((m.attribute("name")?, part.attribute("element")?))
            })
            .collect();
        let wrapper_of = |op: Node, dir: &str| -> Result<(String, Vec<ElemDef>)> {
            let msg = children(op, WSDL, dir)
                .next()
                .and_then(|n| n.attribute("message"))
                .context("operation without message")?;
            let msg = msg.rsplit(':').next().unwrap_or(msg);
            let element = messages
                .get(msg)
                .with_context(|| format!("unknown message {msg}"))?;
            let element = element.rsplit(':').next().unwrap_or(element);
            wrappers
                .get(element)
                .cloned()
                .with_context(|| format!("unknown wrapper element {element}"))
        };

        let service_node = children(root, WSDL, "service")
            .next()
            .context("no <service>")?;
        // The port name is the endpoint path (`…/ws/services/TrackerWebService`);
        // the service element itself is called `TrackerWebServiceService`.
        let port = children(service_node, WSDL, "port")
            .next()
            .context("service without port")?;
        let mut service = Service {
            name: port
                .attribute("name")
                .context("port without name")?
                .to_owned(),
            doc: documentation(service_node),
            ops: Vec::new(),
        };
        for port_type in children(root, WSDL, "portType") {
            for op in children(port_type, WSDL, "operation") {
                let name = op
                    .attribute("name")
                    .context("operation without name")?
                    .to_owned();
                let (ns, params) = wrapper_of(op, "input")?;
                let (_, mut outputs) = wrapper_of(op, "output")?;
                if outputs.len() > 1
                    || outputs
                        .first()
                        .is_some_and(|o| o.name != format!("{name}Return"))
                {
                    bail!("{name}: unexpected response shape {outputs:?}");
                }
                service.ops.push(OpDef {
                    doc: documentation(op),
                    ns,
                    params,
                    ret: outputs.pop(),
                    name,
                });
            }
        }
        service.ops.sort_by(|a, b| a.name.cmp(&b.name));
        self.services.push(service);
        Ok(())
    }
}

fn children<'a, 'i>(
    node: Node<'a, 'i>,
    ns: &'a str,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(move |c| {
        c.is_element() && c.tag_name().namespace() == Some(ns) && c.tag_name().name() == name
    })
}

fn documentation(node: Node) -> Option<String> {
    children(node, WSDL, "documentation")
        .next()
        .map(|d| d.text().unwrap_or("").trim().to_owned())
        .filter(|d| !d.is_empty())
}

/// Resolve a `prefix:local` reference in the scope of `node`.
fn qname(node: Node, value: &str) -> Result<QName> {
    let (prefix, local) = match value.split_once(':') {
        Some((p, l)) => (Some(p), l),
        None => (None, value),
    };
    let ns = node
        .lookup_namespace_uri(prefix)
        .with_context(|| format!("unbound prefix in `{value}`"))?;
    Ok(QName {
        ns: ns.to_owned(),
        name: local.to_owned(),
    })
}

fn type_ref(node: Node, value: Option<&str>) -> Result<TypeRef> {
    let Some(value) = value else {
        return Ok(TypeRef::Any);
    };
    let q = qname(node, value)?;
    Ok(if q.ns == XSD {
        if q.name == "anyType" {
            TypeRef::Any
        } else {
            TypeRef::Xsd(q.name)
        }
    } else {
        TypeRef::Named(q)
    })
}

fn parse_complex(ct: Node, tns: &str) -> Result<ComplexDef> {
    let mut def = ComplexDef {
        qname: QName {
            ns: tns.to_owned(),
            name: ct.attribute("name").unwrap_or_default().to_owned(),
        },
        base: None,
        elements: Vec::new(),
        attrs: Vec::new(),
    };
    parse_content(ct, tns, &mut def)?;
    Ok(def)
}

fn parse_content(node: Node, tns: &str, def: &mut ComplexDef) -> Result<()> {
    for child in node.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "sequence" => {
                for el in child.children().filter(Node::is_element) {
                    if el.tag_name().name() != "element" {
                        bail!(
                            "{}: unsupported <{}> in sequence",
                            def.qname.name,
                            el.tag_name().name()
                        );
                    }
                    if el.attribute("ref").is_some()
                        || el
                            .children()
                            .any(|c| c.is_element() && c.tag_name().name() != "annotation")
                    {
                        bail!(
                            "{}: element refs and inline element types are not supported",
                            def.qname.name
                        );
                    }
                    let max = el.attribute("maxOccurs").unwrap_or("1");
                    def.elements.push(ElemDef {
                        name: el
                            .attribute("name")
                            .context("element without name")?
                            .to_owned(),
                        ns: tns.to_owned(),
                        ty: type_ref(el, el.attribute("type"))?,
                        min: el.attribute("minOccurs").unwrap_or("1").parse()?,
                        unbounded: max == "unbounded" || max.parse::<u32>().is_ok_and(|m| m > 1),
                        nillable: el.attribute("nillable") == Some("true"),
                    });
                }
            }
            "complexContent" => {
                let ext = child
                    .children()
                    .find(|c| c.is_element() && c.tag_name().name() == "extension")
                    .context("complexContent without extension")?;
                def.base = Some(qname(
                    ext,
                    ext.attribute("base").context("extension without base")?,
                )?);
                parse_content(ext, tns, def)?;
            }
            "attribute" => def.attrs.push(AttrDef {
                name: child
                    .attribute("name")
                    .context("attribute without name")?
                    .to_owned(),
                ty: type_ref(child, child.attribute("type"))?,
            }),
            "annotation" => {}
            other => bail!("{}: unsupported <{other}>", def.qname.name),
        }
    }
    Ok(())
}
