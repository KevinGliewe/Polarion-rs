//! Output: JSON, or text for people (column tables, an indented key/value tree, text as it is).

use std::fmt::Write as _;

use polarion_commands::View;
use serde_json::Value;

fn str_at(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// JSON for programs: pretty on a terminal, one line otherwise.
pub fn json(value: &Value, pretty: bool) -> String {
    let text = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    };
    text.expect("a serde_json::Value always serializes")
}

/// Text for people. Empty for an empty list or no result.
pub fn text(value: &Value, view: View) -> String {
    match view {
        View::Table(columns) => {
            let rows: Vec<Vec<String>> = items(value)
                .map(|item| columns.iter().map(|c| one_line(&(c.cell)(item))).collect())
                .collect();
            if rows.is_empty() {
                return String::new();
            }
            table(&columns.iter().map(|c| c.header).collect::<Vec<_>>(), &rows)
        }
        View::Object => {
            let mut out = String::new();
            tree(&mut out, 0, value);
            out
        }
        View::Lines => items(value).map(|v| format!("{}\n", inline(v))).collect(),
        View::Links => {
            let mut rows = Vec::new();
            for (key, direction) in [("outgoing", "out"), ("incoming", "in")] {
                for link in value.get(key).into_iter().flat_map(items) {
                    rows.push(vec![
                        direction.to_owned(),
                        str_at(link, "role"),
                        link.get("item").map(inline).unwrap_or_default(),
                        if link.get("suspect") == Some(&Value::Bool(true)) {
                            "suspect".to_owned()
                        } else {
                            String::new()
                        },
                    ]);
                }
            }
            if rows.is_empty() {
                return String::new();
            }
            table(&["DIRECTION", "ROLE", "ITEM", ""], &rows)
        }
        View::Scalar => match value {
            Value::Null => String::new(),
            other => format!("{}\n", inline(other)),
        },
        // As it is (`> file` keeps it), ending with a line break.
        View::Text => match value {
            Value::Null => String::new(),
            Value::String(s) if s.is_empty() || s.ends_with('\n') => s.clone(),
            Value::String(s) => format!("{s}\n"),
            other => format!("{other}\n"),
        },
        View::Bytes => format!(
            "{} bytes (use --out FILE to save them)\n",
            value.get("size").and_then(Value::as_u64).unwrap_or(0)
        ),
        View::Done => match value.get("changed") {
            Some(Value::Bool(true)) => "changed\n".into(),
            Some(Value::Bool(false)) => "unchanged\n".into(),
            _ => "done\n".into(),
        },
    }
}

/// The command list for `polarion commands`.
pub fn commands(list: &[(String, bool, String)]) -> String {
    let rows: Vec<Vec<String>> = list
        .iter()
        .map(|(name, write, summary)| {
            vec![
                name.clone(),
                if *write { "write" } else { "" }.to_owned(),
                summary.clone(),
            ]
        })
        .collect();
    table(&["COMMAND", "", "SUMMARY"], &rows)
}

fn items(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

/// Aligned columns; the last column is not padded.
fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    let header_row: Vec<String> = headers.iter().map(|h| (*h).to_owned()).collect();
    for row in std::iter::once(&header_row).chain(rows) {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            if i + 1 < row.len() {
                let pad = widths[i] - cell.chars().count();
                let _ = write!(line, "{cell}{}  ", " ".repeat(pad));
            } else {
                line.push_str(cell);
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// Indented `key: value` lines.
fn tree(out: &mut String, indent: usize, value: &Value) {
    let pad = " ".repeat(indent);
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if let Some(text) = rich_text(child) {
                    let mut lines = text.lines();
                    let first = lines.next().unwrap_or_default();
                    let _ = writeln!(out, "{pad}{key}: {first}");
                    for line in lines {
                        let _ = writeln!(out, "{pad}  {line}");
                    }
                } else if is_inline(child) {
                    let _ = writeln!(out, "{pad}{key}: {}", inline(child));
                } else {
                    let _ = writeln!(out, "{pad}{key}:");
                    tree(out, indent + 2, child);
                }
            }
        }
        Value::Array(list) => {
            for item in list {
                if is_inline(item) {
                    let _ = writeln!(out, "{pad}- {}", inline(item));
                } else {
                    let _ = writeln!(out, "{pad}-");
                    tree(out, indent + 2, item);
                }
            }
        }
        other => {
            let _ = writeln!(out, "{pad}{}", inline(other));
        }
    }
}

/// Whether a value fits on its key's line. A list whose texts contain commas does not: joined
/// with `, ` its items could not be told apart (sentences such as warnings).
fn is_inline(value: &Value) -> bool {
    match value {
        Value::Array(list) => list
            .iter()
            .all(|v| !v.is_array() && is_inline(v) && !v.as_str().is_some_and(|s| s.contains(','))),
        Value::Object(_) => reference(value).is_some(),
        _ => true,
    }
}

/// Scalars as text, scalar lists comma-separated, references as `PROJECT/ID`.
fn inline(value: &Value) -> String {
    match value {
        Value::String(s) => one_line(s),
        Value::Null => String::new(),
        Value::Array(list) => list.iter().map(inline).collect::<Vec<_>>().join(", "),
        Value::Object(_) => reference(value).unwrap_or_else(|| value.to_string()),
        other => other.to_string(),
    }
}

/// `{"project", "id"}` and `{"project", "space", "name"}` objects (work item and document references).
fn reference(value: &Value) -> Option<String> {
    let map = value.as_object()?;
    let parts: Option<Vec<&str>> = match map.len() {
        1 => Some(vec![map.get("uri")?.as_str()?]),
        2 => ["project", "id"]
            .iter()
            .map(|k| map.get(*k)?.as_str())
            .collect(),
        3 => ["project", "space", "name"]
            .iter()
            .map(|k| map.get(*k)?.as_str())
            .collect(),
        _ => None,
    };
    parts.map(|p| p.join("/"))
}

/// Rich text (`{"type": "text/…", "content": …}`) as plain text.
fn rich_text(value: &Value) -> Option<String> {
    let map = value.as_object()?;
    if map.len() != 2 {
        return None;
    }
    let kind = map.get("type")?.as_str()?;
    let content = map.get("content")?.as_str()?;
    if !kind.starts_with("text/") {
        return None;
    }
    Some(if kind == "text/html" {
        strip_html(content)
    } else {
        content.trim().to_owned()
    })
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// HTML to readable text: block ends become line breaks, tags are dropped, common entities decoded.
pub fn strip_html(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            rest = &rest[start..];
            break;
        };
        let tag = rest[start + 1..start + end].trim().to_ascii_lowercase();
        let name = tag.trim_start_matches('/');
        let name = name
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default();
        if matches!(
            name,
            "br" | "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        ) {
            out.push('\n');
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    decoded
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tables_are_aligned() {
        let items = json!([
            {"id": "P-1", "type": "requirement", "status": "open", "title": "Brakes"},
            {"id": "P-10", "type": "task", "title": "Multi\nline"},
        ]);
        assert_eq!(
            text(&items, View::Table(polarion_commands::ITEM_COLUMNS)),
            "ID    TYPE         STATUS  TITLE\n\
             P-1   requirement  open    Brakes\n\
             P-10  task                 Multi line\n"
        );
        assert_eq!(
            text(&json!([]), View::Table(polarion_commands::ITEM_COLUMNS)),
            ""
        );
    }

    #[test]
    fn objects_render_as_a_tree() {
        let item = json!({
            "id": "P-1",
            "description": {"type": "text/html", "content": "<p>First &amp; foremost</p><p>Second</p>"},
            "assignees": ["amy", "bob"],
            "document": {"project": "P", "space": "Specs", "name": "SRS"},
            "links": [{"role": "parent", "item": {"project": "P", "id": "P-0"}}],
            "customFields": {"asil": "D", "count": 3},
        });
        assert_eq!(
            text(&item, View::Object),
            "id: P-1\n\
             description: First & foremost\n  Second\n\
             assignees: amy, bob\n\
             document: P/Specs/SRS\n\
             links:\n  -\n    role: parent\n    item: P/P-0\n\
             customFields:\n  asil: D\n  count: 3\n"
        );
    }

    #[test]
    fn links_render_in_both_directions() {
        let links = json!({
            "outgoing": [{"role": "parent", "item": {"project": "P", "id": "P-0"}, "suspect": true}],
            "incoming": [{"role": "verifies", "item": {"uri": "x"}}],
        });
        assert_eq!(
            text(&links, View::Links),
            "DIRECTION  ROLE      ITEM   \n".trim_end().to_owned()
                + "\n"
                + "out        parent    P/P-0  suspect\n"
                + "in         verifies  x\n"
        );
    }

    #[test]
    fn lists_of_sentences_get_a_line_each() {
        let update = json!({
            "updated": true,
            "deleted": ["2-mermaid-0123456789abcdef.svg", "3-mermaid-fedcba9876543210.svg"],
            "warnings": [
                "mermaid diagram (a, b) could not be rendered: bad, really",
                "the diagram attachment 4-x.svg is no longer shown but could not be deleted: locked"
            ],
        });
        assert_eq!(
            text(&update, View::Object),
            "updated: true\n\
             deleted: 2-mermaid-0123456789abcdef.svg, 3-mermaid-fedcba9876543210.svg\n\
             warnings:\n  \
             - mermaid diagram (a, b) could not be rendered: bad, really\n  \
             - the diagram attachment 4-x.svg is no longer shown but could not be deleted: locked\n"
        );
    }

    #[test]
    fn text_is_printed_as_it_is() {
        let markdown = "# Brakes\n\n- stop\n-  stop  again\n\n```mermaid\ngraph TD\n```";
        assert_eq!(text(&json!(markdown), View::Text), format!("{markdown}\n"));
        // One line break at the end, not two.
        assert_eq!(text(&json!("a\n\nb\n"), View::Text), "a\n\nb\n");
        assert_eq!(text(&json!(""), View::Text), "");
        assert_eq!(text(&Value::Null, View::Text), "");
        // A scalar is one line.
        assert_eq!(text(&json!("a\n\nb"), View::Scalar), "a b\n");
    }

    #[test]
    fn html_is_stripped() {
        assert_eq!(
            strip_html("<h1>Title</h1><p>a<br/>b &lt;c&gt;</p><div><span>d</span></div>"),
            "Title\na\nb <c>\nd"
        );
        assert_eq!(strip_html("no tags"), "no tags");
        assert_eq!(strip_html("broken <tag"), "broken <tag");
    }
}
