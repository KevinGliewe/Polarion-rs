//! Restricted mode ([`crate::PolarionBuilder::restrict_writes`]): the SOAP write operations a
//! write-restricted client may send, and the SOAP write filter that checks every write request
//! before it is sent, including those made through [`crate::Polarion::soap`].
//!
//! The filter works on the request body the SOAP client is about to send. It is a second line
//! behind the library's guards, so it is deliberately blunt. It parses the body and refuses:
//! - an operation that is not listed in [`OPERATIONS`];
//! - markup the request writer never emits (comments, CDATA, document types, processing
//!   instructions), elements in namespaces the services do not use, and a body that is not
//!   exactly the operation element;
//! - parameter elements that are not the operation's, out of order, or repeated (except arrays),
//!   and repeated fields inside a parameter;
//! - a request whose object (the parameter that names what the operation changes, see
//!   [`Subject`]) is missing or not a write target, or that names any object URI (in an element's
//!   text or an attribute value, the whole value) that is not a write target, an object owned by
//!   the target, or the project of a budgeted create (in a session of its own, objects it
//!   created count as targets, see [`Created`]);
//! - identity fields (`id`, `project`, ...) in the object of an update, which could name another
//!   object than its URI;
//! - `status` and `resolution` in field writes; in documents link settings (structure link role,
//!   auto-suspect, link direction) and work item macros (`module-workitem`), which change the
//!   work items of the document, and a body or outline numbering the library did not check
//!   first (see [`Approvals`]).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use roxmltree::{Document, Node};

use crate::client::{CreateKind, Creates, is_status_key};
use crate::uri;

/// How restricted mode treats a write operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    /// Changes the object its [`Subject`] names, which must be a write target; every other
    /// object URI in the request must be a target too.
    Target,
    /// Changes comments or work records: its [`Subject`] is a write target or owned by one
    /// ([`uri::owner_of`]), and every object URI in the request is that target or owned by it.
    Owned,
    /// Creates an object of a budgeted kind in the create budget's project.
    Create(CreateKind),
    /// Starts or ends an explicit transaction; names no object.
    Transaction,
}

/// Where a request names the object the operation changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Subject {
    /// The text of the first parameter (a URI).
    First,
    /// The `uri` attribute of the first parameter (the object, e.g. `updateWorkItem`'s
    /// `content`), which must not carry identity fields.
    FirstUri,
    /// The text of the only `parentItemURI` of the first parameter (`setCustomField`).
    ParentItemUri,
    /// None: creates (checked by their project) and transactions.
    None,
}

/// The write operations a write-restricted client may send, with their parameter elements in
/// order (`[]`: an array, sent as repeated elements). Every other write operation of the SOAP API
/// is refused (`tests/write_targets.rs` lists them, with the reason, and checks the parameters
/// against the generated services).
#[rustfmt::skip]
pub(crate) const OPERATIONS: &[(&str, Class, Subject, &[&str])] = &[
    // Work items.
    ("updateWorkItem", Class::Target, Subject::FirstUri, &["content"]),
    ("setCustomField", Class::Target, Subject::ParentItemUri, &["customField"]),
    ("setFieldsNull", Class::Target, Subject::First, &["workitemURI", "fields[]"]),
    ("addHyperlink", Class::Target, Subject::First, &["workitemURI", "url", "role"]),
    ("removeHyperlink", Class::Target, Subject::First, &["workitemURI", "url"]),
    ("addCategory", Class::Target, Subject::First, &["workitemURI", "categoryId"]),
    ("removeCategory", Class::Target, Subject::First, &["workitemURI", "categoryId"]),
    ("addPlaningContraint", Class::Target, Subject::First, &["workitemURI", "date", "constraint"]),
    ("removePlaningConstraint", Class::Target, Subject::First, &["workitemURI", "date", "constraint"]),
    ("addAssignee", Class::Target, Subject::First, &["workitemURI", "assigneeId"]),
    ("removeAssignee", Class::Target, Subject::First, &["workitemURI", "assigneeId"]),
    ("addApprovee", Class::Target, Subject::First, &["workitemURI", "approveeId"]),
    ("removeApprovee", Class::Target, Subject::First, &["workitemURI", "approveeId"]),
    ("editApproval", Class::Target, Subject::First, &["workitemURI", "approveeId", "status"]),
    ("createAttachment", Class::Target, Subject::First, &["workitemURI", "fileName", "title", "data"]),
    ("updateAttachment", Class::Target, Subject::First, &["workitemURI", "id", "fileName", "title", "data"]),
    ("deleteAttachment", Class::Target, Subject::First, &["workitemURI", "id"]),
    ("createWorkRecord", Class::Target, Subject::First, &["workitemURI", "user", "date", "timeSpent"]),
    ("createWorkRecordWithTypeAndComment", Class::Target, Subject::First, &["workitemURI", "user", "date", "type", "timeSpent", "comment"]),
    ("deleteWorkRecord", Class::Owned, Subject::First, &["workitemURI", "workRecordURI"]),
    ("setLocalizedWorkItemTitle", Class::Target, Subject::First, &["uri", "language", "title"]),
    ("setLocalizedWorkItemDescription", Class::Target, Subject::First, &["uri", "language", "description"]),
    ("addLinkedItem", Class::Target, Subject::First, &["workitemURI", "linkedWorkitemURI", "role"]),
    ("addLinkedItemWithRev", Class::Target, Subject::First, &["in0", "in1", "in2", "in3", "in4"]),
    ("removeLinkedItem", Class::Target, Subject::First, &["workitemURI", "linkedItemURI", "role"]),
    // Comments (work items and documents).
    ("addComment", Class::Owned, Subject::First, &["parentObjectUri", "title", "content"]),
    ("createCommentNew", Class::Target, Subject::First, &["parentURI", "title", "content", "visibleTo[]"]),
    ("setResolvedComment", Class::Owned, Subject::First, &["commentURI", "resolved"]),
    ("setCommentTags", Class::Owned, Subject::First, &["commentURI", "tags[]"]),
    // Documents and baselines.
    ("updateModule", Class::Target, Subject::FirstUri, &["module"]),
    ("updateBaseline", Class::Target, Subject::FirstUri, &["baseline"]),
    // Test management.
    ("updateTestRun", Class::Target, Subject::FirstUri, &["content"]),
    ("updateWikiContentForTestRun", Class::Target, Subject::First, &["testRunUri", "content"]),
    ("addTestRecordToTestRun", Class::Target, Subject::First, &["testRunUri", "testRecord"]),
    ("updateTestRecordAtIndex", Class::Target, Subject::First, &["testRunUri", "index", "testRecord"]),
    ("setTestSteps", Class::Target, Subject::First, &["workItemURI", "testSteps[]"]),
    ("addAttachmentToTestRun", Class::Target, Subject::First, &["testRunUri", "fileName", "title", "data"]),
    ("updateTestRunAttachment", Class::Target, Subject::First, &["testRunUri", "fileName", "title", "data"]),
    ("deleteTestRunAttachment", Class::Target, Subject::First, &["testRunUri", "fileName"]),
    ("addAttachmentToTestRecord", Class::Target, Subject::First, &["testRunUri", "index", "fileName", "title", "data"]),
    ("deleteAttachmentFromTestRecord", Class::Target, Subject::First, &["testRunUri", "index", "fileName"]),
    ("addAttachmentToTestStep", Class::Target, Subject::First, &["testRunUri", "index", "testStepIndex", "fileName", "title", "data"]),
    ("deleteAttachmentFromTestStep", Class::Target, Subject::First, &["testRunUri", "index", "testStepIndex", "fileName"]),
    // Planning.
    ("updatePlan", Class::Target, Subject::FirstUri, &["content"]),
    ("setPlanWikiContent", Class::Target, Subject::First, &["uri", "text"]),
    ("addPlanItems", Class::Target, Subject::First, &["uri", "items[]"]),
    ("removePlanItems", Class::Target, Subject::First, &["uri", "items[]"]),
    ("addPlanAllowedType", Class::Target, Subject::First, &["uri", "workItemType"]),
    ("removePlanAllowedType", Class::Target, Subject::First, &["uri", "workItemType"]),
    // Creates within the budget.
    ("createWorkItem", Class::Create(CreateKind::WorkItem), Subject::None, &["content"]),
    ("createDocument", Class::Create(CreateKind::Document), Subject::None, &["projectId", "location", "documentName", "documentTitle", "allowedWITypes[]", "structureLinkRole", "homePageContent"]),
    ("createTestRun", Class::Create(CreateKind::TestRun), Subject::None, &["project", "id", "template"]),
    ("createTestRunWithTitle", Class::Create(CreateKind::TestRun), Subject::None, &["project", "id", "title", "template"]),
    // Transactions.
    ("beginTransaction", Class::Transaction, Subject::None, &[]),
    ("endTransaction", Class::Transaction, Subject::None, &["rollback"]),
];

/// Operations whose request carries field values: `status` and `resolution` are refused there.
const FIELD_WRITES: &[&str] = &[
    "updateWorkItem",
    "setCustomField",
    "setFieldsNull",
    "createWorkItem",
    "updateModule",
    "updateTestRun",
    "updatePlan",
];

/// Fields of an updated object that could name another object than its `uri`.
const IDENTITY_FIELDS: &[&str] = &[
    "id",
    "project",
    "projectId",
    "projectURI",
    "moduleFolder",
    "moduleName",
    "location",
];

/// The start of every object URI.
const OBJECT_URI: &str = "subterra:data-service:objects:";

/// The macro that places a work item in a document body.
pub(crate) const WORK_ITEM_MACRO: &str = "module-workitem";

const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
const XSD: &str = "http://www.w3.org/2001/XMLSchema";

fn find(
    operation: &str,
) -> Option<&'static (&'static str, Class, Subject, &'static [&'static str])> {
    OPERATIONS.iter().find(|(op, ..)| *op == operation)
}

/// Whether a text contains a work item macro (any case).
pub(crate) fn has_work_item_macro(text: &str) -> bool {
    text.to_ascii_lowercase().contains(WORK_ITEM_MACRO)
}

/// Document writes the library checked (the document holds no work items): the filter lets a
/// document's body or outline numbering through only with one of these, each good for one
/// request, so a raw [`crate::Polarion::soap`] write never changes them.
#[derive(Debug, Default)]
pub(crate) struct Approvals(Mutex<Vec<String>>);

impl Approvals {
    /// One body write to the document `uri` was checked.
    pub(crate) fn approve(&self, uri: &str) {
        self.0
            .lock()
            .expect("approvals lock")
            .push(uri::strip_revision(uri).to_owned());
    }

    /// Use up one approval for `uri`; `false` when there is none.
    fn take(&self, uri: &str) -> bool {
        let uri = uri::strip_revision(uri);
        let mut approved = self.0.lock().expect("approvals lock");
        match approved.iter().position(|a| a == uri) {
            Some(i) => {
                approved.swap_remove(i);
                true
            }
            None => false,
        }
    }
}

/// The objects a session of its own ([`crate::Polarion::own_session`]) created, by URI without
/// `%revision` ([`crate::Polarion::grant_created`]): its guards and its SOAP write filter treat
/// them as write targets, for its follow-up writes only. Each session has its own set, so
/// neither the main session nor another one (nor [`crate::Polarion::soap`] of the main client)
/// can write to them. Creates never name them (`Filter::allows_create` checks only targets).
#[derive(Debug, Default)]
pub(crate) struct Created(Mutex<BTreeSet<String>>);

impl Created {
    /// The session created the object `uri`.
    pub(crate) fn insert(&self, uri: &str) {
        self.0
            .lock()
            .expect("created lock")
            .insert(uri::strip_revision(uri).to_owned());
    }

    /// Whether the session created the object `uri` (a `%revision` suffix is ignored).
    pub(crate) fn contains(&self, uri: &str) -> bool {
        self.0
            .lock()
            .expect("created lock")
            .contains(uri::strip_revision(uri))
    }
}

/// The SOAP write filter of a write-restricted client, shared by its sessions of their own.
#[derive(Debug)]
pub(crate) struct Filter {
    targets: BTreeSet<String>,
    /// Its own counters: at most the budget's number of create requests is ever sent, whether
    /// through the library or [`crate::Polarion::soap`] (a create repeated after a session
    /// renewal passes the filter again, so it counts too).
    creates: Option<Creates>,
    approvals: Arc<Approvals>,
}

impl Filter {
    pub(crate) fn new(
        targets: Option<&BTreeSet<String>>,
        creates: Option<&Creates>,
        approvals: Arc<Approvals>,
    ) -> Self {
        Self {
            targets: targets.cloned().unwrap_or_default(),
            creates: creates.map(|c| Creates::new(&c.project, c.budget)),
            approvals,
        }
    }

    /// Whether the write `operation` with this request body may be sent, by a session that
    /// created the objects in `created` (a session of its own) or none (the main session).
    pub(crate) fn allows(&self, operation: &str, body: &str, created: Option<&Created>) -> bool {
        let Some((_, class, subject, params)) = find(operation) else {
            return false;
        };
        // Comments, CDATA, document types and processing instructions: never written by the
        // request writer, and a substring search would not see through them.
        if body.contains("<!") || body.contains("<?") {
            return false;
        }
        let wrapped = wrap(body);
        let Ok(document) = Document::parse(&wrapped) else {
            return false;
        };
        let Some(root) = operation_element(&document, operation) else {
            return false;
        };
        let Some(args) = arguments(root, params) else {
            return false;
        };
        // A document body or outline numbering needs the library's approval. This request uses
        // it up whatever the other checks say, so a refused request leaves none behind for a
        // later one.
        let needs_approval = operation == "updateModule" && changes_document_items(root);
        let approved = needs_approval
            && self
                .subject(*subject, &args)
                .is_some_and(|s| self.approvals.take(&s));
        if !args.iter().all(|(_, node)| distinct_fields(*node)) {
            return false;
        }
        if FIELD_WRITES.contains(&operation) && writes_status(root) {
            return false;
        }
        if !document_write_allowed(operation, root) {
            return false;
        }
        let uris = object_uris(root);
        match class {
            Class::Target => self.subject(*subject, &args).is_some_and(|s| {
                self.is_writable(&s, created)
                    && uris.iter().all(|u| self.is_writable(u, created))
                    && (!needs_approval || approved)
            }),
            Class::Owned => self.subject(*subject, &args).is_some_and(|s| {
                let owner = if self.is_writable(&s, created) {
                    Some(uri::strip_revision(&s).to_owned())
                } else {
                    uri::owner_of(&s).filter(|o| self.is_writable(o, created))
                };
                owner.is_some_and(|owner| {
                    uris.iter().all(|u| {
                        uri::strip_revision(u) == owner
                            || uri::owner_of(u).is_some_and(|o| o == owner)
                    })
                })
            }),
            Class::Transaction => uris.is_empty(),
            Class::Create(kind) => self.allows_create(*kind, &args, &uris),
        }
    }

    fn is_target(&self, uri: &str) -> bool {
        self.targets.contains(uri::strip_revision(uri))
    }

    /// A target, or an object the sending session created: what a write to an existing object
    /// may change or name. Creates use [`Filter::is_target`] only.
    fn is_writable(&self, uri: &str, created: Option<&Created>) -> bool {
        self.is_target(uri) || created.is_some_and(|c| c.contains(uri))
    }

    /// The object the request names (see [`Subject`]); `None` when it names none in the expected
    /// place.
    fn subject(&self, subject: Subject, args: &[(usize, Node<'_, '_>)]) -> Option<String> {
        let (index, first) = args.first()?;
        if *index != 0 {
            return None;
        }
        match subject {
            Subject::First => leaf_text(*first),
            Subject::FirstUri => {
                let identity = first
                    .children()
                    .filter(Node::is_element)
                    .any(|c| IDENTITY_FIELDS.contains(&c.tag_name().name()));
                if identity {
                    return None;
                }
                first.attribute("uri").map(str::to_owned)
            }
            Subject::ParentItemUri => {
                let mut parents = first
                    .children()
                    .filter(|c| c.is_element() && c.tag_name().name() == "parentItemURI");
                let parent = parents.next()?;
                if parents.next().is_some() {
                    return None;
                }
                leaf_text(parent)
            }
            Subject::None => None,
        }
    }

    fn allows_create(
        &self,
        kind: CreateKind,
        args: &[(usize, Node<'_, '_>)],
        uris: &[String],
    ) -> bool {
        let Some(creates) = &self.creates else {
            return false;
        };
        let project = creates.project.as_str();
        let Some((0, first)) = args.first() else {
            return false;
        };
        let in_project = match kind {
            // The work item's project, by URI (and id, when given); nothing names the new item.
            CreateKind::WorkItem => {
                let projects: Vec<Node<'_, '_>> = first
                    .children()
                    .filter(|c| c.is_element() && c.tag_name().name() == "project")
                    .collect();
                let project_uri = match projects.as_slice() {
                    [p] => p
                        .attribute("uri")
                        .filter(|u| uri::is_project_uri(u, project)),
                    _ => None,
                };
                let id_ok = projects.iter().all(|p| {
                    p.children()
                        .filter(|c| c.is_element() && c.tag_name().name() == "id")
                        .all(|id| leaf_text(id).as_deref() == Some(project))
                });
                first.attribute("uri").is_none()
                    && id_ok
                    && project_uri
                        .is_some_and(|pu| uris.iter().all(|u| u == pu || self.is_target(u)))
            }
            CreateKind::Document | CreateKind::TestRun => {
                leaf_text(*first).as_deref() == Some(project)
                    && uris.iter().all(|u| self.is_target(u))
            }
        };
        in_project && creates.take(kind)
    }
}

/// The body inside an element that declares the prefixes the SOAP envelope declares.
fn wrap(body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 2048);
    out.push_str("<polarion-rs-filter xmlns:xsi=\"");
    out.push_str(XSI);
    out.push_str("\" xmlns:xsd=\"");
    out.push_str(XSD);
    out.push('"');
    for (prefix, uri) in polarion_soap::NAMESPACES {
        out.push_str(" xmlns:");
        out.push_str(prefix);
        out.push_str("=\"");
        out.push_str(uri);
        out.push('"');
    }
    out.push('>');
    out.push_str(body);
    out.push_str("</polarion-rs-filter>");
    out
}

/// The operation element: the only node in the body, named `operation`, without attributes, and
/// every element in it in a namespace of the services.
fn operation_element<'a, 'i>(document: &'a Document<'i>, operation: &str) -> Option<Node<'a, 'i>> {
    let wrapper = document.root_element();
    let mut children = wrapper.children();
    let root = children.next().filter(Node::is_element)?;
    if children.next().is_some()
        || root.tag_name().name() != operation
        || root.attributes().next().is_some()
    {
        return None;
    }
    let known = |ns: Option<&str>| {
        ns.is_some_and(|ns| {
            ns == XSI || ns == XSD || polarion_soap::NAMESPACES.iter().any(|(_, u)| *u == ns)
        })
    };
    let clean = root.descendants().all(|n| {
        if n.is_element() {
            known(n.tag_name().namespace())
                && n.attributes()
                    .all(|a| a.namespace().is_none() || known(a.namespace()))
        } else {
            n.is_text()
        }
    });
    clean.then_some(root)
}

/// The parameter elements with their index in `params`: only the operation's parameters, in
/// order, each at most once unless it is an array; no text between them.
fn arguments<'a, 'i>(root: Node<'a, 'i>, params: &[&str]) -> Option<Vec<(usize, Node<'a, 'i>)>> {
    let mut out = Vec::new();
    let mut next = 0;
    for child in root.children() {
        if !child.is_element() {
            if child.text().is_some_and(|t| !t.trim().is_empty()) {
                return None;
            }
            continue;
        }
        let name = child.tag_name().name();
        loop {
            let param = params.get(next)?;
            let (param, array) = match param.strip_suffix("[]") {
                Some(p) => (p, true),
                None => (*param, false),
            };
            if param == name {
                out.push((next, child));
                if !array {
                    next += 1;
                }
                break;
            }
            next += 1;
        }
    }
    Some(out)
}

/// Whether, in every element of a parameter, the child elements either all have different names
/// (fields) or all the same (array items).
fn distinct_fields(param: Node<'_, '_>) -> bool {
    param.descendants().filter(Node::is_element).all(|node| {
        let names: Vec<&str> = node
            .children()
            .filter(Node::is_element)
            .map(|c| c.tag_name().name())
            .collect();
        let distinct: BTreeSet<&str> = names.iter().copied().collect();
        distinct.len() == names.len() || distinct.len() == 1
    })
}

/// The text of an element without child elements; `None` for other elements and empty text.
fn leaf_text(node: Node<'_, '_>) -> Option<String> {
    if node.children().any(|c| c.is_element()) {
        return None;
    }
    let text: String = node.children().filter_map(|c| c.text()).collect();
    (!text.is_empty()).then_some(text)
}

/// Every value in the request that contains an object URI (case-insensitive prefix): the whole
/// text of an element, or the whole value of an attribute. Text around a URI stays attached, so
/// such a value never matches a target.
fn object_uris(root: Node<'_, '_>) -> Vec<String> {
    let mut out = Vec::new();
    let mut add = |value: &str| {
        if value.to_ascii_lowercase().contains(OBJECT_URI) {
            out.push(value.to_owned());
        }
    };
    for node in root.descendants() {
        if node.is_element() {
            for attribute in node.attributes() {
                add(attribute.value());
            }
            if !node.children().any(|c| c.is_element()) {
                let text: String = node.children().filter_map(|c| c.text()).collect();
                add(&text);
            }
        } else if let Some(text) = node.text() {
            // Text next to child elements (mixed content).
            if node
                .parent()
                .is_some_and(|p| p.children().any(|c| c.is_element()))
            {
                add(text);
            }
        }
    }
    out
}

/// Whether the request writes `status` or `resolution`: an element of that name (any case), or a
/// `key` or `fields` element with that text (custom fields, cleared fields).
fn writes_status(root: Node<'_, '_>) -> bool {
    root.descendants().filter(Node::is_element).any(|node| {
        let name = node.tag_name().name();
        is_status_key(name)
            || (matches!(name, "key" | "fields")
                && node
                    .children()
                    .filter_map(|c| c.text())
                    .collect::<String>()
                    .split_whitespace()
                    .any(is_status_key))
    })
}

/// Fields of a document the library never updates in restricted mode: they change the links of
/// its work items.
const DOCUMENT_LINK_FIELDS: &[&str] = &[
    "structureLinkRole",
    "autoSuspect",
    "areLinksFromParentToChild",
];

/// A document create or update names no work item (no work item macro), and an update changes
/// no link setting.
fn document_write_allowed(operation: &str, root: Node<'_, '_>) -> bool {
    match operation {
        "createDocument" => !root
            .descendants()
            .any(|n| n.text().is_some_and(has_work_item_macro)),
        "updateModule" => !root.descendants().any(|n| {
            (n.is_element() && DOCUMENT_LINK_FIELDS.contains(&n.tag_name().name()))
                || n.text().is_some_and(has_work_item_macro)
        }),
        _ => true,
    }
}

/// Whether a document update writes what changes the document's work items when it has any:
/// the body or the outline numbering.
fn changes_document_items(root: Node<'_, '_>) -> bool {
    root.descendants().any(|n| {
        n.is_element()
            && matches!(
                n.tag_name().name(),
                "homePageContent" | "usesOutlineNumbering"
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::CreateBudget;

    const BASE: &str = "subterra:data-service:objects:/default/";

    fn filter(targets: &[&str], creates: Option<(&str, CreateBudget)>) -> Filter {
        let targets: BTreeSet<String> = targets.iter().map(|t| (*t).to_owned()).collect();
        let creates = creates.map(|(p, b)| Creates::new(p, b));
        Filter::new(Some(&targets), creates.as_ref(), Arc::default())
    }

    impl Filter {
        /// [`Filter::allows`] for a request of the main session.
        fn allows_main(&self, operation: &str, body: &str) -> bool {
            self.allows(operation, body, None)
        }
    }

    fn parsed(body: &str, check: impl FnOnce(Node<'_, '_>)) {
        let wrapped = wrap(body);
        let document = Document::parse(&wrapped).unwrap();
        check(document.root_element().first_element_child().unwrap());
    }

    #[test]
    fn operations_are_listed_once() {
        let names: BTreeSet<&str> = OPERATIONS.iter().map(|(op, ..)| *op).collect();
        assert_eq!(names.len(), OPERATIONS.len());
        for op in FIELD_WRITES {
            assert!(find(op).is_some(), "{op}");
        }
        for (op, class, subject, params) in OPERATIONS {
            let creates_or_transactions = matches!(class, Class::Create(_) | Class::Transaction);
            assert_eq!(creates_or_transactions, *subject == Subject::None, "{op}");
            assert!(
                *subject == Subject::None || !params.is_empty(),
                "{op} names its object in a parameter"
            );
        }
    }

    #[test]
    fn uris_are_whole_values() {
        let a = format!("{BASE}P${{WorkItem}}P-1");
        let doc = format!("{BASE}P${{Module}}{{moduleFolder}}Specs#A &amp; B");
        let body = format!(
            r#"<trackeri:x><trackeri:a uri="{a}"/><trackeri:y>{doc}</trackeri:y><trackeri:z>SUBTERRA:data-service:objects:/default/Q${{WorkItem}}Q-1</trackeri:z></trackeri:x>"#
        );
        parsed(&body, |root| {
            assert_eq!(
                object_uris(root),
                [
                    a.clone(),
                    format!("{BASE}P${{Module}}{{moduleFolder}}Specs#A & B"),
                    "SUBTERRA:data-service:objects:/default/Q${WorkItem}Q-1".to_owned(),
                ]
            );
        });
        // A URI in escaped rich text keeps the markup around it; character references are
        // resolved; a quote in text is part of the value.
        let text = format!(
            "<trackeri:x><trackeri:c>&lt;a href=&quot;{a}&quot;&gt;x&lt;/a&gt;</trackeri:c><trackeri:d>&#115;ubterra:data-service:objects:/default/Q${{WorkItem}}Q-2</trackeri:d><trackeri:e>{a}\"x</trackeri:e></trackeri:x>"
        );
        parsed(&text, |root| {
            assert_eq!(
                object_uris(root),
                [
                    format!(r#"<a href="{a}">x</a>"#),
                    format!("{BASE}Q${{WorkItem}}Q-2"),
                    format!("{a}\"x"),
                ]
            );
        });
    }

    #[test]
    fn status_writes_are_recognized() {
        for body in [
            "<trackeri:x><trackert:status><trackert:id>open</trackert:id></trackert:status></trackeri:x>",
            r#"<trackeri:x><trackert:Resolution xsi:nil="true"/></trackeri:x>"#,
            "<trackeri:x><trackert:key> Status </trackert:key></trackeri:x>",
            "<trackeri:x><trackeri:fields>resolution</trackeri:fields></trackeri:x>",
        ] {
            parsed(body, |root| assert!(writes_status(root), "{body}"));
        }
        for body in [
            "<trackeri:x><trackert:title>status</trackert:title></trackeri:x>",
            "<trackeri:x><trackert:statusX>1</trackert:statusX></trackeri:x>",
            "<trackeri:x><trackert:key>severity</trackert:key><trackeri:fields>dueDate</trackeri:fields></trackeri:x>",
            "<trackeri:x><trackert:description>&lt;status&gt;</trackert:description></trackeri:x>",
        ] {
            parsed(body, |root| assert!(!writes_status(root), "{body}"));
        }
    }

    fn hyperlink(uri: &str) -> String {
        format!(
            "<trackeri:addHyperlink><trackeri:workitemURI>{uri}</trackeri:workitemURI><trackeri:url>https://x</trackeri:url><trackeri:role><trackert:id>r</trackert:id></trackeri:role></trackeri:addHyperlink>"
        )
    }

    #[test]
    fn markup_the_writer_never_emits_is_refused() {
        let a = format!("{BASE}P${{WorkItem}}P-1");
        let f = filter(&[&a], None);
        assert!(f.allows_main("addHyperlink", &hyperlink(&a)));
        for body in [
            hyperlink(&format!("{a}<!-- x -->")),
            hyperlink(&format!("<![CDATA[{a}]]>")),
            hyperlink(&format!("{a}<?pi x?>")),
            format!("<!DOCTYPE x>{}", hyperlink(&a)),
            // Two operations, text beside it, an attribute on it.
            format!("{}{}", hyperlink(&a), hyperlink(&a)),
            format!("{} ", hyperlink(&a)).replacen("<trackeri:url>", "x<trackeri:url>", 1),
            hyperlink(&a).replacen(
                "<trackeri:addHyperlink>",
                "<trackeri:addHyperlink a=\"1\">",
                1,
            ),
            // An unknown namespace, a redeclared prefix, an undeclared one, not XML.
            hyperlink(&a).replace("trackeri:url", "x:url").replacen(
                "<x:url>",
                "<x:url xmlns:x=\"urn:x\">",
                1,
            ),
            hyperlink(&a).replacen(
                "<trackeri:addHyperlink>",
                "<trackeri:addHyperlink xmlns:trackeri=\"urn:x\">",
                1,
            ),
            hyperlink(&a).replace("trackeri:url", "nope:url"),
            hyperlink(&a).replace("</trackeri:addHyperlink>", ""),
        ] {
            assert!(!f.allows_main("addHyperlink", &body), "{body}");
        }
    }

    #[test]
    fn parameters_are_the_operations_in_order() {
        let a = format!("{BASE}P${{WorkItem}}P-1");
        let b = format!("{BASE}P${{WorkItem}}P-3");
        let f = filter(&[&a], None);
        let custom = |inner: &str| {
            format!(
                "<trackeri:setCustomField><trackeri:customField>{inner}</trackeri:customField></trackeri:setCustomField>"
            )
        };
        let field = |parent: &str| {
            format!(
                "<trackert:key>risk</trackert:key><trackert:parentItemURI>{parent}</trackert:parentItemURI><trackert:value xsi:type=\"xsd:int\">1</trackert:value>"
            )
        };
        assert!(f.allows_main("setCustomField", &custom(&field(&a))));
        for body in [
            // A second parent, a second field element, a parameter twice, an unknown one.
            custom(&format!(
                "{}<trackert:parentItemURI>{a}</trackert:parentItemURI>",
                field(&a)
            )),
            custom(&format!("{}<trackert:key>other</trackert:key>", field(&a))),
            format!(
                "<trackeri:setCustomField><trackeri:customField>{0}</trackeri:customField><trackeri:customField>{0}</trackeri:customField></trackeri:setCustomField>",
                field(&a)
            ),
            format!(
                "<trackeri:setCustomField><trackeri:customField>{}</trackeri:customField><trackeri:other/></trackeri:setCustomField>",
                field(&a)
            ),
            // The parent in another place than the object's.
            custom(&field(&b).replace(
                "<trackert:key>risk</trackert:key>",
                &format!("<trackert:key>{a}</trackert:key>"),
            )),
            custom("<trackert:key>risk</trackert:key>"),
        ] {
            assert!(!f.allows_main("setCustomField", &body), "{body}");
        }
        // Parameters out of order; arrays repeat.
        let swapped = hyperlink(&a).replace(
            "<trackeri:workitemURI>",
            "<trackeri:url>https://y</trackeri:url><trackeri:workitemURI>",
        );
        assert!(!f.allows_main("addHyperlink", &swapped));
        let cleared = format!(
            "<trackeri:setFieldsNull><trackeri:workitemURI>{a}</trackeri:workitemURI><trackeri:fields>dueDate</trackeri:fields><trackeri:fields>severity</trackeri:fields></trackeri:setFieldsNull>"
        );
        assert!(f.allows_main("setFieldsNull", &cleared));
    }

    #[test]
    fn the_object_is_named_where_the_operation_takes_it() {
        let a = format!("{BASE}P${{WorkItem}}P-1");
        let b = format!("{BASE}P${{WorkItem}}P-3");
        let f = filter(&[&a], None);
        let update = |attrs: &str, fields: &str| {
            format!(
                "<trackeri:updateWorkItem><trackeri:content{attrs}>{fields}</trackeri:content></trackeri:updateWorkItem>"
            )
        };
        let title = |t: &str| format!("<trackert:title>{t}</trackert:title>");
        assert!(f.allows_main(
            "updateWorkItem",
            &update(&format!(" uri=\"{a}\""), &title("x"))
        ));
        for body in [
            // No URI: the target's URI as the title does not make it the object.
            update("", &title(&a)),
            update(
                "",
                &format!(
                    "<trackert:id>P-3</trackert:id><trackert:project><trackert:id>P</trackert:id></trackert:project>{}",
                    title(&a)
                ),
            ),
            // Identity fields beside the URI.
            update(&format!(" uri=\"{a}\""), "<trackert:id>P-3</trackert:id>"),
            update(
                &format!(" uri=\"{a}\""),
                "<trackert:project><trackert:id>Q</trackert:id></trackert:project>",
            ),
            update(&format!(" uri=\"{b}\""), &title("x")),
        ] {
            assert!(!f.allows_main("updateWorkItem", &body), "{body}");
        }
        // The first parameter must be the object, not the target named elsewhere.
        let link = |item: &str, linked: &str| {
            format!(
                "<trackeri:addLinkedItem><trackeri:workitemURI>{item}</trackeri:workitemURI><trackeri:linkedWorkitemURI>{linked}</trackeri:linkedWorkitemURI><trackeri:role><trackert:id>r</trackert:id></trackeri:role></trackeri:addLinkedItem>"
            )
        };
        assert!(f.allows_main("addLinkedItem", &link(&a, &a)));
        assert!(!f.allows_main("addLinkedItem", &link(&b, &a)));
        assert!(!f.allows_main("addLinkedItem", &link(&a, &b)));
        let nil = "<trackeri:addHyperlink><trackeri:workitemURI xsi:nil=\"true\"/><trackeri:url>x</trackeri:url></trackeri:addHyperlink>";
        assert!(!f.allows_main("addHyperlink", nil));
        let missing =
            "<trackeri:addHyperlink><trackeri:url>x</trackeri:url></trackeri:addHyperlink>";
        assert!(!f.allows_main("addHyperlink", missing));
    }

    #[test]
    fn targets_owned_objects_and_unknown_operations() {
        let a = format!("{BASE}P${{WorkItem}}P-1");
        let f = filter(&[&a], None);
        let comment = |uri: &str| {
            format!(
                "<trackeri:addComment><trackeri:parentObjectUri>{uri}</trackeri:parentObjectUri><trackeri:title xsi:nil=\"true\"/></trackeri:addComment>"
            )
        };
        let record = |item: &str, record: &str| {
            format!(
                "<trackeri:deleteWorkRecord><trackeri:workitemURI>{item}</trackeri:workitemURI><trackeri:workRecordURI>{record}</trackeri:workRecordURI></trackeri:deleteWorkRecord>"
            )
        };
        // The formats Polarion uses: `…${Comment}{WorkItem}ID#KEY`, `…${WorkRecord}{WorkItem}ID#KEY`.
        let a_comment = |item: &str, key: &str| {
            format!(
                "{}#{key}",
                item.replacen("${WorkItem}", "${Comment}{WorkItem}", 1)
            )
        };
        let a_record = |item: &str, key: &str| {
            format!(
                "{}#{key}",
                item.replacen("${WorkItem}", "${WorkRecord}{WorkItem}", 1)
            )
        };
        assert!(f.allows_main("addHyperlink", &hyperlink(&a)));
        assert!(f.allows_main("addHyperlink", &hyperlink(&format!("{a}%42"))));
        assert!(f.allows_main("addComment", &comment(&a)));
        assert!(f.allows_main("addComment", &comment(&a_comment(&a, "7"))));
        assert!(f.allows_main("deleteWorkRecord", &record(&a, &a_record(&a, "wr-1"))));
        // Only comment and work record operations take owned objects; the owner must be a
        // target, and the work record the work item's.
        assert!(!f.allows_main("addHyperlink", &hyperlink(&a_comment(&a, "7"))));
        let a12 = format!("{BASE}P${{WorkItem}}P-12");
        assert!(!f.allows_main("addComment", &comment(&a_comment(&a12, "7"))));
        assert!(!f.allows_main("addComment", &comment(&a_comment(&a, "x${WorkItem}P-3"))));
        let other = format!("{BASE}P${{WorkItem}}P-2");
        assert!(!f.allows_main("deleteWorkRecord", &record(&a, &a_record(&other, "wr-1"))));

        assert!(!f.allows_main("addHyperlink", &hyperlink(&other)));
        assert!(!f.allows_main("deleteWorkItem", &hyperlink(&a)));
        assert!(!f.allows_main("performWorkflowAction", &hyperlink(&a)));
        assert!(!f.allows_main("noSuchOperation", &hyperlink(&a)));
        // Status and resolution.
        let status = format!(
            r#"<trackeri:updateWorkItem><trackeri:content uri="{a}"><trackert:status><trackert:id>done</trackert:id></trackert:status></trackeri:content></trackeri:updateWorkItem>"#
        );
        assert!(!f.allows_main("updateWorkItem", &status));
        let approval = format!(
            "<trackeri:editApproval><trackeri:workitemURI>{a}</trackeri:workitemURI><trackeri:approveeId>me</trackeri:approveeId><trackeri:status><trackert:id>approved</trackert:id></trackeri:status></trackeri:editApproval>"
        );
        assert!(f.allows_main("editApproval", &approval));
        // Transactions name nothing.
        assert!(f.allows_main(
            "beginTransaction",
            "<sessioni:beginTransaction></sessioni:beginTransaction>"
        ));
        assert!(!f.allows_main(
            "endTransaction",
            &format!("<sessioni:endTransaction><sessioni:rollback>{a}</sessioni:rollback></sessioni:endTransaction>")
        ));
    }

    #[test]
    fn documents_keep_their_work_items() {
        let d = format!("{BASE}P${{Module}}{{moduleFolder}}Specs#SRS");
        let f = filter(&[&d], None);
        let update = |fields: &str| {
            format!(
                "<trackeri:updateModule><trackeri:module uri=\"{d}\">{fields}</trackeri:module></trackeri:updateModule>"
            )
        };
        let body = |html: &str| {
            format!(
                "<trackert:homePageContent><trackert:type>text/html</trackert:type><trackert:content>{html}</trackert:content></trackert:homePageContent>"
            )
        };
        let title = "<trackert:title>t</trackert:title>";
        assert!(f.allows_main("updateModule", &update(title)));
        // The body only after the library checked the document (once per check).
        let paragraph = body("&lt;p&gt;x&lt;/p&gt;");
        assert!(!f.allows_main("updateModule", &update(&paragraph)));
        f.approvals.approve(&d);
        assert!(f.allows_main("updateModule", &update(&paragraph)));
        assert!(!f.allows_main("updateModule", &update(&paragraph)));
        f.approvals.approve(&format!("{d}%12"));
        assert!(f.allows_main(
            "updateModule",
            &update("<trackert:usesOutlineNumbering>true</trackert:usesOutlineNumbering>")
        ));
        f.approvals.approve(&d);
        for fields in [
            body("&lt;div id=&quot;polarion_wiki macro name=module-workitem;params=id=P-3&quot;&gt;&lt;/div&gt;"),
            "<trackert:structureLinkRole><trackert:id>parent</trackert:id></trackert:structureLinkRole>".to_owned(),
            "<trackert:autoSuspect>true</trackert:autoSuspect>".to_owned(),
            "<trackert:moduleName>Other</trackert:moduleName>".to_owned(),
        ] {
            assert!(!f.allows_main("updateModule", &update(&fields)), "{fields}");
        }
        // A refused body uses up its approval: none is left for the next request.
        f.approvals.approve(&d);
        let other = format!("{BASE}P${{WorkItem}}P-3");
        assert!(!f.allows_main(
            "updateModule",
            &update(&body(&format!(
                "&lt;a href=&quot;{other}&quot;&gt;x&lt;/a&gt;"
            )))
        ));
        assert!(!f.allows_main("updateModule", &update(&paragraph)));
        // A new document never names work items either.
        let budget = CreateBudget {
            documents: 1,
            ..CreateBudget::default()
        };
        let creator = filter(&[], Some(("P", budget)));
        let create = |html: &str| {
            format!(
                "<trackeri:createDocument><trackeri:projectId>P</trackeri:projectId><trackeri:documentName>D</trackeri:documentName><trackeri:homePageContent>{html}</trackeri:homePageContent></trackeri:createDocument>"
            )
        };
        assert!(!creator.allows_main(
            "createDocument",
            &create(
                "&lt;div id=&quot;polarion_wiki macro name=Module-WorkItem;params=id=P-3&quot;/&gt;"
            )
        ));
        assert!(creator.allows_main("createDocument", &create("&lt;p&gt;D&lt;/p&gt;")));
    }

    #[test]
    fn creates_stay_in_the_project_and_the_budget() {
        let budget = CreateBudget {
            work_items: 2,
            test_runs: 1,
            documents: 0,
        };
        let f = filter(&[], Some(("P", budget)));
        let item = |project: &str, extra: &str| {
            format!(
                r#"<trackeri:createWorkItem><trackeri:content><trackert:project uri="{BASE}{project}${{Project}}{project}"><trackert:id>{project}</trackert:id></trackert:project>{extra}</trackeri:content></trackeri:createWorkItem>"#
            )
        };
        assert!(!f.allows_main("createWorkItem", &item("Q", "")));
        assert!(!f.allows_main("createWorkItem", "<trackeri:createWorkItem/>"));
        // The project by id only, with the budget's project URI in the title.
        let by_id = format!(
            "<trackeri:createWorkItem><trackeri:content><trackert:project><trackert:id>Q</trackert:id></trackert:project><trackert:title>{BASE}P${{Project}}P</trackert:title></trackeri:content></trackeri:createWorkItem>"
        );
        assert!(!f.allows_main("createWorkItem", &by_id));
        // A project URI with another id, a URI on the content, another object named.
        let mixed = item("P", "").replace(
            "<trackert:id>P</trackert:id>",
            "<trackert:id>Q</trackert:id>",
        );
        assert!(!f.allows_main("createWorkItem", &mixed));
        let with_uri = item("P", "").replacen(
            "<trackeri:content>",
            &format!("<trackeri:content uri=\"{BASE}P${{WorkItem}}P-1\">"),
            1,
        );
        assert!(!f.allows_main("createWorkItem", &with_uri));
        let linked = item(
            "P",
            &format!(
                "<trackert:description><trackert:content>{BASE}P${{WorkItem}}P-1</trackert:content></trackert:description>"
            ),
        );
        assert!(!f.allows_main("createWorkItem", &linked));
        assert!(f.allows_main("createWorkItem", &item("P", "")));
        assert!(f.allows_main(
            "createWorkItem",
            &item("P", "<trackert:title>x</trackert:title>")
        ));
        assert!(
            !f.allows_main("createWorkItem", &item("P", "")),
            "budget used up"
        );

        let run = |project: &str| {
            format!(
                "<testmanagementi:createTestRun><testmanagementi:project>{project}</testmanagementi:project><testmanagementi:id>R</testmanagementi:id></testmanagementi:createTestRun>"
            )
        };
        assert!(!f.allows_main("createTestRun", &run("Q")));
        let titled = run("P").replace("createTestRun>", "createTestRunWithTitle>");
        assert!(f.allows_main("createTestRunWithTitle", &titled));
        assert!(!f.allows_main("createTestRun", &run("P")), "budget used up");

        let doc = "<trackeri:createDocument><trackeri:projectId>P</trackeri:projectId></trackeri:createDocument>";
        assert!(
            !f.allows_main("createDocument", doc),
            "no documents in the budget"
        );
        assert!(!filter(&[], None).allows_main("createDocument", doc));
    }

    #[test]
    fn created_objects_are_writable_only_by_their_session() {
        let a = format!("{BASE}P${{WorkItem}}P-1");
        let new = format!("{BASE}P${{WorkItem}}P-9");
        let budget = CreateBudget {
            work_items: 2,
            ..CreateBudget::default()
        };
        let f = filter(&[&a], Some(("P", budget)));
        let created = Created::default();
        created.insert(&format!("{new}%5"));
        let other = Created::default();
        other.insert(&format!("{BASE}P${{WorkItem}}P-8"));
        // The session that created it, a revision included; not the main session or another one.
        assert!(created.contains(&new));
        assert!(f.allows("addHyperlink", &hyperlink(&new), Some(&created)));
        assert!(f.allows(
            "addHyperlink",
            &hyperlink(&format!("{new}%7")),
            Some(&created)
        ));
        assert!(!f.allows_main("addHyperlink", &hyperlink(&new)));
        assert!(!f.allows("addHyperlink", &hyperlink(&new), Some(&other)));
        // Links between targets and created objects, both ways.
        let link = |item: &str, linked: &str| {
            format!(
                "<trackeri:addLinkedItem><trackeri:workitemURI>{item}</trackeri:workitemURI><trackeri:linkedWorkitemURI>{linked}</trackeri:linkedWorkitemURI><trackeri:role><trackert:id>r</trackert:id></trackeri:role></trackeri:addLinkedItem>"
            )
        };
        assert!(f.allows("addLinkedItem", &link(&new, &a), Some(&created)));
        assert!(f.allows("addLinkedItem", &link(&a, &new), Some(&created)));
        assert!(!f.allows("addLinkedItem", &link(&a, &new), Some(&other)));
        // Comments count as their owner's.
        let comment = format!(
            "<trackeri:addComment><trackeri:parentObjectUri>{}#3</trackeri:parentObjectUri><trackeri:title xsi:nil=\"true\"/></trackeri:addComment>",
            new.replacen("${WorkItem}", "${Comment}{WorkItem}", 1)
        );
        assert!(f.allows("addComment", &comment, Some(&created)));
        assert!(!f.allows_main("addComment", &comment));
        // Identity fields and status stay refused for created objects too.
        let update = |fields: &str| {
            format!(
                "<trackeri:updateWorkItem><trackeri:content uri=\"{new}\">{fields}</trackeri:content></trackeri:updateWorkItem>"
            )
        };
        assert!(f.allows(
            "updateWorkItem",
            &update("<trackert:title>x</trackert:title>"),
            Some(&created)
        ));
        for fields in [
            "<trackert:id>P-1</trackert:id>",
            "<trackert:project><trackert:id>Q</trackert:id></trackert:project>",
            "<trackert:status><trackert:id>done</trackert:id></trackert:status>",
        ] {
            assert!(
                !f.allows("updateWorkItem", &update(fields), Some(&created)),
                "{fields}"
            );
        }
        // A create never names a created object (creates check targets only).
        let create = |extra: &str| {
            format!(
                r#"<trackeri:createWorkItem><trackeri:content><trackert:project uri="{BASE}P${{Project}}P"><trackert:id>P</trackert:id></trackert:project>{extra}</trackeri:content></trackeri:createWorkItem>"#
            )
        };
        let naming = create(&format!("<trackert:title>{new}</trackert:title>"));
        assert!(!f.allows("createWorkItem", &naming, Some(&created)));
        assert!(f.allows(
            "createWorkItem",
            &create(&format!("<trackert:title>{a}</trackert:title>")),
            Some(&created)
        ));
    }
}
