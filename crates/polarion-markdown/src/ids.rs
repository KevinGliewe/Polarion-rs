//! Work item ids (`PREFIX-123`) in text: the one detector both directions use.
//!
//! An id is `KEY-NUM`: KEY an uppercase ASCII letter followed by at least one more uppercase
//! letter, digit or `_`, NUM one or more digits. It stands on its own:
//! - on the left: the start of the run, whitespace, or one of ``( [ { " ' “ ‘ « , ; : ! ?``;
//! - on the right: the end of the run, whitespace, one of ``) ] } " ' ” ’ » , ; : ! ?``, or a
//!   `.` not followed by a letter or digit.
//!
//! A run is the text of adjacent `Text`, `Escaped` and `Raw` nodes, so `MY\_PROJ-12` (three nodes
//! after parsing) is one id. An escaped hyphen (`PROJ\-12`) keeps the text from being an id; other
//! escaped characters count as written.

use std::ops::Range;

use crate::WorkItemIds;

/// The ids of a run that become links under `ids`, in order and without overlaps. `escaped` flags
/// the bytes of `text` that were backslash-escaped in the Markdown; missing entries count as not
/// escaped.
pub(crate) fn find(text: &str, escaped: &[bool], ids: &WorkItemIds) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    if *ids == WorkItemIds::Off {
        return found;
    }
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_uppercase() {
            i += 1;
            continue;
        }
        if let Some(end) = id_at(text, escaped, i, ids) {
            found.push(i..end);
            i = end;
        } else {
            // No id starts inside this word: its characters are no left boundary.
            i += 1;
            while i < bytes.len() && is_key_byte(bytes[i]) {
                i += 1;
            }
        }
    }
    found
}

/// Every id-shaped token of a run, whatever the setting: [`find`] with [`WorkItemIds::Any`].
/// Escaping a token's hyphen (its first `-`) keeps the forward direction from linking it, so the
/// reverse direction writes plain text that never becomes a link.
pub(crate) fn tokens(text: &str, escaped: &[bool]) -> Vec<Range<usize>> {
    find(text, escaped, &WorkItemIds::Any)
}

/// Whether the forward direction links exactly `text[range]` of this run under `ids`: true
/// if and only if [`find`] returns `range`.
pub(crate) fn links(text: &str, escaped: &[bool], range: Range<usize>, ids: &WorkItemIds) -> bool {
    text.is_char_boundary(range.start) && id_at(text, escaped, range.start, ids) == Some(range.end)
}

/// The end of the id starting at byte `start`, if one does and `ids` links it.
fn id_at(text: &str, escaped: &[bool], start: usize, ids: &WorkItemIds) -> Option<usize> {
    let bytes = text.as_bytes();
    if !bytes.get(start)?.is_ascii_uppercase() || !left_boundary(text[..start].chars().next_back())
    {
        return None;
    }
    let mut hyphen = start + 1;
    while hyphen < bytes.len() && is_key_byte(bytes[hyphen]) {
        hyphen += 1;
    }
    if hyphen - start < 2 || bytes.get(hyphen) != Some(&b'-') || escaped.get(hyphen) == Some(&true)
    {
        return None;
    }
    let mut end = hyphen + 1;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    (end > hyphen + 1 && right_boundary(&text[end..]) && ids.links(&text[start..hyphen]))
        .then_some(end)
}

fn is_key_byte(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'
}

fn left_boundary(prev: Option<char>) -> bool {
    prev.is_none_or(|c| c.is_whitespace() || "([{\"'“‘«,;:!?".contains(c))
}

fn right_boundary(rest: &str) -> bool {
    let mut chars = rest.chars();
    match chars.next() {
        None => true,
        Some('.') => !chars.next().is_some_and(char::is_alphanumeric),
        Some(c) => c.is_whitespace() || ")]}\"'”’»,;:!?".contains(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(text: &str) -> Vec<&str> {
        tokens(text, &[]).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn shapes() {
        assert_eq!(ids("PROJ-123"), ["PROJ-123"]);
        assert_eq!(ids("MP-42"), ["MP-42"]);
        assert_eq!(ids("MY_PROJ-12"), ["MY_PROJ-12"]);
        assert_eq!(ids("A1-1 X_-2 AB_C9-007"), ["A1-1", "X_-2", "AB_C9-007"]);
        assert_eq!(ids("UTF-8"), ["UTF-8"]);
        for none in [
            "A-1", "a-1", "Proj-1", "PROJ-", "PROJ-x", "PROJ_1", "PROJ 1", "-1", "PRÖJ-1", "1A-1",
            "_AB-1",
        ] {
            assert!(ids(none).is_empty(), "{none}");
        }
    }

    #[test]
    fn left_boundaries() {
        for prefix in [
            "", " ", "\t", "\u{a0}", "(", "[", "{", "\"", "'", "“", "‘", "«", ",", ";", ":", "!",
            "?",
        ] {
            let text = format!("{prefix}PROJ-1");
            assert_eq!(ids(&text), ["PROJ-1"], "{text:?}");
        }
        for prefix in [
            "a", "x.", "-", "_", "/", "#", "@", "$", "=", "é", ")", "*", "`", "1", "„",
        ] {
            let text = format!("{prefix}PROJ-1");
            assert!(ids(&text).is_empty(), "{text:?}");
        }
        // The whole uppercase word is the key; nothing starts inside it.
        assert_eq!(ids("XPROJ-1"), ["XPROJ-1"]);
        assert!(ids("AB-CD-12").is_empty());
        assert!(ids("PROJ-12-34").is_empty());
    }

    #[test]
    fn right_boundaries() {
        for suffix in [
            "", " ", "\n", ")", "]", "}", "\"", "'", "”", "’", "»", ",", ";", ":", "!", "?", ".",
            ". ", ".)", "..",
        ] {
            let text = format!("PROJ-1{suffix}");
            assert_eq!(ids(&text), ["PROJ-1"], "{text:?}");
        }
        for suffix in [
            "a", "A", "_", "-", "-2", ".5", ".a", ".é", "/", "#", "(", "*", "%", "é", "“",
        ] {
            let text = format!("PROJ-1{suffix}");
            assert!(ids(&text).is_empty(), "{text:?}");
        }
    }

    #[test]
    fn several_and_adjacent() {
        assert_eq!(ids("See MP-100 and PROJ-200."), ["MP-100", "PROJ-200"]);
        assert_eq!(
            ids("(PROJ-1),PROJ-2;PROJ-3"),
            ["PROJ-1", "PROJ-2", "PROJ-3"]
        );
        assert_eq!(ids("PROJ-1 PROJ-1"), ["PROJ-1", "PROJ-1"]);
        assert_eq!(ids("“PROJ-1” «MP-2»"), ["PROJ-1", "MP-2"]);
        assert_eq!(ids("PROJ-1\"MP-2"), ["PROJ-1", "MP-2"]);
        assert_eq!(find("x PROJ-12 y", &[], &WorkItemIds::Any), vec![2..9]);
    }

    #[test]
    fn an_escaped_hyphen_suppresses_the_id() {
        let text = "MY_PROJ-12";
        let mut escaped = vec![false; text.len()];
        escaped[2] = true; // `\_` keeps the id
        assert_eq!(find(text, &escaped, &WorkItemIds::Any), vec![0..10]);
        escaped[7] = true; // `\-` does not
        assert!(find(text, &escaped, &WorkItemIds::Any).is_empty());
        assert!(tokens(text, &escaped).is_empty());
        // Other escaped punctuation still bounds the id.
        let text = "(PROJ-1)";
        let escaped = [true, false, false, false, false, false, false, true];
        assert_eq!(find(text, &escaped, &WorkItemIds::Any), vec![1..7]);
        // A mask shorter than the text: the rest is not escaped.
        assert_eq!(find("a PROJ-1", &[true], &WorkItemIds::Any), vec![2..8]);
    }

    #[test]
    fn settings() {
        let text = "MP-1 OP-2 UTF-8";
        let mp = WorkItemIds::Prefixes(vec!["MP".into(), "UTF".into()]);
        assert_eq!(find(text, &[], &mp), [0..4, 10..15]);
        assert!(find(text, &[], &WorkItemIds::Off).is_empty());
        assert!(find(text, &[], &WorkItemIds::Prefixes(vec![])).is_empty());
        // A prefix only counts as a whole key.
        assert!(find("MPX-1", &[], &WorkItemIds::Prefixes(vec!["MP".into()])).is_empty());
        // Id-shaped tokens do not depend on the setting.
        assert_eq!(tokens(text, &[]), [0..4, 5..9, 10..15]);
    }

    #[test]
    fn links_asks_about_one_position() {
        let text = "a PROJ-1 b";
        assert!(links(text, &[], 2..8, &WorkItemIds::Any));
        assert!(!links(text, &[], 2..7, &WorkItemIds::Any));
        assert!(!links(text, &[], 3..8, &WorkItemIds::Any));
        assert!(!links(text, &[], 2..8, &WorkItemIds::Off));
        assert!(!links("aPROJ-1", &[], 1..7, &WorkItemIds::Any));
        assert!(!links("é", &[], 1..2, &WorkItemIds::Any));
        assert!(!links("PROJ-1", &[], 7..9, &WorkItemIds::Any));
        let mut escaped = vec![false; text.len()];
        escaped[6] = true;
        assert!(!links(text, &escaped, 2..8, &WorkItemIds::Any));
    }

    #[test]
    fn unicode_text_around_ids() {
        assert_eq!(ids("Größe: PROJ-1 – fertig"), ["PROJ-1"]);
        assert_eq!(ids("日本 PROJ-1。"), Vec::<&str>::new());
        assert_eq!(ids("日本 PROJ-1"), ["PROJ-1"]);
    }

    /// A deterministic xorshift generator.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 as usize
        }
    }

    /// `links` answers exactly what `find` does, `tokens` is `find` with any prefix, and a
    /// prefix setting only filters the tokens.
    #[test]
    fn the_queries_agree_on_random_runs() {
        const PIECES: [&str; 18] = [
            "A", "B", "Z", "_", "-", "1", "9", "a", " ", ".", "(", ")", "\"", "é", "“", ",", "\n",
            "PROJ-1",
        ];
        let settings = [
            WorkItemIds::Any,
            WorkItemIds::Off,
            WorkItemIds::Prefixes(vec!["AB".into(), "PROJ".into(), "Z_".into()]),
        ];
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        for _ in 0..3_000 {
            let len = rng.next() % 12;
            let text: String = (0..len)
                .map(|_| PIECES[rng.next() % PIECES.len()])
                .collect();
            let escaped: Vec<bool> = (0..text.len())
                .map(|_| rng.next().is_multiple_of(5))
                .collect();
            let all = tokens(&text, &escaped);
            for ids in &settings {
                let found = find(&text, &escaped, ids);
                let filtered: Vec<Range<usize>> = all
                    .iter()
                    .filter(|r| {
                        let token = &text[r.start..r.end];
                        ids.links(&token[..token.find('-').unwrap()])
                    })
                    .cloned()
                    .collect();
                assert_eq!(found, filtered, "{text:?} {ids:?}");
                for start in 0..=text.len() {
                    for end in start..=text.len() {
                        assert_eq!(
                            links(&text, &escaped, start..end, ids),
                            found.contains(&(start..end)),
                            "{text:?} {start}..{end} {ids:?}"
                        );
                    }
                }
            }
        }
    }
}
