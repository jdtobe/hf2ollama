//! Markdown model cards as a small, safe block tree. Pure: no egui, no
//! network. Raw HTML never survives, images are never loaded, and only
//! absolute http(s) links are kept.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// Largest input parsed, matching what the core fetch already enforces.
pub const MAX_INPUT_BYTES: usize = 256 * 1024;
/// Container (list and quote) nesting beyond this is flattened.
pub const MAX_DEPTH: usize = 8;
/// Leaf blocks kept; the rest is cut and reported as truncated.
pub const MAX_BLOCKS: usize = 400;
/// Spans kept in one paragraph or heading; the rest is dropped.
pub const MAX_SPANS_PER_BLOCK: usize = 2000;
/// Table columns and rows kept; the rest is dropped.
pub const MAX_TABLE_COLS: usize = 20;
pub const MAX_TABLE_ROWS: usize = 200;
/// Spans plus table cells kept in a whole document.
pub const MAX_DOC_ITEMS: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    pub text: String,
    pub strong: bool,
    pub emph: bool,
    pub code: bool,
    /// Set only for links that passed `is_safe_link`.
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum Block {
    Heading(u8, Vec<Span>),
    Paragraph(Vec<Span>),
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    CodeBlock(String),
    /// Rows of cells of spans; the first row is the header.
    Table(Vec<Vec<Vec<Span>>>),
    Rule,
}

/// How much of a card the collapsed view shows.
#[derive(Debug, Clone, Copy)]
pub struct PreviewLimits {
    pub lines: usize,
    pub chars: usize,
}

pub const COLLAPSED: PreviewLimits = PreviewLimits {
    lines: 12,
    chars: 900,
};

/// Longest URL accepted as a link.
const MAX_URL_LEN: usize = 2048;

/// Only absolute http and https URLs may be opened.
pub fn is_safe_link(url: &str) -> bool {
    if url.len() > MAX_URL_LEN || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    else {
        return false;
    };
    rest.chars()
        .next()
        .is_some_and(|c| c != '/' && c != '?' && c != '#')
}

/// Parses Markdown into blocks; see `parse_capped` for the cut flag.
#[cfg(test)]
pub fn parse(src: &str) -> Vec<Block> {
    parse_capped(src).0
}

/// Like `parse`, also telling whether the block cap cut the document.
pub fn parse_capped(src: &str) -> (Vec<Block>, bool) {
    let mut end = src.len().min(MAX_INPUT_BYTES);
    while !src.is_char_boundary(end) {
        end -= 1;
    }
    let mut b = Builder::default();
    b.frames.push(Frame::Root(Vec::new()));
    let parser = Parser::new_ext(&src[..end], Options::ENABLE_TABLES);
    for ev in parser {
        b.event(ev);
        if b.truncated {
            break;
        }
    }
    b.finish()
}

enum Frame {
    Root(Vec<Block>),
    Quote(Vec<Block>),
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Item(Vec<Block>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Inline {
    Paragraph,
    Heading(u8),
    Cell,
}

#[derive(Default)]
struct Builder {
    frames: Vec<Frame>,
    /// One entry per open quote, list or item: true when it was flattened
    /// because of the depth cap.
    flattened: Vec<bool>,
    /// Open quotes and lists that were kept (not flattened).
    depth: usize,
    spans: Vec<Span>,
    inline: Option<Inline>,
    strong: u32,
    emph: u32,
    link: Option<String>,
    image_alt: Option<String>,
    code: Option<String>,
    html: Option<String>,
    in_comment: bool,
    table: Option<Vec<Vec<Vec<Span>>>>,
    row: Vec<Vec<Span>>,
    leaves: usize,
    /// Spans and table cells kept so far.
    items: usize,
    /// Something was dropped without stopping the parse.
    overflow: bool,
    truncated: bool,
}

impl Builder {
    fn push_block(&mut self, block: Block) {
        if self.leaves >= MAX_BLOCKS {
            self.truncated = true;
            return;
        }
        self.leaves += 1;
        if let Some(Frame::Root(v) | Frame::Quote(v) | Frame::Item(v)) = self.frames.last_mut() {
            v.push(block);
        }
    }

    /// True when one more span or cell may be kept. A full paragraph only
    /// drops its remainder; a full document ends the parse.
    fn room(&mut self) -> bool {
        if self.spans.len() >= MAX_SPANS_PER_BLOCK {
            self.overflow = true;
            return false;
        }
        if self.items >= MAX_DOC_ITEMS {
            self.truncated = true;
            return false;
        }
        self.items += 1;
        true
    }

    fn add_span(&mut self, text: String) {
        if text.is_empty() || !self.room() {
            return;
        }
        self.spans.push(Span {
            text,
            strong: self.strong > 0,
            emph: self.emph > 0,
            code: false,
            link: self.link.clone(),
        });
    }

    /// Text of a tight list item that no paragraph tag wrapped.
    fn flush_implicit(&mut self) {
        if self.inline.is_none() && !self.spans.is_empty() {
            let spans = std::mem::take(&mut self.spans);
            self.push_block(Block::Paragraph(spans));
        }
    }

    fn open_container(&mut self, frame: Frame) {
        self.flush_implicit();
        let parent_flat = self.flattened.last().copied().unwrap_or(false);
        let is_item = matches!(frame, Frame::Item(_));
        let flat = if is_item {
            parent_flat
        } else {
            parent_flat || self.depth >= MAX_DEPTH
        };
        self.flattened.push(flat);
        if !flat {
            if !is_item {
                self.depth += 1;
            }
            self.frames.push(frame);
        }
    }

    /// Closes the innermost container, returning its frame when kept.
    fn close_container(&mut self, is_item: bool) {
        self.flush_implicit();
        let flat = self.flattened.pop().unwrap_or(false);
        if flat {
            return;
        }
        if !is_item {
            self.depth = self.depth.saturating_sub(1);
        }
        self.merge_top();
    }

    /// Folds the top frame into its parent.
    fn merge_top(&mut self) {
        if self.frames.len() < 2 {
            return;
        }
        let Some(top) = self.frames.pop() else {
            return;
        };
        match top {
            Frame::Item(blocks) => {
                if let Some(Frame::List { items, .. }) = self.frames.last_mut() {
                    items.push(blocks);
                }
            }
            Frame::List { ordered, items } => {
                if !items.is_empty() {
                    self.attach(Block::List { ordered, items });
                }
            }
            Frame::Quote(blocks) => {
                if !blocks.is_empty() {
                    self.attach(Block::Quote(blocks));
                }
            }
            Frame::Root(_) => {}
        }
    }

    /// Adds a finished container to its parent without counting it.
    fn attach(&mut self, block: Block) {
        if let Some(Frame::Root(v) | Frame::Quote(v) | Frame::Item(v)) = self.frames.last_mut() {
            v.push(block);
        }
    }

    fn end_inline(&mut self) {
        let spans = std::mem::take(&mut self.spans);
        match self.inline.take() {
            Some(Inline::Paragraph) if !spans.is_empty() => {
                self.push_block(Block::Paragraph(spans));
            }
            Some(Inline::Heading(level)) if !spans.is_empty() => {
                self.push_block(Block::Heading(level, spans));
            }
            Some(Inline::Cell) => {
                if self.row.len() >= MAX_TABLE_COLS {
                    self.overflow = true;
                } else if self.items >= MAX_DOC_ITEMS {
                    self.truncated = true;
                } else {
                    self.items += 1;
                    self.row.push(spans);
                }
            }
            _ => {}
        }
    }

    fn event(&mut self, ev: Event<'_>) {
        if self.in_comment {
            // Inside a multi-line inline comment nothing is kept.
            if let Event::InlineHtml(s) | Event::Html(s) | Event::Text(s) = &ev {
                if s.contains("-->") {
                    self.in_comment = false;
                }
            }
            return;
        }
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(end) => self.end(end),
            Event::Text(s) => {
                if let Some(code) = &mut self.code {
                    code.push_str(&s);
                } else if let Some(html) = &mut self.html {
                    html.push_str(&s);
                } else if let Some(alt) = &mut self.image_alt {
                    alt.push_str(&s);
                } else {
                    self.add_span(s.into_string());
                }
            }
            Event::Code(s) => {
                if let Some(alt) = &mut self.image_alt {
                    alt.push_str(&s);
                } else if self.room() {
                    self.spans.push(Span {
                        text: s.into_string(),
                        strong: self.strong > 0,
                        emph: self.emph > 0,
                        code: true,
                        link: self.link.clone(),
                    });
                }
            }
            Event::Html(s) => {
                if let Some(html) = &mut self.html {
                    html.push_str(&s);
                }
            }
            Event::InlineHtml(s) => {
                if s.contains("<!--") && !s.contains("-->") {
                    self.in_comment = true;
                } else {
                    let text = strip_html(&s);
                    self.add_span(text);
                }
            }
            Event::SoftBreak => {
                if let Some(alt) = &mut self.image_alt {
                    alt.push(' ');
                } else if self.code.is_none() {
                    self.add_span(" ".to_string());
                }
            }
            Event::HardBreak => self.add_span("\n".to_string()),
            Event::Rule => {
                self.flush_implicit();
                self.push_block(Block::Rule);
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.flush_implicit();
                self.inline = Some(Inline::Paragraph);
            }
            Tag::Heading { level, .. } => {
                self.flush_implicit();
                self.inline = Some(Inline::Heading(level as u8));
            }
            Tag::BlockQuote(_) => self.open_container(Frame::Quote(Vec::new())),
            Tag::List(first) => self.open_container(Frame::List {
                ordered: first.is_some(),
                items: Vec::new(),
            }),
            Tag::Item => self.open_container(Frame::Item(Vec::new())),
            Tag::CodeBlock(_) => {
                self.flush_implicit();
                self.code = Some(String::new());
            }
            Tag::HtmlBlock => {
                self.flush_implicit();
                self.html = Some(String::new());
            }
            Tag::Table(_) => {
                self.flush_implicit();
                self.table = Some(Vec::new());
            }
            Tag::TableHead | Tag::TableRow => self.row = Vec::new(),
            Tag::TableCell => {
                self.spans.clear();
                self.inline = Some(Inline::Cell);
            }
            Tag::Strong => self.strong += 1,
            Tag::Emphasis => self.emph += 1,
            Tag::Link { dest_url, .. } => {
                self.link = is_safe_link(&dest_url).then(|| dest_url.into_string());
            }
            Tag::Image { .. } => self.image_alt = Some(String::new()),
            _ => {}
        }
    }

    fn end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell => self.end_inline(),
            TagEnd::BlockQuote(_) => self.close_container(false),
            TagEnd::List(_) => self.close_container(false),
            TagEnd::Item => self.close_container(true),
            TagEnd::CodeBlock => {
                if let Some(code) = self.code.take() {
                    self.push_block(Block::CodeBlock(code));
                }
            }
            TagEnd::HtmlBlock => {
                if let Some(html) = self.html.take() {
                    let text = strip_html(&html);
                    let text = text.trim();
                    if !text.is_empty() {
                        self.push_block(Block::Paragraph(vec![Span {
                            text: text.to_string(),
                            ..Span::default()
                        }]));
                    }
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                let row = std::mem::take(&mut self.row);
                if let Some(t) = &mut self.table {
                    if t.len() >= MAX_TABLE_ROWS {
                        self.overflow = true;
                    } else {
                        t.push(row);
                    }
                }
            }
            TagEnd::Table => {
                if let Some(rows) = self.table.take() {
                    self.push_block(Block::Table(rows));
                }
            }
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Emphasis => self.emph = self.emph.saturating_sub(1),
            TagEnd::Link => self.link = None,
            TagEnd::Image => {
                if let Some(alt) = self.image_alt.take() {
                    let alt = alt.trim();
                    let text = if alt.is_empty() {
                        "[image]".to_string()
                    } else {
                        format!("[image: {alt}]")
                    };
                    self.add_span(text);
                }
            }
            _ => {}
        }
    }

    fn finish(mut self) -> (Vec<Block>, bool) {
        if self.truncated {
            // Keep what was being built when the budget ran out.
            if matches!(self.inline, Some(Inline::Paragraph | Inline::Heading(_))) {
                self.end_inline();
            }
            if let Some(rows) = self.table.take() {
                self.push_block(Block::Table(rows));
            }
        } else {
            self.flush_implicit();
        }
        while self.frames.len() > 1 {
            self.merge_top();
        }
        let blocks = match self.frames.pop() {
            Some(Frame::Root(v)) => v,
            _ => Vec::new(),
        };
        (blocks, self.truncated || self.overflow)
    }
}

/// Removes tags, comments and the content of script and style elements,
/// keeping the text between them.
fn strip_html(src: &str) -> String {
    // Lowercased once; ASCII lowercasing keeps every byte offset.
    let lower_src = src.to_ascii_lowercase();
    let mut out = String::new();
    let mut rest = src;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(after) = tail.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |j| &after[j + 3..]);
            continue;
        }
        let starts_tag = tail[1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'));
        if !starts_tag {
            out.push('<');
            rest = &tail[1..];
            continue;
        }
        let name: String = tail[1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let tag_end = tail.find('>').map_or(tail.len(), |j| j + 1);
        rest = &tail[tag_end..];
        if matches!(name.as_str(), "script" | "style") && !tail.starts_with("</") {
            let close = format!("</{name}");
            let lower = &lower_src[src.len() - rest.len()..];
            rest = match lower.find(&close) {
                Some(j) => {
                    let after = &rest[j..];
                    after.find('>').map_or("", |k| &after[k + 1..])
                }
                None => "",
            };
        }
    }
    out.push_str(rest);
    out
}

/// Rough characters per wrapped line, used to estimate paragraph height.
const LINE_CHARS: usize = 90;

struct Budget {
    lines: usize,
    chars: usize,
    cut: bool,
}

fn span_chars(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.text.chars().count()).sum()
}

/// The leading `max` characters of `spans`, ending in an ellipsis.
fn truncate_spans(spans: &[Span], max: usize) -> Vec<Span> {
    let mut left = max;
    let mut out = Vec::new();
    for s in spans {
        if left == 0 {
            break;
        }
        let n = s.text.chars().count();
        if n <= left {
            left -= n;
            out.push(s.clone());
        } else {
            let mut cut = s.clone();
            cut.text = s.text.chars().take(left).collect();
            out.push(cut);
            left = 0;
        }
    }
    out.push(Span {
        text: "…".to_string(),
        ..Span::default()
    });
    out
}

fn take(blocks: &[Block], b: &mut Budget) -> Vec<Block> {
    let mut out = Vec::new();
    for block in blocks {
        if b.cut {
            break;
        }
        match block {
            Block::Heading(_, spans) => {
                let chars = span_chars(spans);
                if b.lines >= 1 && b.chars >= chars {
                    b.lines -= 1;
                    b.chars -= chars;
                    out.push(block.clone());
                } else {
                    b.cut = true;
                }
            }
            Block::Rule => {
                if b.lines >= 1 {
                    b.lines -= 1;
                    out.push(Block::Rule);
                } else {
                    b.cut = true;
                }
            }
            Block::Paragraph(spans) => {
                let chars = span_chars(spans);
                let lines = chars.div_ceil(LINE_CHARS).max(1);
                if lines <= b.lines && chars <= b.chars {
                    b.lines -= lines;
                    b.chars -= chars;
                    out.push(block.clone());
                } else {
                    let allowed = b.chars.min(b.lines * LINE_CHARS);
                    if allowed > 0 {
                        out.push(Block::Paragraph(truncate_spans(spans, allowed)));
                    }
                    b.cut = true;
                }
            }
            Block::CodeBlock(code) => {
                let lines = code.lines().count().max(1);
                if lines <= b.lines && code.chars().count() <= b.chars {
                    b.lines -= lines;
                    b.chars -= code.chars().count();
                    out.push(block.clone());
                } else {
                    let mut kept = String::new();
                    let mut used = 0;
                    for line in code.lines().take(b.lines) {
                        let n = line.chars().count() + 1;
                        if used + n > b.chars {
                            break;
                        }
                        used += n;
                        kept.push_str(line);
                        kept.push('\n');
                    }
                    if !kept.is_empty() {
                        out.push(Block::CodeBlock(kept));
                    }
                    b.cut = true;
                }
            }
            Block::Table(rows) => {
                let mut kept = Vec::new();
                for row in rows {
                    let row: Vec<Vec<Span>> = row.iter().take(MAX_TABLE_COLS).cloned().collect();
                    let chars: usize = row.iter().map(|c| span_chars(c)).sum();
                    if b.lines >= 1 && b.chars >= chars {
                        b.lines -= 1;
                        b.chars -= chars;
                        kept.push(row);
                    } else {
                        b.cut = true;
                        break;
                    }
                }
                if !kept.is_empty() {
                    out.push(Block::Table(kept));
                }
            }
            Block::Quote(inner) => {
                let kept = take(inner, b);
                if !kept.is_empty() {
                    out.push(Block::Quote(kept));
                }
            }
            Block::List { ordered, items } => {
                let mut kept = Vec::new();
                for item in items {
                    if b.cut {
                        break;
                    }
                    let inner = take(item, b);
                    if !inner.is_empty() {
                        kept.push(inner);
                    }
                }
                if !kept.is_empty() {
                    out.push(Block::List {
                        ordered: *ordered,
                        items: kept,
                    });
                }
            }
        }
    }
    out
}

/// The leading part of `blocks` within `limits`, and whether more follows.
pub fn preview(blocks: &[Block], limits: PreviewLimits) -> (Vec<Block>, bool) {
    let mut budget = Budget {
        lines: limits.lines,
        chars: limits.chars,
        cut: false,
    };
    let kept = take(blocks, &mut budget);
    (kept, budget.cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    /// Every piece of text in a block tree, for "must not contain" checks.
    fn all_text(blocks: &[Block]) -> String {
        let mut out = String::new();
        for b in blocks {
            match b {
                Block::Heading(_, s) | Block::Paragraph(s) => out.push_str(&plain(s)),
                Block::List { items, .. } => items.iter().for_each(|i| out.push_str(&all_text(i))),
                Block::Quote(c) => out.push_str(&all_text(c)),
                Block::CodeBlock(c) => out.push_str(c),
                Block::Table(rows) => rows
                    .iter()
                    .flatten()
                    .for_each(|cell| out.push_str(&plain(cell))),
                Block::Rule => {}
            }
            out.push('\n');
        }
        out
    }

    fn all_spans(blocks: &[Block]) -> Vec<Span> {
        let mut out = Vec::new();
        for b in blocks {
            match b {
                Block::Heading(_, s) | Block::Paragraph(s) => out.extend(s.iter().cloned()),
                Block::List { items, .. } => items.iter().for_each(|i| out.extend(all_spans(i))),
                Block::Quote(c) => out.extend(all_spans(c)),
                Block::Table(rows) => rows.iter().flatten().for_each(|c| out.extend(c.clone())),
                _ => {}
            }
        }
        out
    }

    fn depth(blocks: &[Block]) -> usize {
        blocks
            .iter()
            .map(|b| match b {
                Block::Quote(c) => 1 + depth(c),
                Block::List { items, .. } => 1 + items.iter().map(|i| depth(i)).max().unwrap_or(0),
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }

    fn leaf_count(blocks: &[Block]) -> usize {
        blocks
            .iter()
            .map(|b| match b {
                Block::Quote(c) => leaf_count(c),
                Block::List { items, .. } => items.iter().map(|i| leaf_count(i)).sum(),
                _ => 1,
            })
            .sum()
    }

    #[test]
    fn empty_input_has_no_blocks() {
        assert!(parse("").is_empty());
        assert!(parse("  \n\n ").is_empty());
    }

    #[test]
    fn headings_carry_their_level() {
        let b = parse("# One\n\n### Three\n");
        assert_eq!(b.len(), 2);
        match (&b[0], &b[1]) {
            (Block::Heading(1, a), Block::Heading(3, c)) => {
                assert_eq!(plain(a), "One");
                assert_eq!(plain(c), "Three");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn paragraphs_join_soft_breaks_with_a_space() {
        let b = parse("one\ntwo\n\nthree");
        assert_eq!(b.len(), 2);
        assert!(matches!(&b[0], Block::Paragraph(s) if plain(s) == "one two"));
    }

    #[test]
    fn nested_lists_keep_structure() {
        let b = parse("- a\n  - b\n  - c\n- d\n\n1. x\n2. y\n");
        match &b[0] {
            Block::List { ordered, items } => {
                assert!(!ordered);
                assert_eq!(items.len(), 2);
                assert!(matches!(&items[0][0], Block::Paragraph(s) if plain(s) == "a"));
                assert!(
                    matches!(&items[0][1], Block::List { items, .. } if items.len() == 2),
                    "{:?}",
                    items[0]
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&b[1], Block::List { ordered: true, items } if items.len() == 2));
    }

    #[test]
    fn fenced_code_is_kept_verbatim() {
        let b = parse("```python\nprint('<hi>')\n  x = 1\n```\n");
        assert_eq!(b, vec![Block::CodeBlock("print('<hi>')\n  x = 1\n".into())]);
    }

    #[test]
    fn tables_become_rows_of_cells() {
        let b = parse("| a | b |\n|---|---|\n| 1 | `2` |\n");
        match &b[0] {
            Block::Table(rows) => {
                assert_eq!(rows.len(), 2);
                assert_eq!(plain(&rows[0][0]), "a");
                assert_eq!(plain(&rows[1][0]), "1");
                assert!(rows[1][1][0].code);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn emphasis_and_inline_code_set_span_flags() {
        let b = parse("a **b** *c* `d` ***e***");
        let spans = all_spans(&b);
        let find = |t: &str| spans.iter().find(|s| s.text == t).cloned().unwrap();
        assert!(find("b").strong && !find("b").emph);
        assert!(find("c").emph && !find("c").strong);
        assert!(find("d").code);
        assert!(find("e").strong && find("e").emph);
        assert!(!find("a ").strong);
    }

    #[test]
    fn safe_links_keep_their_url_and_text() {
        let b = parse("see [the docs](https://example.com/a?b=1) now");
        let s = all_spans(&b);
        let link = s.iter().find(|s| s.link.is_some()).unwrap();
        assert_eq!(link.text, "the docs");
        assert_eq!(link.link.as_deref(), Some("https://example.com/a?b=1"));
    }

    #[test]
    fn unsafe_and_relative_links_are_plain_text() {
        for md in [
            "[x](javascript:alert(1))",
            "[x](JavaScript:alert(1))",
            "[x](file:///etc/passwd)",
            "[x](data:text/html;base64,AAAA)",
            "[x](relative/path.md)",
            "[x](//evil.example/x)",
            "[x](vbscript:run)",
        ] {
            let spans = all_spans(&parse(md));
            assert!(spans.iter().all(|s| s.link.is_none()), "{md}: {spans:?}");
            assert_eq!(plain(&spans), "x", "{md}");
        }
    }

    #[test]
    fn is_safe_link_allows_only_absolute_http_and_https() {
        assert!(is_safe_link("http://example.com"));
        assert!(is_safe_link("https://example.com/a?b#c"));
        assert!(is_safe_link("HTTPS://example.com"));
        for bad in [
            "",
            "javascript:alert(1)",
            "file:///x",
            "data:text/plain,hi",
            "ftp://example.com",
            "/relative",
            "relative.md",
            "#anchor",
            "mailto:a@b.c",
            "https://",
            "https://exa mple.com",
            "https://example.com/\n",
            " https://example.com",
        ] {
            assert!(!is_safe_link(bad), "{bad:?}");
        }
    }

    #[test]
    fn images_become_alt_text_and_never_a_link() {
        let b = parse("![the logo](http://tracker/pixel.png) after");
        let text = all_text(&b);
        assert!(text.contains("[image: the logo]"), "{text}");
        assert!(!text.contains("tracker"), "{text}");
        assert!(all_spans(&b).iter().all(|s| s.link.is_none()));
    }

    #[test]
    fn linked_badge_images_keep_only_the_link() {
        let b = parse("[![build](https://img.example/b.svg)](https://ci.example/run)");
        let s = all_spans(&b);
        assert!(s.iter().any(|s| s.text.contains("[image: build]")
            && s.link.as_deref() == Some("https://ci.example/run")));
        assert!(!all_text(&b).contains("img.example"));
    }

    #[test]
    fn raw_html_is_never_kept() {
        let md = "<script>alert(1)</script>\n\n<img src=x onerror=alert(1)>\n\ntext <b onclick=\"x()\">bold</b> end\n\n<div align=\"center\"><h1>Title</h1></div>\n";
        let text = all_text(&parse(md));
        for bad in ["<", ">", "onerror", "onclick", "alert", "script"] {
            assert!(!text.contains(bad), "{bad:?} in {text:?}");
        }
        assert!(text.contains("bold"), "{text:?}");
        assert!(text.contains("Title"), "{text:?}");
    }

    #[test]
    fn html_comments_are_dropped() {
        let md = "before\n\n<!-- secret\nmulti line -->\n\nafter <!-- inline --> end\n";
        let text = all_text(&parse(md));
        assert!(
            !text.contains("secret") && !text.contains("inline"),
            "{text:?}"
        );
        assert!(text.contains("before") && text.contains("after") && text.contains("end"));
    }

    #[test]
    fn nesting_depth_is_capped_at_eight() {
        let md = format!("{} deep\n", ">".repeat(10_000));
        let b = parse(&md);
        assert!(depth(&b) <= MAX_DEPTH, "depth {}", depth(&b));
        assert!(all_text(&b).contains("deep"));
        let mut list = String::new();
        for i in 0..40 {
            list.push_str(&format!("{}- item {i}\n", "  ".repeat(i)));
        }
        let b = parse(&list);
        assert!(depth(&b) <= MAX_DEPTH, "list depth {}", depth(&b));
        assert!(all_text(&b).contains("item 39"));
    }

    #[test]
    fn long_documents_are_cut_at_the_block_cap() {
        let md: String = (0..10_000).map(|i| format!("line {i}\n\n")).collect();
        let (blocks, cut) = parse_capped(&md);
        assert!(cut);
        assert_eq!(leaf_count(&blocks), MAX_BLOCKS);
        let (_, cut) = parse_capped("short\n");
        assert!(!cut);
    }

    #[test]
    fn a_quarter_megabyte_single_line_does_not_panic() {
        let md = "a".repeat(MAX_INPUT_BYTES);
        let b = parse(&md);
        assert_eq!(b.len(), 1);
        let over = "b".repeat(MAX_INPUT_BYTES * 2);
        let text = all_text(&parse(&over));
        assert!(text.len() <= MAX_INPUT_BYTES + 2, "{}", text.len());
    }

    #[test]
    fn input_is_cut_on_a_char_boundary() {
        let md = "é".repeat(MAX_INPUT_BYTES);
        let b = parse(&md);
        assert!(!b.is_empty());
    }

    fn para(text: &str) -> Block {
        Block::Paragraph(vec![Span {
            text: text.into(),
            ..Span::default()
        }])
    }

    #[test]
    fn preview_of_a_short_card_is_the_whole_card() {
        let blocks = parse("# T\n\nshort text\n");
        let (p, more) = preview(&blocks, COLLAPSED);
        assert_eq!(p, blocks);
        assert!(!more);
    }

    #[test]
    fn preview_stops_after_twelve_lines() {
        let blocks: Vec<Block> = (0..30).map(|i| para(&format!("p{i}"))).collect();
        let (p, more) = preview(&blocks, COLLAPSED);
        assert!(more);
        assert_eq!(p.len(), 12);
        assert_eq!(p[..], blocks[..12]);
    }

    #[test]
    fn preview_cuts_a_long_paragraph_at_the_character_limit() {
        let blocks = vec![para(&"word ".repeat(1000))];
        let (p, more) = preview(&blocks, COLLAPSED);
        assert!(more);
        let text = all_text(&p);
        assert!(
            text.chars().count() <= COLLAPSED.chars + 4,
            "{}",
            text.len()
        );
        assert!(text.starts_with("word word"));
    }

    #[test]
    fn preview_counts_code_lines_and_cuts_inside_a_long_block() {
        let code = (0..50).map(|i| format!("l{i}\n")).collect::<String>();
        let (p, more) = preview(&[Block::CodeBlock(code)], COLLAPSED);
        assert!(more);
        match &p[0] {
            Block::CodeBlock(c) => assert_eq!(c.lines().count(), 12),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn preview_cuts_inside_lists_and_quotes() {
        let items: Vec<Vec<Block>> = (0..40).map(|i| vec![para(&format!("i{i}"))]).collect();
        let blocks = vec![Block::Quote(vec![Block::List {
            ordered: false,
            items,
        }])];
        let (p, more) = preview(&blocks, COLLAPSED);
        assert!(more);
        assert!(leaf_count(&p) <= 12 && leaf_count(&p) > 0);
    }

    /// Spans and cells in a block tree.
    fn items(blocks: &[Block]) -> usize {
        blocks
            .iter()
            .map(|b| match b {
                Block::Heading(_, s) | Block::Paragraph(s) => s.len(),
                Block::List { items: it, .. } => it.iter().map(|i| items(i)).sum(),
                Block::Quote(c) => items(c),
                Block::Table(rows) => {
                    rows.iter().map(|r| r.len()).sum::<usize>()
                        + rows.iter().flatten().map(Vec::len).sum::<usize>()
                }
                _ => 0,
            })
            .sum()
    }

    #[test]
    fn a_table_of_100k_cells_is_bounded_and_flagged() {
        let md = format!("|a|b|\n|-|-|\n{}", "|||\n".repeat(50_000));
        let (b, cut) = parse_capped(&md);
        assert!(cut);
        match &b[0] {
            Block::Table(rows) => {
                assert_eq!(rows.len(), MAX_TABLE_ROWS);
                assert!(rows.iter().all(|r| r.len() <= MAX_TABLE_COLS));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_table_wider_than_the_column_cap_is_cut_and_flagged() {
        let row = |c: &str| format!("|{}\n", c.repeat(100));
        let md = format!("{}{}{}", row("h|"), row("-|"), row("x|"));
        let (b, cut) = parse_capped(&md);
        assert!(cut);
        match &b[0] {
            Block::Table(rows) => assert!(rows.iter().all(|r| r.len() == MAX_TABLE_COLS)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_paragraph_of_120k_spans_is_bounded_and_flagged() {
        let (b, cut) = parse_capped(&"*a* ".repeat(60_000));
        assert!(cut);
        match &b[0] {
            Block::Paragraph(s) => assert_eq!(s.len(), MAX_SPANS_PER_BLOCK),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_whole_document_is_bounded_in_spans_and_cells() {
        let para = format!("{}\n\n", "*a* ".repeat(900));
        let (b, cut) = parse_capped(&para.repeat(30));
        assert!(cut);
        assert!(items(&b) <= MAX_DOC_ITEMS, "{}", items(&b));
        assert!(items(&b) > MAX_DOC_ITEMS / 2);
    }

    #[test]
    fn ten_thousand_tiny_paragraphs_and_one_huge_line_stay_bounded() {
        let md: String = (0..10_000).map(|i| format!("p{i}\n\n")).collect();
        let (b, cut) = parse_capped(&md);
        assert!(cut);
        assert_eq!(leaf_count(&b), MAX_BLOCKS);
        let (b, _) = parse_capped(&"a".repeat(MAX_INPUT_BYTES));
        assert_eq!(items(&b), 1);
    }

    #[test]
    fn preview_caps_cells_per_row() {
        let cell = vec![Span::default()];
        let table = Block::Table(vec![vec![cell; 100]]);
        let (p, _) = preview(&[table], COLLAPSED);
        match &p[0] {
            Block::Table(rows) => assert_eq!(rows[0].len(), MAX_TABLE_COLS),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn strip_html_is_linear_in_the_number_of_script_tags() {
        let html = "<script>a</script>".repeat(20_000);
        let start = std::time::Instant::now();
        let out = strip_html(&html);
        assert!(out.is_empty());
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "took {:?}",
            start.elapsed()
        );
    }
}
