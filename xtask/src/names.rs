//! Identifier and type-name conventions.

use anyhow::{Result, bail};
use heck::{ToSnakeCase, ToUpperCamelCase};

/// Rust names for every Polarion resource type. Resource type strings have no
/// word boundaries (`workitems`, `testrun_comments`), so this is maintained by
/// hand; codegen fails on unknown types so new ones get a deliberate name.
const RESOURCES: &[(&str, &str)] = &[
    ("collections", "Collection"),
    ("customfields", "CustomField"),
    ("document_attachments", "DocumentAttachment"),
    ("document_comments", "DocumentComment"),
    ("document_parts", "DocumentPart"),
    ("documents", "Document"),
    ("enumerations", "Enumeration"),
    ("externallylinkedworkitems", "ExternallyLinkedWorkItem"),
    ("featureselections", "FeatureSelection"),
    ("globalroles", "GlobalRole"),
    ("icons", "Icon"),
    ("jobs", "Job"),
    ("license", "License"),
    ("license_assignments", "LicenseAssignment"),
    ("license_slots", "LicenseSlot"),
    ("linkedoslcresources", "LinkedOslcResource"),
    ("linkedworkitems", "LinkedWorkItem"),
    ("llms", "Llm"),
    ("metadata", "Metadata"),
    ("page_attachments", "PageAttachment"),
    ("page_comments", "PageComment"),
    ("pages", "Page"),
    ("plans", "Plan"),
    ("projects", "Project"),
    ("projecttemplates", "ProjectTemplate"),
    ("revisions", "Revision"),
    ("testparameter_definitions", "TestParameterDefinition"),
    ("testparameters", "TestParameter"),
    ("testrecord_attachments", "TestRecordAttachment"),
    ("testrecords", "TestRecord"),
    ("testrun_attachments", "TestRunAttachment"),
    ("testrun_comments", "TestRunComment"),
    ("testruns", "TestRun"),
    ("teststep_results", "TestStepResult"),
    ("teststepresult_attachments", "TestStepResultAttachment"),
    ("teststeps", "TestStep"),
    ("usergroups", "UserGroup"),
    ("users", "User"),
    ("workitem_approvals", "WorkItemApproval"),
    ("workitem_attachments", "WorkItemAttachment"),
    ("workitem_comments", "WorkItemComment"),
    ("workitems", "WorkItem"),
    ("workrecords", "WorkRecord"),
];

pub fn resource_name(resource_type: &str) -> Result<&'static str> {
    match RESOURCES.iter().find(|(t, _)| *t == resource_type) {
        Some((_, name)) => Ok(name),
        None => bail!(
            "unknown resource type `{resource_type}`: add it to RESOURCES in xtask/src/names.rs"
        ),
    }
}

const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield", "default",
];

/// `camelCase` / `kebab-case` / anything → valid snake_case Rust identifier.
pub fn snake(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let mut out = cleaned.to_snake_case();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if KEYWORDS.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

pub fn pascal(s: &str) -> String {
    s.to_upper_camel_case()
}

/// Naive English singular for array item type names (`hyperlinks` → `hyperlink`).
pub fn singular(s: &str) -> String {
    if let Some(stem) = s.strip_suffix("ies") {
        format!("{stem}y")
    } else if s.ends_with("ss") || s.ends_with("us") {
        format!("{s}Item")
    } else if let Some(stem) = s.strip_suffix('s') {
        stem.to_owned()
    } else {
        format!("{s}Item")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers() {
        assert_eq!(snake("getWorkItems"), "get_work_items");
        assert_eq!(snake("iconURL"), "icon_url");
        assert_eq!(snake("type"), "type_");
        assert_eq!(snake("self"), "self_");
        assert_eq!(snake("page[size]"), "page_size");
        assert_eq!(singular("hyperlinks"), "hyperlink");
        assert_eq!(singular("properties"), "property");
        assert_eq!(singular("status"), "statusItem");
    }
}
