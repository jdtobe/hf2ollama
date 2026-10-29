//! The read-only Files tools: `list_dir`, `read_file`, `find_files` and
//! `search_text`. Every result has deterministic limits and says so when it
//! was cut. Nothing here panics on odd input; failures are `ToolError`s.

use super::workspace::{is_sensitive, EntryKind, ToolError, Workspace};
use globset::GlobBuilder;
use regex::RegexBuilder;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

/// The limits from the design. Tests pass smaller ones for the big counts.
#[derive(Debug, Clone)]
pub struct Limits {
    pub list_entries: usize,
    pub read_default_lines: usize,
    pub read_max_lines: usize,
    pub output_bytes: usize,
    pub find_results: usize,
    pub depth: usize,
    pub examined: usize,
    pub search_matches: usize,
    pub per_file_matches: usize,
    pub line_chars: usize,
    pub read_file_bytes: u64,
    pub search_file_bytes: u64,
    /// A command is stopped after this long.
    pub run_timeout: std::time::Duration,
    /// A command's output is cut here.
    pub run_output_bytes: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            list_entries: 200,
            read_default_lines: 200,
            read_max_lines: 1_000,
            output_bytes: 24 * 1024,
            find_results: 200,
            depth: 64,
            examined: 100_000,
            search_matches: 200,
            per_file_matches: 10,
            line_chars: 300,
            read_file_bytes: 16 * 1024 * 1024,
            search_file_bytes: 2 * 1024 * 1024,
            run_timeout: std::time::Duration::from_secs(120),
            run_output_bytes: 20 * 1024,
        }
    }
}

const BINARY_SNIFF: usize = 8 * 1024;
/// A single line is never read past this, so one huge line cannot use memory.
const LINE_READ_MAX: usize = 16 * 1024;

/// Text with a byte cap; the first line that does not fit ends it.
struct Out {
    buf: String,
    cap: usize,
    full: bool,
}

impl Out {
    fn new(cap: usize) -> Out {
        Out {
            buf: String::new(),
            cap,
            full: false,
        }
    }

    /// Adds a line unless it would pass the cap. False once the cap is hit.
    fn line(&mut self, text: &str) -> bool {
        if self.full {
            return false;
        }
        if self.buf.len() + text.len() + 1 > self.cap {
            self.full = true;
            return false;
        }
        self.buf.push_str(text);
        self.buf.push('\n');
        true
    }

    fn finish(mut self, notes: &[String]) -> String {
        for n in notes {
            self.buf.push_str(n);
            self.buf.push('\n');
        }
        self.buf.trim_end().to_string()
    }
}

fn str_arg(args: &Value, name: &str) -> Result<Option<String>, ToolError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ToolError::InvalidArguments(format!(
            "{name} must be a string"
        ))),
    }
}

fn required_str(args: &Value, name: &str) -> Result<String, ToolError> {
    str_arg(args, name)?.ok_or_else(|| ToolError::InvalidArguments(format!("{name} is required")))
}

fn uint_arg(args: &Value, name: &str) -> Result<Option<u64>, ToolError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| ToolError::InvalidArguments(format!("{name} must be a whole number"))),
    }
}

fn bool_arg(args: &Value, name: &str) -> Result<bool, ToolError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(ToolError::InvalidArguments(format!(
            "{name} must be true or false"
        ))),
    }
}

pub(crate) fn show(rel: &Path) -> String {
    let s = rel.to_string_lossy().replace('\\', "/");
    if s.is_empty() {
        ".".into()
    } else {
        s
    }
}

// ------------------------------------------------------------------ list_dir

pub fn list_dir(ws: &Workspace, args: &Value, lim: &Limits) -> Result<String, ToolError> {
    let rel = ws.resolve(&str_arg(args, "path")?.unwrap_or_default())?;
    let entries = ws.list(&rel)?;
    if entries.is_empty() {
        return Ok("(empty directory)".into());
    }
    let mut out = Out::new(lim.output_bytes);
    let mut shown = 0;
    for e in entries.iter().take(lim.list_entries) {
        let mark = match e.kind {
            EntryKind::Dir => "/",
            EntryKind::Symlink => "@",
            _ => "",
        };
        let protected = if is_sensitive(&rel.join(&e.name)) {
            " [protected]"
        } else {
            ""
        };
        if !out.line(&format!("{}{mark}{protected}", e.name)) {
            break;
        }
        shown += 1;
    }
    let mut notes = Vec::new();
    if shown < entries.len() {
        notes.push(format!(
            "[truncated: showing {shown} of {} entries]",
            entries.len()
        ));
    }
    Ok(out.finish(&notes))
}

// ----------------------------------------------------------------- read_file

fn read_line(r: &mut impl BufRead, buf: &mut Vec<u8>) -> std::io::Result<(usize, bool)> {
    buf.clear();
    let n = r
        .by_ref()
        .take(LINE_READ_MAX as u64 + 1)
        .read_until(b'\n', buf)?;
    let mut cut = false;
    if buf.len() > LINE_READ_MAX {
        cut = true;
        buf.truncate(LINE_READ_MAX);
        // Drop the rest of an over-long line.
        let mut sink = Vec::new();
        r.read_until(b'\n', &mut sink)?;
    }
    Ok((n, cut))
}

pub fn read_file(ws: &Workspace, args: &Value, lim: &Limits) -> Result<String, ToolError> {
    let rel = ws.resolve(&required_str(args, "path")?)?;
    let offset = uint_arg(args, "offset")?.unwrap_or(1);
    if offset == 0 {
        return Err(ToolError::InvalidArguments("offset starts at 1".into()));
    }
    let limit = match uint_arg(args, "limit")? {
        None => lim.read_default_lines,
        Some(0) => {
            return Err(ToolError::InvalidArguments(
                "limit must be at least 1".into(),
            ))
        }
        Some(n) => (n as usize).min(lim.read_max_lines),
    };
    let (file, size) = ws.open_file(&rel)?;
    if size > lim.read_file_bytes {
        return Err(ToolError::Unreadable(format!(
            "{} is too large to read ({size} bytes); use search_text to find the part you need",
            show(&rel)
        )));
    }
    let mut reader = BufReader::new(file);
    {
        let head = reader
            .fill_buf()
            .map_err(|e| ToolError::Unreadable(format!("could not read {}: {e}", show(&rel))))?;
        if head.iter().take(BINARY_SNIFF).any(|b| *b == 0) {
            return Err(ToolError::Unreadable(format!(
                "{} is a binary file; use read_binary to see its bytes",
                show(&rel)
            )));
        }
    }
    let mut out = Out::new(lim.output_bytes);
    let mut buf = Vec::new();
    let mut line_no: u64 = 0;
    let mut returned = 0usize;
    let mut next: Option<u64> = None;
    loop {
        let (n, cut) = read_line(&mut reader, &mut buf)
            .map_err(|e| ToolError::Unreadable(format!("could not read {}: {e}", show(&rel))))?;
        if n == 0 {
            break;
        }
        line_no += 1;
        if line_no < offset {
            continue;
        }
        if returned >= limit {
            next = Some(line_no);
            break;
        }
        let text = std::str::from_utf8(&buf).map_err(|_| {
            ToolError::Unreadable(format!("{} is not valid UTF-8 text", show(&rel)))
        })?;
        let text = text.trim_end_matches(['\n', '\r']);
        let suffix = if cut { " … [line cut]" } else { "" };
        if !out.line(&format!("{line_no}| {text}{suffix}")) {
            next = Some(line_no);
            break;
        }
        returned += 1;
    }
    let mut notes = Vec::new();
    if let Some(n) = next {
        notes.push(format!("[truncated; call read_file again with offset={n}]"));
    } else if returned == 0 {
        return Ok(format!(
            "(no lines at offset {offset}; the file has {line_no} line{})",
            if line_no == 1 { "" } else { "s" }
        ));
    }
    Ok(out.finish(&notes))
}

/// A whole number given as a JSON number or as text, decimal or `0x` hex
/// (models often write offsets the way a hex editor shows them).
fn byte_arg(args: &Value, name: &str) -> Result<Option<u64>, ToolError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(Some).ok_or_else(|| {
            ToolError::InvalidArguments(format!("{name} must be a whole number of bytes"))
        }),
        Some(Value::String(s)) => {
            let t = s.trim();
            let parsed = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                Some(hex) => u64::from_str_radix(hex, 16),
                None => t.parse::<u64>(),
            };
            parsed.map(Some).map_err(|_| {
                ToolError::InvalidArguments(format!(
                    "{name} must be a whole number of bytes, like 4096 or 0x1000"
                ))
            })
        }
        Some(_) => Err(ToolError::InvalidArguments(format!(
            "{name} must be a whole number of bytes"
        ))),
    }
}

/// Most bytes one `read_binary` call returns.
const BINARY_MAX_BYTES: u64 = 4096;
const BINARY_DEFAULT_BYTES: u64 = 256;

/// Shows bytes of any file as a hex dump: offset, 16 bytes in hex, then the
/// same bytes as text. Reads only the range asked for, so a file of any size
/// is fine.
pub fn read_binary(ws: &Workspace, args: &Value, lim: &Limits) -> Result<String, ToolError> {
    use std::io::{Seek, SeekFrom};
    let rel = ws.resolve(&required_str(args, "path")?)?;
    let offset = byte_arg(args, "offset")?.unwrap_or(0);
    let length = match byte_arg(args, "length")? {
        None => BINARY_DEFAULT_BYTES,
        Some(0) => {
            return Err(ToolError::InvalidArguments(
                "length must be at least 1".into(),
            ))
        }
        Some(n) => n.min(BINARY_MAX_BYTES),
    };
    let (mut file, size) = ws.open_file(&rel)?;
    let name = show(&rel);
    if offset >= size {
        return Ok(format!(
            "{name}: {size} bytes. Offset {offset} is past the end of the file."
        ));
    }
    let want = length.min(size - offset);
    let mut bytes = vec![0u8; want as usize];
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(&mut bytes))
        .map_err(|e| ToolError::Unreadable(format!("could not read {name}: {e}")))?;
    let mut out = Out::new(lim.output_bytes);
    out.line(&format!(
        "{name}: {size} bytes; showing {offset} to {} (0x{:x} to 0x{:x})",
        offset + want - 1,
        offset,
        offset + want - 1
    ));
    for (i, row) in bytes.chunks(16).enumerate() {
        let at = offset + (i * 16) as u64;
        let mut hex = String::new();
        for (j, b) in row.iter().enumerate() {
            if j == 8 {
                hex.push(' ');
            }
            hex.push_str(&format!("{b:02x} "));
        }
        let text: String = row
            .iter()
            .map(|b| {
                if (0x20..0x7f).contains(b) {
                    *b as char
                } else {
                    '.'
                }
            })
            .collect();
        // Padded to a full row so the text column lines up on a short last row.
        if !out.line(&format!("{at:08x}  {:<49} |{text}|", hex.trim_end())) {
            break;
        }
    }
    let mut notes = Vec::new();
    if offset + want < size {
        notes.push(format!(
            "[call read_binary again with offset={}]",
            offset + want
        ));
    }
    Ok(out.finish(&notes))
}

// -------------------------------------------------------------------- walking

/// What the walkers found out about their own run.
struct Walk {
    examined: usize,
    hit_examined: bool,
    hit_depth: bool,
}

/// Depth-first, name-sorted walk that never follows symlinks, skips `.git`
/// and protected paths, and stops after `lim.examined` entries. `visit`
/// returns false to stop the walk.
fn walk(
    ws: &Workspace,
    start: &Path,
    lim: &Limits,
    visit: &mut dyn FnMut(&Path, EntryKind) -> bool,
) -> Result<Walk, ToolError> {
    let mut state = Walk {
        examined: 0,
        hit_examined: false,
        hit_depth: false,
    };
    // A stack of directories to read, each with its depth.
    let mut stack: Vec<(PathBuf, usize)> = vec![(start.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let entries = match ws.list(&dir) {
            Ok(e) => e,
            // The start must exist; deeper failures (races, permissions) are skipped.
            Err(e) if dir == start => return Err(e),
            Err(_) => continue,
        };
        let mut subdirs = Vec::new();
        for e in entries {
            if state.examined >= lim.examined {
                state.hit_examined = true;
                return Ok(state);
            }
            state.examined += 1;
            let path = dir.join(&e.name);
            if e.name == ".git" || is_sensitive(&path) {
                continue;
            }
            match e.kind {
                EntryKind::Symlink | EntryKind::Other => {}
                EntryKind::Dir => {
                    if !visit(&path, EntryKind::Dir) {
                        return Ok(state);
                    }
                    if depth + 1 >= lim.depth {
                        state.hit_depth = true;
                    } else {
                        subdirs.push((path, depth + 1));
                    }
                }
                EntryKind::File => {
                    if !visit(&path, EntryKind::File) {
                        return Ok(state);
                    }
                }
            }
        }
        // Reverse so the stack pops them in name order.
        stack.extend(subdirs.into_iter().rev());
    }
    Ok(state)
}

fn walk_notes(w: &Walk, lim: &Limits) -> Vec<String> {
    let mut notes = Vec::new();
    if w.hit_examined {
        notes.push(format!(
            "[stopped after examining {} entries]",
            lim.examined
        ));
    }
    if w.hit_depth {
        notes.push(format!(
            "[directories deeper than {} levels were not searched]",
            lim.depth
        ));
    }
    notes
}

// ---------------------------------------------------------------- find_files

pub fn find_files(ws: &Workspace, args: &Value, lim: &Limits) -> Result<String, ToolError> {
    let pattern = required_str(args, "pattern")?;
    let start = ws.resolve(&str_arg(args, "path")?.unwrap_or_default())?;
    // A pattern without a slash matches names at any depth, like `*.rs`.
    let effective = if pattern.contains('/') {
        pattern.clone()
    } else {
        format!("**/{pattern}")
    };
    let glob = GlobBuilder::new(&effective)
        .literal_separator(true)
        .build()
        .map_err(|e| ToolError::InvalidArguments(format!("invalid glob: {e}")))?
        .compile_matcher();
    let mut out = Out::new(lim.output_bytes);
    let mut found = 0usize;
    let mut capped = false;
    let w = walk(ws, &start, lim, &mut |path, kind| {
        let rel_to_start = path.strip_prefix(&start).unwrap_or(path);
        if !glob.is_match(rel_to_start) {
            return true;
        }
        if found >= lim.find_results {
            capped = true;
            return false;
        }
        let name = format!(
            "{}{}",
            show(path),
            if kind == EntryKind::Dir { "/" } else { "" }
        );
        if !out.line(&name) {
            capped = true;
            return false;
        }
        found += 1;
        true
    })?;
    if found == 0 && !capped {
        let mut notes = walk_notes(&w, lim);
        notes.insert(0, "(no matches)".into());
        return Ok(out.finish(&notes));
    }
    let mut notes = Vec::new();
    if capped {
        notes.push(format!(
            "[truncated at {found} results; narrow the pattern or the path]"
        ));
    }
    notes.extend(walk_notes(&w, lim));
    Ok(out.finish(&notes))
}

// --------------------------------------------------------------- search_text

fn cut_line(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_string();
    }
    let head: String = text.chars().take(chars).collect();
    format!("{head}…")
}

/// Matches in one file, or `None` when the file is skipped (binary, large,
/// unreadable or not UTF-8).
fn search_one(
    ws: &Workspace,
    rel: &Path,
    re: &regex::Regex,
    lim: &Limits,
    room: usize,
) -> Option<Vec<(usize, String)>> {
    let (file, size) = ws.open_file(rel).ok()?;
    if size > lim.search_file_bytes {
        return None;
    }
    let mut reader = BufReader::new(file);
    let head = reader.fill_buf().ok()?;
    if head.iter().take(BINARY_SNIFF).any(|b| *b == 0) {
        return None;
    }
    let mut found = Vec::new();
    let mut buf = Vec::new();
    let mut n = 0usize;
    let cap = lim.per_file_matches.min(room);
    loop {
        let (read, _) = read_line(&mut reader, &mut buf).ok()?;
        if read == 0 {
            break;
        }
        n += 1;
        let Ok(text) = std::str::from_utf8(&buf) else {
            return None;
        };
        let text = text.trim_end_matches(['\n', '\r']);
        if re.is_match(text) {
            found.push((n, cut_line(text, lim.line_chars)));
            if found.len() >= cap {
                break;
            }
        }
    }
    Some(found)
}

pub fn search_text(ws: &Workspace, args: &Value, lim: &Limits) -> Result<String, ToolError> {
    let pattern = required_str(args, "pattern")?;
    let start = ws.resolve(&str_arg(args, "path")?.unwrap_or_default())?;
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(bool_arg(args, "case_insensitive")?)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| ToolError::InvalidArguments(format!("invalid regular expression: {e}")))?;
    let mut out = Out::new(lim.output_bytes);
    let mut total = 0usize;
    let mut capped = false;
    let mut files_with_matches = 0usize;
    let mut scan = |rel: &Path, out: &mut Out, total: &mut usize, capped: &mut bool| -> bool {
        let room = lim.search_matches - *total;
        let Some(found) = search_one(ws, rel, &re, lim, room) else {
            return true;
        };
        if !found.is_empty() {
            files_with_matches += 1;
        }
        for (line, text) in found {
            if !out.line(&format!("{}:{line}: {text}", show(rel))) {
                *capped = true;
                return false;
            }
            *total += 1;
            if *total >= lim.search_matches {
                *capped = true;
                return false;
            }
        }
        true
    };
    let walked = if ws.list(&start).is_ok() {
        walk(ws, &start, lim, &mut |path, kind| {
            kind != EntryKind::File || scan(path, &mut out, &mut total, &mut capped)
        })?
    } else {
        // Not a directory: search the single file (errors surface here).
        ws.open_file(&start)?;
        scan(&start, &mut out, &mut total, &mut capped);
        Walk {
            examined: 1,
            hit_examined: false,
            hit_depth: false,
        }
    };
    let _ = files_with_matches;
    let mut notes = Vec::new();
    if total == 0 && !capped {
        notes.push("(no matches)".to_string());
    }
    if capped {
        notes.push(format!(
            "[truncated at {total} matches; narrow the pattern or the path]"
        ));
    }
    notes.extend(walk_notes(&walked, lim));
    Ok(out.finish(&notes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn ws() -> (TempDir, Workspace) {
        let root = TempDir::new().unwrap();
        let w = Workspace::open(root.path()).unwrap();
        (root, w)
    }

    fn write(root: &TempDir, rel: &str, text: &str) {
        let p = root.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn lim() -> Limits {
        Limits::default()
    }

    // ---- list_dir

    #[test]
    fn list_dir_sorts_marks_kinds_and_flags_protected_entries() {
        let (r, w) = ws();
        write(&r, "b.txt", "");
        write(&r, "a.txt", "");
        write(&r, "src/main.rs", "");
        write(&r, ".env", "KEY=1");
        let out = list_dir(&w, &json!({}), &lim()).unwrap();
        assert_eq!(out, ".env [protected]\na.txt\nb.txt\nsrc/");
        assert_eq!(
            list_dir(&w, &json!({"path": "src"}), &lim()).unwrap(),
            "main.rs"
        );
    }

    #[test]
    fn list_dir_truncates_with_a_marker_and_reports_empty_directories() {
        let (r, w) = ws();
        for i in 0..250 {
            write(&r, &format!("f{i:03}.txt"), "");
        }
        fs::create_dir(r.path().join("empty")).unwrap();
        let out = list_dir(&w, &json!({}), &lim()).unwrap();
        assert!(
            out.ends_with("[truncated: showing 200 of 251 entries]"),
            "{}",
            out.lines().last().unwrap()
        );
        assert_eq!(out.lines().count(), 201);
        assert_eq!(
            list_dir(&w, &json!({"path": "empty"}), &lim()).unwrap(),
            "(empty directory)"
        );
    }

    #[test]
    fn list_dir_refuses_escapes_and_files() {
        let (r, w) = ws();
        write(&r, "f.txt", "x");
        assert!(matches!(
            list_dir(&w, &json!({"path": ".."}), &lim()),
            Err(ToolError::InvalidPath(_))
        ));
        assert!(matches!(
            list_dir(&w, &json!({"path": "/etc"}), &lim()),
            Err(ToolError::InvalidPath(_))
        ));
        assert!(matches!(
            list_dir(&w, &json!({"path": "f.txt"}), &lim()),
            Err(ToolError::NotADirectory(_))
        ));
        assert!(matches!(
            list_dir(&w, &json!({"path": 5}), &lim()),
            Err(ToolError::InvalidArguments(_))
        ));
    }

    // ---- read_file

    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    fn read_file_returns_numbered_lines_with_defaults() {
        let (r, w) = ws();
        write(&r, "a.txt", "alpha\nbeta\n");
        assert_eq!(
            read_file(&w, &json!({"path": "a.txt"}), &lim()).unwrap(),
            "1| alpha\n2| beta"
        );
    }

    #[test]
    fn read_file_pages_with_offset_and_limit_and_names_the_next_offset() {
        let (r, w) = ws();
        write(&r, "n.txt", &numbered(500));
        let first = read_file(&w, &json!({"path": "n.txt"}), &lim()).unwrap();
        assert!(first.starts_with("1| line 1\n"));
        assert!(first.contains("200| line 200\n"));
        assert!(first.ends_with("[truncated; call read_file again with offset=201]"));
        let mid = read_file(
            &w,
            &json!({"path": "n.txt", "offset": 10, "limit": 3}),
            &lim(),
        )
        .unwrap();
        assert_eq!(
            mid,
            "10| line 10\n11| line 11\n12| line 12\n[truncated; call read_file again with offset=13]"
        );
        let last = read_file(&w, &json!({"path": "n.txt", "offset": 499}), &lim()).unwrap();
        assert_eq!(last, "499| line 499\n500| line 500");
    }

    #[test]
    fn read_file_clamps_the_limit_and_validates_arguments() {
        let (r, w) = ws();
        write(&r, "n.txt", &numbered(1500));
        let out = read_file(&w, &json!({"path": "n.txt", "limit": 99999}), &lim()).unwrap();
        assert!(out.contains("1000| line 1000"));
        assert!(!out.contains("1001| "));
        assert!(matches!(
            read_file(&w, &json!({"path": "n.txt", "offset": 0}), &lim()),
            Err(ToolError::InvalidArguments(_))
        ));
        assert!(matches!(
            read_file(&w, &json!({"path": "n.txt", "limit": 0}), &lim()),
            Err(ToolError::InvalidArguments(_))
        ));
        assert!(matches!(
            read_file(&w, &json!({}), &lim()),
            Err(ToolError::InvalidArguments(_))
        ));
        assert!(matches!(
            read_file(&w, &json!({"path": "n.txt", "offset": "x"}), &lim()),
            Err(ToolError::InvalidArguments(_))
        ));
    }

    #[test]
    fn read_file_caps_output_at_24_kib_and_says_where_to_continue() {
        let (r, w) = ws();
        let long = format!("{}\n", "x".repeat(1000));
        write(&r, "wide.txt", &long.repeat(100));
        let out = read_file(&w, &json!({"path": "wide.txt", "limit": 100}), &lim()).unwrap();
        assert!(out.len() <= 24 * 1024 + 200, "{}", out.len());
        assert!(out.contains("[truncated; call read_file again with offset="));
        let off: u64 = out
            .rsplit("offset=")
            .next()
            .unwrap()
            .trim_end_matches(']')
            .parse()
            .unwrap();
        assert!(off > 1 && off < 101, "{off}");
    }

    #[test]
    fn read_file_past_the_end_says_how_long_the_file_is() {
        let (r, w) = ws();
        write(&r, "short.txt", "one\n");
        assert_eq!(
            read_file(&w, &json!({"path": "short.txt", "offset": 9}), &lim()).unwrap(),
            "(no lines at offset 9; the file has 1 line)"
        );
        write(&r, "empty.txt", "");
        assert!(read_file(&w, &json!({"path": "empty.txt"}), &lim())
            .unwrap()
            .starts_with("(no lines at offset 1"));
    }

    // ---- read_binary

    fn nes_header() -> Vec<u8> {
        let mut b = vec![0x4e, 0x45, 0x53, 0x1a, 2, 1, 0x01, 0x00];
        b.extend([0u8; 8]);
        b.extend(b"PRG".iter());
        b
    }

    #[test]
    fn read_binary_shows_offsets_hex_and_text_like_a_hex_dump() {
        let (r, w) = ws();
        fs::write(r.path().join("game.nes"), nes_header()).unwrap();
        let out = read_binary(&w, &json!({"path": "game.nes"}), &lim()).unwrap();
        assert!(out.contains("game.nes"), "{out}");
        assert!(out.contains("19 bytes"), "{out}");
        assert!(
            out.contains(
                "00000000  4e 45 53 1a 02 01 01 00  00 00 00 00 00 00 00 00  |NES.............|"
            ),
            "{out}"
        );
        assert!(out.contains("00000010  50 52 47"), "{out}");
        assert!(out.contains("|PRG|"), "{out}");
    }

    #[test]
    fn read_binary_pages_through_a_file_and_says_where_to_continue() {
        let (r, w) = ws();
        fs::write(r.path().join("b.bin"), (0..=255u8).collect::<Vec<_>>()).unwrap();
        let out = read_binary(
            &w,
            &json!({"path": "b.bin", "offset": 16, "length": 32}),
            &lim(),
        )
        .unwrap();
        assert!(out.starts_with("b.bin"), "{out}");
        assert!(out.contains("00000010  10 11 12"), "{out}");
        assert!(out.contains("00000020  20 21"), "{out}");
        assert!(!out.contains("00000030"), "{out}");
        assert!(out.contains("read_binary again with offset=48"), "{out}");
        // Hex offsets as text are accepted; a length past the cap is clamped.
        let out = read_binary(
            &w,
            &json!({"path": "b.bin", "offset": "0xF0", "length": 999999}),
            &lim(),
        )
        .unwrap();
        assert!(out.contains("000000f0  f0 f1"), "{out}");
        assert!(
            !out.contains("read_binary again"),
            "that was the end: {out}"
        );
        let past = read_binary(&w, &json!({"path": "b.bin", "offset": 300}), &lim()).unwrap();
        assert!(
            past.contains("256 bytes") && past.contains("past the end"),
            "{past}"
        );
    }

    #[test]
    fn read_binary_reads_a_big_file_without_loading_it() {
        let (r, w) = ws();
        let f = fs::File::create(r.path().join("big.bin")).unwrap();
        f.set_len(200 * 1024 * 1024).unwrap();
        let out = read_binary(
            &w,
            &json!({"path": "big.bin", "offset": 200 * 1024 * 1024 - 16}),
            &lim(),
        )
        .unwrap();
        assert!(out.contains("209715200 bytes"), "{out}");
        assert!(out.contains("0c7ffff0"), "{out}");
    }

    #[test]
    fn read_binary_refuses_protected_missing_folders_and_bad_numbers() {
        let (r, w) = ws();
        write(&r, ".env", "KEY=1");
        write(&r, "dir/x", "");
        for (args, want) in [
            (json!({"path": ".env"}), "protected"),
            (json!({"path": "nope.bin"}), "nope.bin"),
            (json!({"path": "dir"}), "dir"),
            (json!({"path": "../x"}), "parent"),
            (json!({"path": "dir/x", "offset": "soon"}), "offset"),
            (json!({"path": "dir/x", "length": 0}), "length"),
            (json!({}), "path"),
        ] {
            let e = read_binary(&w, &args, &lim()).unwrap_err().to_string();
            assert!(e.contains(want), "{args}: {e}");
        }
    }

    #[test]
    fn read_file_points_to_read_binary_for_a_binary_file() {
        let (r, w) = ws();
        fs::write(r.path().join("bin.dat"), [1u8, 0, 2, 3]).unwrap();
        let e = read_file(&w, &json!({"path": "bin.dat"}), &lim())
            .unwrap_err()
            .to_string();
        assert!(e.contains("read_binary"), "{e}");
    }

    #[test]
    fn read_file_refuses_binary_invalid_utf8_oversized_protected_and_missing() {
        let (r, w) = ws();
        fs::write(r.path().join("bin.dat"), [1u8, 0, 2, 3]).unwrap();
        fs::write(
            r.path().join("bad.txt"),
            [b'o', b'k', b'\n', 0xff, 0xfe, b'\n'],
        )
        .unwrap();
        write(&r, ".env", "KEY=1");
        write(&r, "dir/x", "");
        let small = Limits {
            read_file_bytes: 10,
            ..lim()
        };
        write(&r, "big.txt", "0123456789ABCDEF");
        for (p, want) in [("bin.dat", "binary"), ("bad.txt", "UTF-8")] {
            let e = read_file(&w, &json!({"path": p}), &lim())
                .unwrap_err()
                .to_string();
            assert!(e.contains(want), "{p}: {e}");
        }
        assert!(read_file(&w, &json!({"path": "big.txt"}), &small)
            .unwrap_err()
            .to_string()
            .contains("too large"));
        assert!(matches!(
            read_file(&w, &json!({"path": ".env"}), &lim()),
            Err(ToolError::Denied(_))
        ));
        assert!(matches!(
            read_file(&w, &json!({"path": "dir"}), &lim()),
            Err(ToolError::NotAFile(_))
        ));
        assert!(matches!(
            read_file(&w, &json!({"path": "missing"}), &lim()),
            Err(ToolError::NotFound(_))
        ));
        assert!(matches!(
            read_file(&w, &json!({"path": "../x"}), &lim()),
            Err(ToolError::InvalidPath(_))
        ));
    }

    #[test]
    fn read_file_cuts_a_single_enormous_line_instead_of_loading_it() {
        let (r, w) = ws();
        write(&r, "min.js", &format!("{}\nnext\n", "y".repeat(200_000)));
        let out = read_file(&w, &json!({"path": "min.js"}), &lim()).unwrap();
        assert!(out.len() < 30_000, "{}", out.len());
        assert!(out.contains("[line cut]"));
        assert!(out.contains("2| next"));
    }

    // ---- find_files

    fn tree(r: &TempDir) {
        write(r, "src/main.rs", "");
        write(r, "src/lib.rs", "");
        write(r, "src/deep/mod.rs", "");
        write(r, "README.md", "");
        write(r, ".git/config", "");
        write(r, ".env", "KEY=1");
        write(r, "certs/server.pem", "");
    }

    #[test]
    fn find_files_matches_names_at_any_depth_and_paths_with_slashes() {
        let (r, w) = ws();
        tree(&r);
        assert_eq!(
            find_files(&w, &json!({"pattern": "*.rs"}), &lim()).unwrap(),
            "src/lib.rs\nsrc/main.rs\nsrc/deep/mod.rs"
        );
        assert_eq!(
            find_files(&w, &json!({"pattern": "src/*.rs"}), &lim()).unwrap(),
            "src/lib.rs\nsrc/main.rs"
        );
        assert_eq!(
            find_files(&w, &json!({"pattern": "deep"}), &lim()).unwrap(),
            "src/deep/"
        );
        assert_eq!(
            find_files(&w, &json!({"pattern": "*.rs", "path": "src/deep"}), &lim()).unwrap(),
            "src/deep/mod.rs"
        );
    }

    #[test]
    fn find_files_never_returns_git_internals_or_protected_files() {
        let (r, w) = ws();
        tree(&r);
        let all = find_files(&w, &json!({"pattern": "*"}), &lim()).unwrap();
        for hidden in [".git", ".env", "server.pem"] {
            assert!(!all.contains(hidden), "{hidden} leaked: {all}");
        }
        assert_eq!(
            find_files(&w, &json!({"pattern": "*.pem"}), &lim()).unwrap(),
            "(no matches)"
        );
    }

    #[test]
    fn find_files_caps_results_and_says_so() {
        let (r, w) = ws();
        for i in 0..260 {
            write(&r, &format!("d/f{i:03}.log"), "");
        }
        let out = find_files(&w, &json!({"pattern": "*.log"}), &lim()).unwrap();
        assert_eq!(out.lines().count(), 201);
        assert!(out.ends_with("[truncated at 200 results; narrow the pattern or the path]"));
    }

    #[test]
    fn find_files_stops_at_the_examined_and_depth_limits() {
        let (r, w) = ws();
        for i in 0..30 {
            write(&r, &format!("f{i:02}.txt"), "");
        }
        let tight = Limits {
            examined: 10,
            ..lim()
        };
        let out = find_files(&w, &json!({"pattern": "*.txt"}), &tight).unwrap();
        assert!(
            out.contains("[stopped after examining 10 entries]"),
            "{out}"
        );
        assert_eq!(out.lines().count(), 11);

        let mut nest = String::new();
        for i in 0..8 {
            nest.push_str(&format!("d{i}/"));
        }
        write(&r, &format!("{nest}deep.txt"), "");
        let shallow = Limits { depth: 3, ..lim() };
        let out = find_files(&w, &json!({"pattern": "deep.txt"}), &shallow).unwrap();
        assert!(out.contains("(no matches)"), "{out}");
        assert!(out.contains("deeper than 3 levels"), "{out}");
    }

    #[cfg(unix)]
    #[test]
    fn walkers_do_not_follow_symlinks_out_of_the_workspace() {
        use std::os::unix::fs::symlink;
        let (r, w) = ws();
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("stolen.txt"), "needle").unwrap();
        symlink(outside.path(), r.path().join("portal")).unwrap();
        write(&r, "real.txt", "needle");
        assert_eq!(
            find_files(&w, &json!({"pattern": "*.txt"}), &lim()).unwrap(),
            "real.txt"
        );
        let hits = search_text(&w, &json!({"pattern": "needle"}), &lim()).unwrap();
        assert_eq!(hits, "real.txt:1: needle");
        assert!(search_text(&w, &json!({"pattern": "needle", "path": "portal"}), &lim()).is_err());
    }

    #[test]
    fn find_files_rejects_bad_globs_and_paths() {
        let (_r, w) = ws();
        assert!(matches!(
            find_files(&w, &json!({"pattern": "[a"}), &lim()),
            Err(ToolError::InvalidArguments(_))
        ));
        assert!(matches!(
            find_files(&w, &json!({}), &lim()),
            Err(ToolError::InvalidArguments(_))
        ));
        assert!(matches!(
            find_files(&w, &json!({"pattern": "*", "path": "../x"}), &lim()),
            Err(ToolError::InvalidPath(_))
        ));
        assert!(matches!(
            find_files(&w, &json!({"pattern": "*", "path": "nope"}), &lim()),
            Err(ToolError::NotFound(_))
        ));
    }

    // ---- search_text

    #[test]
    fn search_text_reports_path_line_and_text() {
        let (r, w) = ws();
        write(&r, "a.rs", "fn main() {}\nlet x = 1;\nfn other() {}\n");
        write(&r, "sub/b.rs", "fn deep() {}\n");
        let out = search_text(&w, &json!({"pattern": r"fn \w+"}), &lim()).unwrap();
        assert_eq!(
            out,
            "a.rs:1: fn main() {}\na.rs:3: fn other() {}\nsub/b.rs:1: fn deep() {}"
        );
        let one = search_text(&w, &json!({"pattern": "let", "path": "a.rs"}), &lim()).unwrap();
        assert_eq!(one, "a.rs:2: let x = 1;");
        let ci = search_text(
            &w,
            &json!({"pattern": "LET", "case_insensitive": true, "path": "a.rs"}),
            &lim(),
        )
        .unwrap();
        assert_eq!(ci, "a.rs:2: let x = 1;");
        assert_eq!(
            search_text(&w, &json!({"pattern": "zzz"}), &lim()).unwrap(),
            "(no matches)"
        );
    }

    #[test]
    fn search_text_limits_matches_per_file_and_in_total() {
        let (r, w) = ws();
        write(&r, "many.txt", &"hit\n".repeat(50));
        let per_file = search_text(&w, &json!({"pattern": "hit"}), &lim()).unwrap();
        assert_eq!(per_file.lines().count(), 10);
        for i in 0..40 {
            write(&r, &format!("f{i:02}.txt"), &"hit\n".repeat(10));
        }
        let total = search_text(&w, &json!({"pattern": "hit"}), &lim()).unwrap();
        assert_eq!(total.lines().count(), 201);
        assert!(total.ends_with("[truncated at 200 matches; narrow the pattern or the path]"));
    }

    #[test]
    fn search_text_cuts_long_lines_and_skips_binary_git_and_protected_files() {
        let (r, w) = ws();
        write(&r, "wide.txt", &format!("{}end\n", "w".repeat(1000)));
        write(&r, ".git/HEAD", "needle\n");
        write(&r, ".env", "needle=1\n");
        write(&r, "certs/a.pem", "needle\n");
        fs::write(r.path().join("bin.dat"), b"needle\0\0\0").unwrap();
        fs::write(r.path().join("latin1.txt"), [b'n', b'e', 0xe9, b'\n']).unwrap();
        write(&r, "ok.txt", "a needle here\n");
        let out = search_text(&w, &json!({"pattern": "needle"}), &lim()).unwrap();
        assert_eq!(out, "ok.txt:1: a needle here");
        let cut = search_text(&w, &json!({"pattern": "w+"}), &lim()).unwrap();
        let line = cut.lines().next().unwrap();
        assert!(line.ends_with('…'));
        assert_eq!(line.chars().count(), "wide.txt:1: ".len() + 300 + 1);
    }

    #[test]
    fn search_text_rejects_bad_patterns_and_runaway_regexes() {
        let (r, w) = ws();
        write(&r, "a.txt", "x\n");
        assert!(matches!(
            search_text(&w, &json!({"pattern": "("}), &lim()),
            Err(ToolError::InvalidArguments(_))
        ));
        assert!(matches!(
            search_text(&w, &json!({"pattern": "(a{1000}){1000}"}), &lim()),
            Err(ToolError::InvalidArguments(_))
        ));
        assert!(matches!(
            search_text(&w, &json!({}), &lim()),
            Err(ToolError::InvalidArguments(_))
        ));
        assert!(matches!(
            search_text(&w, &json!({"pattern": "x", "path": "../a"}), &lim()),
            Err(ToolError::InvalidPath(_))
        ));
        assert!(matches!(
            search_text(&w, &json!({"pattern": "x", "path": ".env"}), &lim()),
            Err(ToolError::Denied(_))
        ));
        assert!(matches!(
            search_text(
                &w,
                &json!({"pattern": "x", "case_insensitive": "yes"}),
                &lim()
            ),
            Err(ToolError::InvalidArguments(_))
        ));
    }

    #[test]
    fn search_text_caps_output_bytes() {
        let (r, w) = ws();
        for i in 0..30 {
            write(
                &r,
                &format!("f{i:02}.txt"),
                &format!("{}\n", format!("needle {}", "z".repeat(280)).repeat(1)).repeat(10),
            );
        }
        let out = search_text(&w, &json!({"pattern": "needle"}), &lim()).unwrap();
        assert!(out.len() <= 24 * 1024 + 200, "{}", out.len());
        assert!(out.contains("[truncated at "));
    }
}
