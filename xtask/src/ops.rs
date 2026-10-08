//! Operation model: one entry per path + method.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::docs::doc_lines;
use crate::names::{resource_name, snake};
use crate::spec::{Spec, Verb, envelope, ref_name};
use crate::types::Registry;

#[derive(Debug)]
pub struct Op {
    pub fn_name: String,
    pub method: &'static str,
    pub tag: String,
    pub doc: Vec<String>,
    pub segments: Vec<Segment>,
    pub path_params: Vec<String>,
    /// Required query parameters, passed positionally: `(json name, ident)`.
    pub required_query: Vec<(String, String)>,
    /// Optional query parameters, set via `Query`. Empty → no `query` argument.
    pub optional_query: Vec<String>,
    pub body: Body,
    pub response: Response,
}

#[derive(Debug)]
pub enum Segment {
    Lit(String),
    Param(String),
}

/// A JSON value passed by the caller.
#[derive(Debug)]
pub enum Payload {
    /// `&[Resource<A>]`, sent as `{"data": [...]}`.
    Resources(String),
    /// `&Resource<A>`, sent as `{"data": {...}}`.
    Resource(String),
    /// `&[ResourceRef]`, sent as `{"data": [...]}`.
    Refs,
    /// `&RelationshipData`, sent as `{"data": ...}`.
    Relationship,
    /// `&T` for a generated struct, sent as is.
    Plain(String),
    /// `&Value`, sent as is.
    Value,
}

impl Payload {
    pub fn arg_type(&self) -> String {
        match self {
            Payload::Resources(a) => format!("&[Resource<{a}>]"),
            Payload::Resource(a) => format!("&Resource<{a}>"),
            Payload::Refs => "&[ResourceRef]".into(),
            Payload::Relationship => "&RelationshipData".into(),
            Payload::Plain(t) => format!("&{t}"),
            Payload::Value => "&Value".into(),
        }
    }

    /// Whether the value is wrapped in `{"data": ...}` on the wire.
    pub fn wrapped(&self) -> bool {
        matches!(
            self,
            Payload::Resources(_) | Payload::Resource(_) | Payload::Refs | Payload::Relationship
        )
    }

    fn default_ident(&self) -> &'static str {
        if self.wrapped() { "data" } else { "body" }
    }
}

#[derive(Debug)]
pub enum Body {
    None,
    Json { ident: String, payload: Payload },
    Bytes,
    Multipart(Vec<MultipartPart>),
}

#[derive(Debug)]
pub struct MultipartPart {
    pub name: String,
    pub ident: String,
    pub kind: PartKind,
    pub required: bool,
}

#[derive(Debug)]
pub enum PartKind {
    File,
    Files,
    Json(Payload),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Response {
    Unit,
    Bytes,
    Json(String),
}

const METHODS: &[(&str, &str)] = &[
    ("get", "GET"),
    ("post", "POST"),
    ("patch", "PATCH"),
    ("delete", "DELETE"),
    ("put", "PUT"),
];

pub fn collect(spec: &Spec, reg: &mut Registry) -> Result<Vec<Op>> {
    let mut ops = Vec::new();
    let paths = spec
        .root
        .get("paths")
        .and_then(Value::as_object)
        .context("spec has no paths")?;
    for (path, item) in paths {
        let shared_params = item
            .get("parameters")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (key, method) in METHODS {
            let Some(op) = item.get(*key) else { continue };
            let built = build(spec, reg, path, method, op, &shared_params)
                .with_context(|| format!("{method} {path}"))?;
            ops.push(built);
        }
    }
    let mut names = BTreeSet::new();
    for op in &ops {
        if !names.insert(op.fn_name.clone()) {
            bail!("duplicate operation name `{}`", op.fn_name);
        }
    }
    ops.sort_by(|a, b| (&a.tag, &a.fn_name).cmp(&(&b.tag, &b.fn_name)));
    Ok(ops)
}

fn build(
    spec: &Spec,
    reg: &mut Registry,
    path: &str,
    method: &'static str,
    op: &Value,
    shared_params: &[Value],
) -> Result<Op> {
    let op_id = op
        .get("operationId")
        .and_then(Value::as_str)
        .context("missing operationId")?;
    let fn_name = snake(op_id);

    // Parameters.
    let mut path_param_names = BTreeSet::new();
    let mut required_query = Vec::new();
    let mut optional_query = Vec::new();
    let params = shared_params.iter().chain(
        op.get("parameters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten(),
    );
    for param in params {
        let param = spec.resolve(param);
        let name = param
            .get("name")
            .and_then(Value::as_str)
            .context("parameter without name")?;
        let required = param
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        match param.get("in").and_then(Value::as_str) {
            Some("path") => {
                path_param_names.insert(name.to_owned());
            }
            Some("query") if required => required_query.push((name.to_owned(), snake(name))),
            Some("query") => {
                let style = param.get("style").and_then(Value::as_str);
                optional_query.push(if style == Some("deepObject") {
                    format!("{name}[<type>]")
                } else {
                    name.to_owned()
                });
            }
            other => bail!("unsupported parameter location {other:?} for `{name}`"),
        }
    }

    let mut segments = Vec::new();
    let mut path_params = Vec::new();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        match seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            Some(name) => {
                if !path_param_names.remove(name) {
                    bail!("path parameter `{name}` is not declared");
                }
                let ident = snake(name);
                path_params.push(ident.clone());
                segments.push(Segment::Param(ident));
            }
            None => segments.push(Segment::Lit(seg.to_owned())),
        }
    }
    if !path_param_names.is_empty() {
        bail!("declared path parameters not in path: {path_param_names:?}");
    }

    let body = request_body(spec, reg, op)?;
    let response = response(spec, reg, op)?;

    // Argument names must be unique.
    let mut idents: Vec<&str> = path_params.iter().map(String::as_str).collect();
    idents.extend(required_query.iter().map(|(_, i)| i.as_str()));
    if !optional_query.is_empty() {
        idents.push("query");
    }
    match &body {
        Body::Json { ident, .. } => idents.push(ident),
        Body::Bytes => idents.push("body"),
        Body::Multipart(parts) => idents.extend(parts.iter().map(|p| p.ident.as_str())),
        Body::None => {}
    }
    let unique: BTreeSet<&str> = idents.iter().copied().collect();
    if unique.len() != idents.len() {
        bail!("argument name collision in {idents:?}");
    }

    // Documentation.
    let summary = op
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let description = op
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let mut doc = doc_lines(summary);
    if !description.is_empty() && description != summary {
        doc.push(String::new());
        doc.extend(doc_lines(description));
    }
    if op.get("deprecated").and_then(Value::as_bool) == Some(true) {
        doc.push(String::new());
        doc.push("**Deprecated** in the Polarion API.".into());
    }
    doc.push(String::new());
    doc.push(format!("`{method} {path}` (`{op_id}`)"));
    if !optional_query.is_empty() {
        doc.push(String::new());
        let list: Vec<String> = optional_query.iter().map(|q| format!("`{q}`")).collect();
        doc.push(format!("Query parameters: {}.", list.join(", ")));
    }

    let tag = op
        .get("tags")
        .and_then(Value::as_array)
        .and_then(|t| t.first())
        .and_then(Value::as_str)
        .unwrap_or("Other")
        .to_owned();

    Ok(Op {
        fn_name,
        method,

        tag,
        doc,
        segments,
        path_params,
        required_query,
        optional_query,
        body,
        response,
    })
}

fn request_body(spec: &Spec, reg: &mut Registry, op: &Value) -> Result<Body> {
    let Some(content) = op
        .pointer("/requestBody/content")
        .and_then(Value::as_object)
    else {
        return Ok(Body::None);
    };
    if content.len() != 1 {
        bail!("request body with {} content types", content.len());
    }
    let (content_type, media) = content.iter().next().expect("len checked");
    let schema = media.get("schema").cloned().unwrap_or(Value::Null);
    match content_type.as_str() {
        "application/json" => {
            let payload = payload(spec, reg, &schema)?;
            Ok(Body::Json {
                ident: payload.default_ident().to_owned(),
                payload,
            })
        }
        // The spec declares some of these as `object`; the wire format is raw bytes.
        "application/octet-stream" => Ok(Body::Bytes),
        "multipart/form-data" => {
            let schema = spec.resolve(&schema);
            let required: BTreeSet<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let props = schema
                .get("properties")
                .and_then(Value::as_object)
                .context("multipart body without properties")?;
            let mut parts = Vec::new();
            for (name, prop) in props {
                let resolved = spec.resolve(prop);
                let is_binary =
                    |s: &Value| s.get("format").and_then(Value::as_str) == Some("binary");
                let kind = if is_binary(resolved) {
                    PartKind::File
                } else if resolved.get("type").and_then(Value::as_str) == Some("array")
                    && resolved
                        .get("items")
                        .is_some_and(|i| is_binary(spec.resolve(i)))
                {
                    PartKind::Files
                } else {
                    PartKind::Json(payload(spec, reg, prop)?)
                };
                parts.push(MultipartPart {
                    name: name.clone(),
                    ident: snake(name),
                    kind,
                    required: required.contains(name.as_str()),
                });
            }
            // JSON metadata before file content: Polarion matches files to the
            // `resource` entries by order, and streaming parsers want metadata first.
            parts.sort_by_key(|p| !matches!(p.kind, PartKind::Json(_)));
            Ok(Body::Multipart(parts))
        }
        other => bail!("unsupported request content type `{other}`"),
    }
}

fn payload(spec: &Spec, reg: &mut Registry, schema: &Value) -> Result<Payload> {
    let Some(name) = ref_name(schema) else {
        return Ok(Payload::Value);
    };
    if let Some(env) = envelope(spec, name) {
        if env.verb == Verb::Delete {
            return Ok(Payload::Refs);
        }
        let attrs = attributes_type(env.resource_type.as_deref())?;
        return Ok(match (env.list, env.verb) {
            (true, Verb::Post | Verb::Patch) => Payload::Resources(attrs),
            (false, Verb::Post | Verb::Patch) => Payload::Resource(attrs),
            _ => bail!("unexpected request envelope `{name}`"),
        });
    }
    if name.starts_with("Relationships") && name.ends_with("Body") {
        return Ok(Payload::Relationship);
    }
    Ok(Payload::Plain(reg.plain(name).rust()))
}

fn response(spec: &Spec, reg: &mut Registry, op: &Value) -> Result<Response> {
    let responses = op
        .get("responses")
        .and_then(Value::as_object)
        .context("missing responses")?;
    let mut kinds = BTreeSet::new();
    for (code, resp) in responses {
        if !code.starts_with('2') {
            continue;
        }
        let resp = spec.resolve(resp);
        let content = resp.get("content").and_then(Value::as_object);
        let Some(content) = content.filter(|c| !c.is_empty()) else {
            kinds.insert(Response::Unit);
            continue;
        };
        for (content_type, media) in content {
            let schema = media.get("schema");
            let kind = match (content_type.as_str(), schema.and_then(ref_name)) {
                ("application/json", Some(name)) => Response::Json(response_type(spec, reg, name)?),
                _ => Response::Bytes,
            };
            kinds.insert(kind);
        }
    }
    let mut kinds: Vec<Response> = kinds.into_iter().collect();
    match kinds.len() {
        0 => bail!("no success response"),
        1 => Ok(kinds.pop().expect("len checked")),
        _ => bail!("multiple success response kinds: {kinds:?}"),
    }
}

fn response_type(spec: &Spec, reg: &mut Registry, name: &str) -> Result<String> {
    if let Some(env) = envelope(spec, name) {
        if env.request {
            bail!("request envelope `{name}` used as response");
        }
        let attrs = match env.resource_type.as_deref() {
            Some(_) => attributes_type(env.resource_type.as_deref())?,
            None => "Value".into(),
        };
        return Ok(if env.list {
            format!("ListEnvelope<{attrs}>")
        } else {
            format!("SingleEnvelope<{attrs}>")
        });
    }
    if name == "RelationshipResponseBody" {
        return Ok("Envelope<Option<RelationshipData>>".into());
    }
    Ok(reg.plain(name).rust())
}

fn attributes_type(resource_type: Option<&str>) -> Result<String> {
    let rt = resource_type.context("envelope without a single resource type")?;
    Ok(format!("{}Attributes", resource_name(rt)?))
}
