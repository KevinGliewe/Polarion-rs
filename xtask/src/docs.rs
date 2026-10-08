//! Turn spec descriptions (plain text with some HTML) into rustdoc-safe lines.

/// Convert a description into doc-comment lines: HTML tags become line breaks or
/// disappear, markdown-significant brackets are escaped and bare URLs are
/// wrapped in `<...>` so rustdoc does not warn about them.
pub fn doc_lines(text: &str) -> Vec<String> {
    let text = strip_html(text);
    let mut lines = Vec::new();
    for raw in text.lines() {
        let line = raw
            .split_whitespace()
            .map(escape_word)
            .collect::<Vec<_>>()
            .join(" ");
        if line.is_empty() && lines.last().is_none_or(|l: &String| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

fn strip_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => {
                let tag = rest[start + 1..start + end]
                    .trim_start_matches('/')
                    .to_ascii_lowercase();
                let name = tag.split_whitespace().next().unwrap_or("");
                if matches!(
                    name,
                    "br" | "p"
                        | "li"
                        | "ul"
                        | "ol"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "div"
                ) {
                    out.push('\n');
                } else if !name.chars().all(|c| c.is_ascii_alphanumeric()) || name.is_empty() {
                    // Not a tag, e.g. "a < b": keep the text.
                    out.push_str(&rest[start..start + end + 1]);
                }
                rest = &rest[start + end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn escape_word(word: &str) -> String {
    if word.starts_with("http://") || word.starts_with("https://") {
        let trimmed = word.trim_end_matches(['.', ',', ')', ';', ':']);
        return format!("<{trimmed}>{}", &word[trimmed.len()..]);
    }
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if matches!(c, '[' | ']' | '<' | '>' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A short inline-code rendering of a JSON example, if it fits on one line.
pub fn example(value: &serde_json::Value) -> Option<String> {
    let s = match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) => value.to_string(),
        _ => return None,
    };
    if s.is_empty() || s.len() > 100 || s.contains('`') || s.contains('\n') {
        return None;
    }
    Some(format!("`{s}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes() {
        assert_eq!(
            doc_lines("<h5>About</h5>See <a href=\"x\">guide</a> at https://x.y/z. a[0] < b"),
            vec!["About", "See guide at <https://x.y/z>. a\\[0\\] \\< b"]
        );
    }
}
