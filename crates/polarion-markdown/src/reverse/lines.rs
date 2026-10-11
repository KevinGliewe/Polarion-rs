//! Lines: a container's flow split into paragraphs, lines and headings, with the breaks between
//! blocks absorbed as the forward direction writes them (`styles::separator_breaks`).
//!
//! One `<br>` between two lines is a line break, two or more a paragraph break; a block element
//! without margins ends a line as a `<br>` does, one with margins (`p`, ...) a paragraph. Around
//! each block exactly the separator of the shared table is absorbed; extra breaks become raw
//! `<br/>` in keep mode (at the start of the next paragraph, at the end of the previous block
//! before a heading, else a raw block of breaks between boxes where the separators around it
//! leave room) and are dropped in drop mode. A single `<br>` ending a container is invisible and
//! dropped.
//! Whitespace collapses as in a browser; `&nbsp;` is kept.

use comrak::nodes::{AlertType, TableAlignment};

use super::build;
use super::css::Style;
use super::walk::{Atom, AtomKind, Block, Container, Item};
use crate::html;
use crate::styles::{self, BlockKind};

/// A block of the Markdown to build.
#[derive(Debug)]
pub(super) enum LBlock {
    /// A paragraph; `Br` atoms are its line breaks.
    Para(Vec<Atom>),
    Heading(u8, Vec<Atom>),
    /// A display formula alone in its paragraph that needs a ```` ```math ```` block.
    MathBlock(String),
    Quote(usize, Vec<LBlock>),
    Alert(AlertType, Option<String>, Vec<LBlock>),
    Code(String, String),
    Diagram(String),
    List {
        ordered: bool,
        start: usize,
        tight: bool,
        items: Vec<(Option<bool>, Vec<LBlock>)>,
    },
    Table {
        aligns: Vec<TableAlignment>,
        /// Cells as inline atoms (`RawBr` for their line breaks), the header row first.
        rows: Vec<Vec<Vec<Atom>>>,
    },
    Raw(String),
    RawWrap(String, Vec<LBlock>),
    Rule,
}

impl LBlock {
    /// The kinds the block starts and ends with in the HTML read (for the separators).
    pub(super) fn kinds(&self) -> (BlockKind, BlockKind) {
        match self {
            LBlock::Para(_) | LBlock::Heading(..) | LBlock::MathBlock(_) | LBlock::Diagram(_) => {
                (BlockKind::Text, BlockKind::Text)
            }
            LBlock::Raw(html) => html::raw_block_kinds(html),
            _ => (BlockKind::Box, BlockKind::Box),
        }
    }

    /// Whether raw `<br/>` can be added to the block. Not to an empty heading: its HTML would then
    /// hold breaks only and read back as no heading.
    fn takes_breaks(&self) -> bool {
        match self {
            LBlock::Para(_) => true,
            LBlock::Heading(_, atoms) => !atoms.is_empty(),
            _ => false,
        }
    }

    fn add_breaks(&mut self, n: usize, front: bool) {
        if let LBlock::Para(atoms) | LBlock::Heading(_, atoms) = self {
            let breaks = std::iter::repeat_with(raw_br).take(n);
            if front {
                atoms.splice(0..0, breaks);
            } else {
                atoms.extend(breaks);
            }
        }
    }
}

/// Takes a task marker (`☐`, `☑`, `☒` and a space or no-break space) off the start of a
/// paragraph's collapsed atoms.
fn take_task_marker(atoms: &mut Vec<Atom>) -> Option<bool> {
    let markers = [
        (styles::TASK_OPEN, false),
        (styles::TASK_DONE, true),
        (styles::TASK_CROSSED, true),
    ];
    let space = |c: char| c == ' ' || c == '\u{a0}';
    let AtomKind::Text(first) = &atoms.first()?.kind else {
        return None;
    };
    for (marker, done) in markers {
        let Some(rest) = first.strip_prefix(marker.trim_end()) else {
            continue;
        };
        if let Some(rest) = rest.strip_prefix(space) {
            let rest = rest.to_owned();
            if rest.is_empty() {
                atoms.remove(0);
            } else {
                atoms[0].kind = AtomKind::Text(rest);
            }
            return Some(done);
        }
        if rest.is_empty()
            && let Some(Atom {
                kind: AtomKind::Text(next),
                ..
            }) = atoms.get(1)
            && let Some(after) = next.strip_prefix(space)
        {
            let after = after.to_owned();
            atoms.remove(0);
            if after.is_empty() {
                atoms.remove(0);
            } else {
                atoms[0].kind = AtomKind::Text(after);
            }
            return Some(done);
        }
        return None;
    }
    None
}

fn raw_br() -> Atom {
    Atom {
        kind: AtomKind::RawBr,
        style: Style::default(),
    }
}

/// What a line is.
enum Line {
    Blank,
    Content,
    Heading(u8),
}

/// The blocks of a container.
pub(super) fn blocks(container: Container, keep: bool) -> Vec<LBlock> {
    let in_li = container.in_li;
    // The blocks with the number of breaks before each.
    let mut out: Vec<(LBlock, usize)> = Vec::new();
    let mut para: Option<(Vec<Atom>, usize)> = None;
    let mut line: Vec<Atom> = Vec::new();
    let mut gap = 0usize;
    let mut gap_style: Option<Style> = None;

    fn flush(para: &mut Option<(Vec<Atom>, usize)>, out: &mut Vec<(LBlock, usize)>) {
        if let Some((atoms, gap)) = para.take() {
            out.push((LBlock::Para(atoms), gap));
        }
    }

    let end_line = |line: &mut Vec<Atom>,
                    para: &mut Option<(Vec<Atom>, usize)>,
                    out: &mut Vec<(LBlock, usize)>,
                    gap: &mut usize,
                    gap_style: &mut Option<Style>| {
        let atoms = std::mem::take(line);
        match classify(&atoms) {
            Line::Blank => {}
            Line::Heading(level) => {
                flush(para, out);
                out.push((LBlock::Heading(level, collapse(atoms)), *gap));
                *gap = 0;
                *gap_style = None;
            }
            Line::Content => {
                let atoms = collapse(atoms);
                match para {
                    Some((current, _)) if *gap == 1 => {
                        current.push(Atom {
                            kind: AtomKind::Br,
                            style: gap_style.take().unwrap_or_default(),
                        });
                        current.extend(atoms);
                    }
                    _ => {
                        flush(para, out);
                        *para = Some((atoms, *gap));
                    }
                }
                *gap = 0;
                *gap_style = None;
            }
        }
    };

    for item in container.items {
        match item {
            Item::Atom(atom) if matches!(atom.kind, AtomKind::Br) => {
                end_line(&mut line, &mut para, &mut out, &mut gap, &mut gap_style);
                gap += 1;
                if gap_style.is_none() {
                    gap_style = Some(atom.style);
                }
            }
            Item::Atom(atom) => line.push(atom),
            Item::Boundary => {
                end_line(&mut line, &mut para, &mut out, &mut gap, &mut gap_style);
                flush(&mut para, &mut out);
            }
            // A line with content ends here, as at a `<br>`.
            Item::LineEnd => {
                if !matches!(classify(&line), Line::Blank) {
                    end_line(&mut line, &mut para, &mut out, &mut gap, &mut gap_style);
                    gap += 1;
                }
            }
            Item::Block(block) => {
                end_line(&mut line, &mut para, &mut out, &mut gap, &mut gap_style);
                flush(&mut para, &mut out);
                out.push((convert(block, keep), gap));
                gap = 0;
                gap_style = None;
            }
        }
    }
    end_line(&mut line, &mut para, &mut out, &mut gap, &mut gap_style);
    flush(&mut para, &mut out);
    let trail = gap;

    // A display formula alone in its paragraph that `$$…$$` cannot hold.
    for (block, _) in &mut out {
        if let LBlock::Para(atoms) = block
            && let [
                Atom {
                    kind:
                        AtomKind::Math {
                            source,
                            display: true,
                            ..
                        },
                    ..
                },
            ] = atoms.as_slice()
            && (source.contains('\n') || !build::display_math(source))
        {
            *block = LBlock::MathBlock(source.clone());
        }
    }

    // Breaks beyond the separators.
    let mut blocks: Vec<LBlock> = Vec::with_capacity(out.len() + 1);
    let mut gaps = Vec::with_capacity(out.len());
    for (block, gap) in out {
        blocks.push(block);
        gaps.push(gap);
    }
    if keep {
        // Raw breaks of their own, before the block at each index.
        let mut inserted: Vec<(usize, usize)> = Vec::new();
        for i in 0..blocks.len() {
            let expected = match i {
                0 => 0,
                _ => styles::separator_breaks(blocks[i - 1].kinds().1, blocks[i].kinds().0, in_li),
            };
            let extra = gaps[i].saturating_sub(expected);
            if extra == 0 {
                continue;
            }
            let heading = matches!(blocks[i], LBlock::Heading(..));
            // Not into a heading where the block before can take them: there they would show at
            // the heading's size, at the end of the block before as the HTML had them.
            if i > 0 && heading && blocks[i - 1].takes_breaks() {
                blocks[i - 1].add_breaks(extra, false);
            } else if blocks[i].takes_breaks() {
                blocks[i].add_breaks(extra, true);
            } else if i > 0 && blocks[i - 1].takes_breaks() {
                blocks[i - 1].add_breaks(extra, false);
            } else {
                // Between boxes, or before one at the start: a raw block of breaks, where the
                // separators around it leave room for at least one.
                let before = match i {
                    0 => 0,
                    _ => styles::separator_breaks(blocks[i - 1].kinds().1, BlockKind::Text, in_li),
                };
                let after = styles::separator_breaks(BlockKind::Box, blocks[i].kinds().0, in_li);
                let own = gaps[i].saturating_sub(before + after);
                if own > 0 {
                    inserted.push((i, own));
                }
            }
        }
        for (i, own) in inserted.into_iter().rev() {
            blocks.insert(i, LBlock::Raw(vec![styles::BR; own].join("\n")));
        }
        // At the end: after text a single break is invisible; after a box the breaks beyond
        // the separator to a paragraph become one of raw breaks.
        match blocks.last_mut() {
            None if trail >= 2 => blocks.push(LBlock::Para(
                std::iter::repeat_with(raw_br).take(trail).collect(),
            )),
            Some(last) if last.takes_breaks() => {
                if trail >= 2 {
                    last.add_breaks(trail, false);
                }
            }
            Some(last) => {
                let separator = styles::separator_breaks(last.kinds().1, BlockKind::Text, in_li);
                let extra = trail.saturating_sub(separator);
                if extra > 0 {
                    blocks.push(LBlock::Para(
                        std::iter::repeat_with(raw_br).take(extra).collect(),
                    ));
                }
            }
            None => {}
        }
    }
    blocks
}

fn convert(block: Block, keep: bool) -> LBlock {
    match block {
        Block::Quote(depth, c) => LBlock::Quote(depth, blocks(c, keep)),
        Block::Alert(kind, title, c) => LBlock::Alert(kind, title, blocks(c, keep)),
        Block::Code(lang, text) => LBlock::Code(lang, text),
        Block::Diagram(source) => LBlock::Diagram(source),
        Block::List {
            ordered,
            start,
            items,
        } => {
            let items: Vec<(Option<bool>, Vec<LBlock>)> = items
                .into_iter()
                .map(|(task, c)| {
                    let mut blocks = blocks(c, keep);
                    // A marker the walk did not see (its space came from collapsed whitespace
                    // or a later element) reads back as one all the same.
                    let task = match (task, blocks.first_mut()) {
                        (None, Some(LBlock::Para(atoms))) => take_task_marker(atoms),
                        (task, _) => task,
                    };
                    match (task, blocks.first_mut()) {
                        // Markdown's task marker takes the item's first paragraph: before other
                        // blocks the marker stays text, in a heading its first text.
                        (Some(done), Some(first)) if !matches!(first, LBlock::Para(_)) => {
                            let marker = if done {
                                styles::TASK_DONE
                            } else {
                                styles::TASK_OPEN
                            };
                            match first {
                                // Not before breaks: the heading would end at them.
                                LBlock::Heading(_, atoms)
                                    if atoms.first().is_some_and(|a| {
                                        !matches!(a.kind, AtomKind::RawBr | AtomKind::Br)
                                    }) =>
                                {
                                    let text = Atom {
                                        kind: AtomKind::Text(marker.to_owned()),
                                        style: atoms[0].style.clone(),
                                    };
                                    atoms.insert(0, text);
                                }
                                _ => {
                                    let text = Atom {
                                        kind: AtomKind::Text(marker.trim_end().to_owned()),
                                        style: Style::default(),
                                    };
                                    let mut para = vec![text];
                                    // Breaks kept before the first block (a block of their
                                    // own, the start of a heading) now end the marker's
                                    // paragraph, as they would on reading it back.
                                    match &mut blocks[0] {
                                        LBlock::Raw(html)
                                            if html.split('\n').all(|line| line == styles::BR) =>
                                        {
                                            let breaks = html.split('\n').count();
                                            para.extend(
                                                std::iter::repeat_with(raw_br).take(breaks),
                                            );
                                            blocks.remove(0);
                                        }
                                        LBlock::Heading(_, atoms) => {
                                            let breaks = atoms
                                                .iter()
                                                .take_while(|a| matches!(a.kind, AtomKind::RawBr))
                                                .count();
                                            para.extend(atoms.drain(..breaks));
                                        }
                                        _ => {}
                                    }
                                    blocks.insert(0, LBlock::Para(para));
                                }
                            }
                            (None, blocks)
                        }
                        _ => (task, blocks),
                    }
                })
                .collect();
            // Tight only when no item holds blocks a tight list would run together: a paragraph
            // and then lists. Right after the paragraph a list must be able to interrupt it (not
            // start with an empty item or a number other than 1).
            let tight = items.iter().all(|(_, blocks)| {
                let (rest, after_paragraph) = match blocks.first() {
                    // A lone tag starts an HTML block, which would run on into the next block.
                    Some(LBlock::Para(atoms)) if lone_tag(atoms) && blocks.len() > 1 => {
                        return false;
                    }
                    Some(LBlock::Para(_)) => (&blocks[1..], true),
                    _ => (&blocks[..], false),
                };
                rest.iter().all(|b| matches!(b, LBlock::List { .. }))
                    && !(after_paragraph && rest.first().is_some_and(|b| !interrupts(b)))
            });
            LBlock::List {
                ordered,
                start,
                tight,
                items,
            }
        }
        Block::Table { aligns, rows } => LBlock::Table {
            aligns,
            rows: rows
                .into_iter()
                .map(|row| row.into_iter().map(cell).collect())
                .collect(),
        },
        Block::Raw(html) => LBlock::Raw(html),
        Block::RawWrap(open, c) => LBlock::RawWrap(open, blocks(c, keep)),
        Block::Rule => LBlock::Rule,
    }
}

/// Whether a paragraph is a single HTML tag (a raw image, a raw break): as a line of its own it
/// starts an HTML block.
fn lone_tag(atoms: &[Atom]) -> bool {
    match atoms {
        [
            Atom {
                kind: AtomKind::RawBr,
                ..
            },
        ] => true,
        [
            Atom {
                kind: AtomKind::Html(html),
                ..
            },
        ] => !html.contains("</"),
        _ => false,
    }
}

/// Whether a list can interrupt a paragraph: its first item is not empty and, ordered, it starts
/// at 1.
fn interrupts(list: &LBlock) -> bool {
    match list {
        LBlock::List {
            ordered,
            start,
            items,
            ..
        } => {
            (!*ordered || *start == 1)
                && items
                    .first()
                    .is_some_and(|(task, blocks)| task.is_some() || !blocks.is_empty())
        }
        _ => true,
    }
}

/// A table cell: one line, every `<br>` kept as raw `<br/>`.
fn cell(container: Container) -> Vec<Atom> {
    let mut out = Vec::new();
    let mut line = Vec::new();
    for item in container.items {
        match item {
            Item::Atom(atom) if matches!(atom.kind, AtomKind::Br) => {
                out.extend(collapse(std::mem::take(&mut line)));
                out.push(raw_br());
            }
            Item::Atom(atom) => line.push(atom),
            // Cells hold inline content only.
            Item::Block(_) | Item::Boundary | Item::LineEnd => {}
        }
    }
    out.extend(collapse(line));
    out
}

fn is_content(atom: &Atom) -> bool {
    match &atom.kind {
        AtomKind::Text(t) => t.chars().any(|c| !c.is_ascii_whitespace()),
        AtomKind::Br | AtomKind::RawBr | AtomKind::Empty => false,
        AtomKind::Image { .. }
        | AtomKind::Html(_)
        | AtomKind::Id { .. }
        | AtomKind::Math { .. } => true,
    }
}

/// Whether text in this style is heading text: at least 14pt, or more than 10pt and bold.
fn heading_size(style: &Style) -> Option<f32> {
    style
        .size
        .filter(|&size| size >= 14.0 || (size > 10.0 && style.bold))
}

fn classify(atoms: &[Atom]) -> Line {
    let mut content = false;
    let mut smallest: Option<f32> = None;
    for atom in atoms.iter().filter(|a| is_content(a)) {
        let visible = match &atom.kind {
            AtomKind::Text(t) => t.chars().any(|c| !c.is_whitespace()),
            _ => true,
        };
        content = true;
        if !visible {
            continue;
        }
        let Some(size) = heading_size(&atom.style) else {
            return Line::Content;
        };
        smallest = Some(smallest.map_or(size, |s: f32| s.min(size)));
    }
    if content && smallest.is_none() {
        // Only no-break spaces: a heading if they are heading text.
        smallest = atoms
            .iter()
            .filter(|a| is_content(a))
            .map(|a| heading_size(&a.style))
            .try_fold(f32::MAX, |acc, s| s.map(|s| acc.min(s)));
    }
    if !content {
        // No content: an empty element or whitespace in heading style is an empty heading.
        smallest = atoms
            .iter()
            .filter(|a| matches!(a.kind, AtomKind::Text(_) | AtomKind::Empty))
            .filter_map(|a| heading_size(&a.style))
            .reduce(f32::min);
    }
    match smallest.and_then(styles::heading_level) {
        Some(level) => Line::Heading(level),
        None if content => Line::Content,
        None => Line::Blank,
    }
}

/// A line's atoms with whitespace collapsed as a browser does: runs of ASCII whitespace as one
/// space, none at the line's start or end; empty elements removed.
pub(super) fn collapse(atoms: Vec<Atom>) -> Vec<Atom> {
    let mut out: Vec<Atom> = Vec::with_capacity(atoms.len());
    let mut space = true;
    for atom in atoms {
        match atom.kind {
            AtomKind::Text(text) => {
                let mut collapsed = String::with_capacity(text.len());
                for c in text.chars() {
                    if c.is_ascii_whitespace() {
                        if !space {
                            collapsed.push(' ');
                            space = true;
                        }
                    } else {
                        collapsed.push(c);
                        space = false;
                    }
                }
                if !collapsed.is_empty() {
                    out.push(Atom {
                        kind: AtomKind::Text(collapsed),
                        style: atom.style,
                    });
                }
            }
            AtomKind::Empty => {}
            AtomKind::RawBr | AtomKind::Br => {
                out.push(atom);
                space = true;
            }
            _ => {
                out.push(atom);
                space = false;
            }
        }
    }
    // Spaces at the end of the line.
    while let Some(last) = out.last_mut() {
        let AtomKind::Text(text) = &mut last.kind else {
            break;
        };
        let trimmed = text.trim_end_matches(' ').len();
        if trimmed > 0 {
            text.truncate(trimmed);
            break;
        }
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reverse::walk::walk;
    use crate::{DiagramSources, RawHtml, ToMarkdownOptions};

    fn lblocks(html: &str, keep: bool) -> Vec<LBlock> {
        let options = ToMarkdownOptions {
            raw_html: if keep { RawHtml::Keep } else { RawHtml::Drop },
            ..ToMarkdownOptions::default()
        };
        blocks(walk(html, &options, &DiagramSources::new()).body, keep)
    }

    fn shape(blocks: &[LBlock]) -> Vec<String> {
        blocks
            .iter()
            .map(|b| match b {
                LBlock::Para(atoms) | LBlock::Heading(_, atoms) => {
                    let mut s = match b {
                        LBlock::Heading(level, _) => format!("h{level}:"),
                        _ => "p:".to_owned(),
                    };
                    for a in atoms {
                        match &a.kind {
                            AtomKind::Text(t) => s.push_str(t),
                            AtomKind::Br => s.push('|'),
                            AtomKind::RawBr => s.push_str("<br>"),
                            _ => s.push('*'),
                        }
                    }
                    s
                }
                LBlock::Quote(depth, inner) => format!("q{depth}{:?}", shape(inner)),
                other => format!("{other:?}").chars().take(12).collect(),
            })
            .collect()
    }

    #[test]
    fn breaks_split_lines_and_paragraphs() {
        assert_eq!(
            shape(&lblocks("a<br>b<br/><br/>c<br><br><br>d", true)),
            ["p:a|b", "p:c", "p:<br>d"]
        );
        assert_eq!(
            shape(&lblocks("a<br>b<br/><br/>c<br><br><br>d", false)),
            ["p:a|b", "p:c", "p:d"]
        );
        // Whitespace-only lines are no lines; a single break at the end is invisible.
        assert_eq!(shape(&lblocks("a<br/> <br/>b<br/>", true)), ["p:a", "p:b"]);
        assert_eq!(shape(&lblocks("a<br/><br/>", true)), ["p:a<br><br>"]);
    }

    #[test]
    fn separators_around_boxes_are_absorbed() {
        assert_eq!(
            shape(&lblocks(
                "a<br/><br/><div style=\"margin-left: 40px;\">q<br/> </div> <br/>b",
                true
            )),
            ["p:a", "q1[\"p:q\"]", "p:b"]
        );
        // One break more than the separator is kept, in keep mode.
        assert_eq!(
            shape(&lblocks("a<br/><br/><br/><hr/><br/><br/>b", true)),
            ["p:a<br>", "Rule", "p:<br>b"]
        );
    }

    #[test]
    fn whitespace_collapses() {
        assert_eq!(
            shape(&lblocks("  a \n <b> b</b>\u{a0} c  ", true)),
            ["p:ab\u{a0} c"].map(|s| s.replace("ab", "a b"))
        );
    }

    #[test]
    fn headings_by_size() {
        let h = |html: &str| shape(&lblocks(html, true));
        assert_eq!(
            h("<span style=\"font-weight: bold; font-size: 22pt;\">T</span>"),
            ["h1:T"]
        );
        assert_eq!(h("<span style=\"font-size: 18pt;\">T</span>"), ["h2:T"]);
        assert_eq!(h("<span style=\"font-size: 12pt;\">T</span>"), ["p:T"]);
        assert_eq!(
            h("<span style=\"font-weight: bold; font-size: 22pt;\"></span>"),
            ["h1:"]
        );
        assert_eq!(
            h("<span style=\"font-weight: bold; font-size: 22pt;\"> </span>"),
            ["h1:"]
        );
        assert_eq!(
            h("a<br/><span style=\"font-size: 14pt;\">T</span><br/>b"),
            ["p:a", "h3:T", "p:b"]
        );
        assert_eq!(
            h("Text <span style=\"font-size: 12pt;\">12</span>"),
            ["p:Text 12"]
        );
    }
}
