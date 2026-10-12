//! Identifier and type-name conventions.

use heck::{ToSnakeCase, ToUpperCamelCase};

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
    }
}
