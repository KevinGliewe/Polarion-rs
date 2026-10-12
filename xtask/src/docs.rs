//! Turn WSDL documentation (plain text with some HTML) into rustdoc-safe text.

pub fn strip_html(text: &str) -> String {
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

pub fn escape_word(word: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes() {
        assert_eq!(
            strip_html("<h5>About</h5>See <a href=\"x\">guide</a>, a < b &amp; c"),
            "\nAbout\nSee guide, a < b & c"
        );
        assert_eq!(escape_word("https://x.y/z."), "<https://x.y/z>.");
        assert_eq!(escape_word("a[0]<b"), r"a\[0\]\<b");
    }
}
