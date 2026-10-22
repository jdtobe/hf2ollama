//! Markdown model cards as a small, safe block tree. Pure: no egui, no
#![allow(dead_code)] // TODO(task 14b): removed once about.rs draws the blocks.
//! network. Raw HTML never survives, images are never loaded, and only
//! absolute http(s) links are kept.

/// Largest input parsed, matching what the core fetch already enforces.
pub const MAX_INPUT_BYTES: usize = 256 * 1024;
/// Container (list and quote) nesting beyond this is flattened.
pub const MAX_DEPTH: usize = 8;
/// Leaf blocks kept; the rest is cut and reported as truncated.
pub const MAX_BLOCKS: usize = 400;

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

/// Only absolute http and https URLs may be opened.
pub fn is_safe_link(_url: &str) -> bool {
    false
}

/// Parses Markdown into blocks; see `parse_capped` for the cut flag.
pub fn parse(src: &str) -> Vec<Block> {
    parse_capped(src).0
}

/// Like `parse`, also telling whether the block cap cut the document.
pub fn parse_capped(_src: &str) -> (Vec<Block>, bool) {
    (Vec::new(), false)
}

/// The leading part of `blocks` within `limits`, and whether more follows.
pub fn preview(blocks: &[Block], _limits: PreviewLimits) -> (Vec<Block>, bool) {
    (blocks.to_vec(), false)
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
}
