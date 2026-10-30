//! The read-only Files tools: `list_dir`, `read_file`, `find_files` and
//! `search_text`. Every result has deterministic limits and says so when it
//! was cut. Nothing here panics on odd input; failures are `ToolError`s.

use super::workspace::{is_sensitive, EntryKind, Stat, ToolError, Workspace};
use globset::GlobBuilder;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

/// How much the tools may read, list, search, write and run. Saved in
/// `roost.toml` under `[tools]`; any key may be left out. Tests pass smaller
/// ones for the big counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    /// Entries `list_dir` shows in one call.
    pub list_entries: usize,
    /// Lines `read_file` shows when no `limit` is given.
    pub read_default_lines: usize,
    /// Most lines `read_file` shows in one call.
    pub read_max_lines: usize,
    /// Bytes of output any one read, list or search returns.
    pub output_bytes: usize,
    /// Paths `find_files` returns in one call.
    pub find_results: usize,
    /// Folders deep `find_files` and `search_text` look.
    pub depth: usize,
    /// Entries `find_files` and `search_text` look at before stopping.
    pub examined: usize,
    /// Matches `search_text` returns in one call.
    pub search_matches: usize,
    /// Matches `search_text` shows from any one file.
    pub per_file_matches: usize,
    /// Characters of a line shown before it is cut.
    pub line_chars: usize,
    /// Largest text file `read_file` opens.
    pub read_file_bytes: u64,
    /// Largest file `search_text` looks inside.
    pub search_file_bytes: u64,
    /// Largest file `search_bytes` looks inside.
    pub bytes_file_bytes: u64,
    /// Most bytes `write_file` writes in one call.
    pub write_bytes: usize,
    /// Largest existing file `edit_file` and `write_file` may change.
    pub edit_file_bytes: usize,
    /// Bytes of undo copies kept for one chat.
    pub undo_bytes: usize,
    /// A command is stopped after this long.
    #[serde(rename = "run_timeout_secs", with = "secs")]
    pub run_timeout: std::time::Duration,
    /// A command's output is cut here.
    pub run_output_bytes: usize,
}

mod secs {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_secs(u64::deserialize(d)?))
    }
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
            bytes_file_bytes: 64 * 1024 * 1024,
            write_bytes: 256 * 1024,
            edit_file_bytes: 1024 * 1024,
            undo_bytes: 8 * 1024 * 1024,
            run_timeout: std::time::Duration::from_secs(120),
            run_output_bytes: 20 * 1024,
        }
    }
}

impl Limits {
    /// The limits kept inside a range that cannot break the tools or the
    /// model's context: nothing zero, nothing absurdly large. A value out of
    /// range is moved to the nearest end.
    pub fn clamped(mut self) -> Limits {
        const KB: usize = 1024;
        const MB: usize = 1024 * 1024;
        self.list_entries = self.list_entries.clamp(1, 5_000);
        self.read_default_lines = self.read_default_lines.clamp(1, 5_000);
        self.read_max_lines = self.read_max_lines.clamp(self.read_default_lines, 20_000);
        self.output_bytes = self.output_bytes.clamp(KB, MB);
        self.find_results = self.find_results.clamp(1, 5_000);
        self.depth = self.depth.clamp(1, 256);
        self.examined = self.examined.clamp(1_000, 5_000_000);
        self.search_matches = self.search_matches.clamp(1, 5_000);
        self.per_file_matches = self.per_file_matches.clamp(1, 1_000);
        self.line_chars = self.line_chars.clamp(40, 5_000);
        self.read_file_bytes = self.read_file_bytes.clamp(KB as u64, 256 * MB as u64);
        self.search_file_bytes = self.search_file_bytes.clamp(KB as u64, 256 * MB as u64);
        self.bytes_file_bytes = self.bytes_file_bytes.clamp(KB as u64, 1024 * MB as u64);
        self.write_bytes = self.write_bytes.clamp(KB, 4 * MB);
        self.edit_file_bytes = self.edit_file_bytes.clamp(KB, 8 * MB);
        self.undo_bytes = self.undo_bytes.clamp(MB, 256 * MB);
        self.run_timeout = self.run_timeout.clamp(
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(3_600),
        );
        self.run_output_bytes = self.run_output_bytes.clamp(KB, MB);
        self
    }
}

impl Limits {
    /// These limits for a model with a context window of `tokens`. One tool
    /// result may take at most about a sixth of the window (a token is about
    /// three bytes of text), so a small window gets smaller results and a run
    /// does not fail for lack of room. The setting stays the ceiling.
    pub fn for_window(&self, tokens: u32) -> Limits {
        let share = (tokens as usize / 2).max(1024);
        Limits {
            output_bytes: self.output_bytes.min(share),
            run_output_bytes: self.run_output_bytes.min(share),
            ..self.clone()
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

/// Where in a long result to start and how much to return.
struct Page {
    offset: usize,
    limit: usize,
    /// Said when the asked-for `limit` was more than the setting allows.
    note: Option<String>,
}

/// The `offset` and `limit` arguments; `cap` is the setting that `limit` may
/// not pass.
fn page_args(args: &Value, cap: usize) -> Result<Page, ToolError> {
    let offset = usize::try_from(uint_arg(args, "offset")?.unwrap_or(0)).unwrap_or(usize::MAX);
    let asked = uint_arg(args, "limit")?;
    if asked == Some(0) {
        return Err(ToolError::InvalidArguments(
            "limit must be at least 1".into(),
        ));
    }
    let asked = asked.map(|n| usize::try_from(n).unwrap_or(usize::MAX));
    let limit = asked.map_or(cap, |n| n.min(cap));
    let note = asked
        .filter(|n| *n > cap)
        .map(|n| format!("[limit {n} reduced to {cap}, the most one call returns]"));
    Ok(Page {
        offset,
        limit,
        note,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Name,
    Size,
    Modified,
}

/// The `sort` (name, size or modified) and `reverse` arguments. Size and
/// modified start with the largest and the newest.
fn sort_args(args: &Value) -> Result<(SortKey, bool), ToolError> {
    let key = match str_arg(args, "sort")?.as_deref() {
        None | Some("name") => SortKey::Name,
        Some("size") => SortKey::Size,
        Some("modified") => SortKey::Modified,
        Some(other) => {
            return Err(ToolError::InvalidArguments(format!(
                "sort must be name, size or modified, not {other:?}"
            )))
        }
    };
    Ok((key, bool_arg(args, "reverse")?))
}

/// Orders `items` by `key` (largest or newest first, names breaking ties),
/// or by name; `reverse` flips the result.
fn order_by<T>(items: &mut [(T, String, Stat)], key: SortKey, reverse: bool) {
    items.sort_by(|a, b| {
        let by = match key {
            SortKey::Name => a.1.cmp(&b.1),
            SortKey::Size => b.2.size.cmp(&a.2.size).then_with(|| a.1.cmp(&b.1)),
            SortKey::Modified => b.2.modified.cmp(&a.2.modified).then_with(|| a.1.cmp(&b.1)),
        };
        if reverse {
            by.reverse()
        } else {
            by
        }
    });
}

/// `3.4 KB`, `1.5 MB`: a size for a person, one decimal above a kilobyte.
fn size_label(n: u64) -> String {
    const KB: f64 = 1024.0;
    let f = n as f64;
    if f >= KB * KB * KB {
        format!("{:.1} GB", f / (KB * KB * KB))
    } else if f >= KB * KB {
        format!("{:.1} MB", f / (KB * KB))
    } else if f >= KB {
        format!("{:.1} KB", f / KB)
    } else {
        format!("{n} bytes")
    }
}

/// A time as `1971-01-01 00:00 UTC`.
fn utc_stamp(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}

/// What to print after a name when results are ordered by size or time.
fn sort_detail(key: SortKey, kind: EntryKind, st: &Stat) -> String {
    match key {
        SortKey::Name => String::new(),
        SortKey::Size if kind == EntryKind::Dir => String::new(),
        SortKey::Size => format!("  ({})", size_label(st.size)),
        SortKey::Modified => st
            .modified
            .map_or_else(String::new, |t| format!("  ({})", utc_stamp(t))),
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
    let page = page_args(args, lim.list_entries)?;
    let (key, reverse) = sort_args(args)?;
    let entries = ws.list(&rel)?;
    if entries.is_empty() {
        return Ok("(empty directory)".into());
    }
    if page.offset >= entries.len() {
        return Ok(format!(
            "(offset {} is past the end; the folder has {} entries)",
            page.offset,
            entries.len()
        ));
    }
    let total = entries.len();
    let mut items: Vec<_> = entries
        .into_iter()
        .map(|e| {
            let st = if key == SortKey::Name {
                Stat::default()
            } else {
                ws.stat(&rel.join(&e.name))
            };
            let name = e.name.clone();
            (e, name, st)
        })
        .collect();
    if key != SortKey::Name || reverse {
        order_by(&mut items, key, reverse);
    }
    let mut out = Out::new(lim.output_bytes);
    let mut shown = 0;
    for (e, _, st) in items.iter().skip(page.offset).take(page.limit) {
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
        let detail = sort_detail(key, e.kind, st);
        if !out.line(&format!("{}{mark}{protected}{detail}", e.name)) {
            break;
        }
        shown += 1;
    }
    let end = page.offset + shown;
    let mut notes: Vec<String> = page.note.into_iter().collect();
    if end < total {
        notes.push(format!(
            "[showing entries {}-{end} of {total}; call list_dir again with offset={end} to see the rest]",
            page.offset + 1
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
pub const BINARY_MAX_BYTES: u64 = 4096;
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
    let page = page_args(args, lim.find_results)?;
    let (key, reverse) = sort_args(args)?;
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
    let label = |path: &Path, kind: EntryKind| {
        format!(
            "{}{}",
            show(path),
            if kind == EntryKind::Dir { "/" } else { "" }
        )
    };
    let mut out = Out::new(lim.output_bytes);
    let mut seen = 0usize;
    let mut found = 0usize;
    let mut capped = false;
    let w = if key == SortKey::Name && !reverse {
        walk(ws, &start, lim, &mut |path, kind| {
            let rel_to_start = path.strip_prefix(&start).unwrap_or(path);
            if !glob.is_match(rel_to_start) {
                return true;
            }
            // Results before the offset were shown by an earlier call.
            if seen < page.offset {
                seen += 1;
                return true;
            }
            if found >= page.limit {
                capped = true;
                return false;
            }
            if !out.line(&label(path, kind)) {
                capped = true;
                return false;
            }
            found += 1;
            true
        })?
    } else {
        // Sorting needs every match first (bounded by the entries examined).
        let mut all: Vec<(EntryKind, String, Stat)> = Vec::new();
        let w = walk(ws, &start, lim, &mut |path, kind| {
            let rel_to_start = path.strip_prefix(&start).unwrap_or(path);
            if glob.is_match(rel_to_start) {
                let st = if key == SortKey::Name {
                    Stat::default()
                } else {
                    ws.stat(path)
                };
                all.push((kind, label(path, kind), st));
            }
            true
        })?;
        order_by(&mut all, key, reverse);
        let total = all.len();
        for (kind, name, st) in all.iter().skip(page.offset) {
            if found >= page.limit {
                capped = true;
                break;
            }
            if !out.line(&format!("{name}{}", sort_detail(key, *kind, st))) {
                capped = true;
                break;
            }
            found += 1;
        }
        let _ = total;
        w
    };
    let mut notes: Vec<String> = page.note.clone().into_iter().collect();
    if found == 0 && !capped {
        let none = if page.offset > 0 {
            format!("(no more results after offset {})", page.offset)
        } else {
            "(no matches)".to_string()
        };
        notes.insert(0, none);
        notes.extend(walk_notes(&w, lim));
        return Ok(out.finish(&notes));
    }
    if capped {
        let end = page.offset + found;
        notes.push(format!(
            "[showing results {}-{end}; there are more. Call find_files again with offset={end} for the next]",
            page.offset + 1
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

/// Why a file was not searched.
enum Skip {
    TooBig,
    Binary,
    NotText,
    Unreadable,
}

/// The matches in one file, at most `cap` of them, and whether the file had
/// more than that; or why the file was skipped.
#[allow(clippy::type_complexity)]
fn search_one(
    ws: &Workspace,
    rel: &Path,
    re: &regex::Regex,
    lim: &Limits,
    cap: usize,
) -> Result<(Vec<(usize, String)>, bool), Skip> {
    let (file, size) = ws.open_file(rel).map_err(|_| Skip::Unreadable)?;
    if size > lim.search_file_bytes {
        return Err(Skip::TooBig);
    }
    let mut reader = BufReader::new(file);
    let head = reader.fill_buf().map_err(|_| Skip::Unreadable)?;
    if head.iter().take(BINARY_SNIFF).any(|b| *b == 0) {
        return Err(Skip::Binary);
    }
    let mut found = Vec::new();
    let mut more = false;
    let mut buf = Vec::new();
    let mut n = 0usize;
    loop {
        let (read, _) = read_line(&mut reader, &mut buf).map_err(|_| Skip::Unreadable)?;
        if read == 0 {
            break;
        }
        n += 1;
        let Ok(text) = std::str::from_utf8(&buf) else {
            return Err(Skip::NotText);
        };
        let text = text.trim_end_matches(['\n', '\r']);
        if re.is_match(text) {
            if found.len() >= cap {
                more = true;
                break;
            }
            found.push((n, cut_line(text, lim.line_chars)));
        }
    }
    Ok((found, more))
}

/// A size as a person would say it: `2 MB`, `100 KB`, `100 bytes`.
fn human_bytes(n: u64) -> String {
    const MB: u64 = 1024 * 1024;
    if n >= MB && n.is_multiple_of(MB) {
        format!("{} MB", n / MB)
    } else if n >= 1024 && n.is_multiple_of(1024) {
        format!("{} KB", n / 1024)
    } else {
        format!("{n} bytes")
    }
}

/// Files `search_text` could not look inside, by reason, with a few names.
#[derive(Default)]
struct Skipped {
    too_big: usize,
    binary: usize,
    not_text: usize,
    unreadable: usize,
    examples: Vec<String>,
}

impl Skipped {
    fn add(&mut self, why: Skip, rel: &Path) {
        match why {
            Skip::TooBig => self.too_big += 1,
            Skip::Binary => self.binary += 1,
            Skip::NotText => self.not_text += 1,
            Skip::Unreadable => self.unreadable += 1,
        }
        if self.examples.len() < 3 {
            self.examples.push(show(rel));
        }
    }

    fn notes(&self, lim: &Limits) -> Vec<String> {
        let total = self.too_big + self.binary + self.not_text + self.unreadable;
        if total == 0 {
            return Vec::new();
        }
        let mut reasons = Vec::new();
        if self.too_big > 0 {
            reasons.push(format!(
                "{} larger than {}",
                self.too_big,
                human_bytes(lim.search_file_bytes)
            ));
        }
        if self.binary > 0 {
            reasons.push(format!("{} binary", self.binary));
        }
        if self.not_text > 0 {
            reasons.push(format!("{} not UTF-8 text", self.not_text));
        }
        if self.unreadable > 0 {
            reasons.push(format!("{} unreadable", self.unreadable));
        }
        let mut notes = vec![format!(
            "[skipped {total} file{}: {}; for example {}]",
            if total == 1 { "" } else { "s" },
            reasons.join(", "),
            self.examples.join(", ")
        )];
        if self.binary > 0 {
            notes.push("(read_binary can look inside binary files)".to_string());
        }
        notes
    }
}

pub fn search_text(ws: &Workspace, args: &Value, lim: &Limits) -> Result<String, ToolError> {
    let pattern = required_str(args, "pattern")?;
    let start = ws.resolve(&str_arg(args, "path")?.unwrap_or_default())?;
    let page = page_args(args, lim.search_matches)?;
    let offset = page.offset;
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(bool_arg(args, "case_insensitive")?)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| ToolError::InvalidArguments(format!("invalid regular expression: {e}")))?;
    let single = ws.list(&start).is_err();
    // Searching one named file is not held to the per-file limit.
    let per_file = if single {
        lim.search_matches
    } else {
        lim.per_file_matches
    };
    let mut out = Out::new(lim.output_bytes);
    let mut total = 0usize;
    let mut passed = 0usize;
    let mut capped = false;
    let mut skipped = Skipped::default();
    let mut cut_files: Vec<String> = Vec::new();
    let mut scan = |rel: &Path, out: &mut Out, total: &mut usize, capped: &mut bool| -> bool {
        let (found, more) = match search_one(ws, rel, &re, lim, per_file) {
            Ok(r) => r,
            Err(why) => {
                skipped.add(why, rel);
                return true;
            }
        };
        if more {
            cut_files.push(show(rel));
        }
        for (line, text) in found {
            // Matches before the offset were shown by an earlier call.
            if passed < offset {
                passed += 1;
                continue;
            }
            if *total >= page.limit {
                *capped = true;
                return false;
            }
            if !out.line(&format!("{}:{line}: {text}", show(rel))) {
                *capped = true;
                return false;
            }
            *total += 1;
        }
        true
    };
    let walked = if !single {
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
    let mut notes: Vec<String> = page.note.clone().into_iter().collect();
    if total == 0 && !capped {
        notes.push(if offset > 0 {
            format!("(no more matches after offset {offset})")
        } else {
            "(no matches)".to_string()
        });
    }
    if capped {
        let end = offset + total;
        notes.push(format!(
            "[showing matches {}-{end}; there are more. Call search_text again with offset={end} for the next]",
            offset + 1
        ));
    }
    match cut_files.as_slice() {
        [] => {}
        [one] => notes.push(format!(
            "[{one} had more matching lines than the {per_file} shown; search that file on its own to see all of them]"
        )),
        many => notes.push(format!(
            "[{} files had more matching lines than the {per_file} shown (for example {}); search a file on its own to see all of its matches]",
            many.len(),
            many.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        )),
    }
    notes.extend(skipped.notes(lim));
    notes.extend(walk_notes(&walked, lim));
    Ok(out.finish(&notes))
}

// -------------------------------------------------------------- search_bytes

/// Longest byte pattern `search_bytes` accepts.
const PATTERN_MAX: usize = 64;
/// How much of a file is read at a time.
const BYTES_CHUNK: usize = 64 * 1024;

/// Hex digits with optional spaces, and `??` for any byte: `4E 45 ?? 1A`.
fn byte_pattern(text: &str) -> Result<Vec<Option<u8>>, ToolError> {
    let bad = |why: &str| {
        ToolError::InvalidArguments(format!(
            "pattern {why}; write bytes as hex pairs such as \"4E 45 53 1A\", with ?? for any byte"
        ))
    };
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return Err(bad("is empty"));
    }
    if !compact.is_ascii() || !compact.len().is_multiple_of(2) {
        return Err(bad("is not whole hex pairs"));
    }
    let mut out = Vec::new();
    for pair in compact.as_bytes().chunks(2) {
        let pair = std::str::from_utf8(pair).unwrap_or("");
        if pair == "??" {
            out.push(None);
        } else {
            out.push(Some(
                u8::from_str_radix(pair, 16).map_err(|_| bad("has something that is not hex"))?,
            ));
        }
    }
    if out.len() > PATTERN_MAX {
        return Err(bad(&format!("is longer than {PATTERN_MAX} bytes")));
    }
    if out.iter().all(Option::is_none) {
        return Err(bad("matches every position"));
    }
    Ok(out)
}

/// A byte pattern found at an offset, with the bytes that matched.
type ByteHit = (u64, Vec<u8>);

/// Every match of `pat` in `file` as its offset and the bytes found, at most
/// `cap` of them, and whether there were more. Reads in chunks, so a file is never held whole.
fn find_bytes(
    mut file: impl Read,
    pat: &[Option<u8>],
    cap: usize,
) -> std::io::Result<(Vec<ByteHit>, bool)> {
    let mut hits = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    // File offset of buf[0].
    let mut base = 0u64;
    let mut chunk = vec![0u8; BYTES_CHUNK];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            return Ok((hits, false));
        }
        buf.extend_from_slice(&chunk[..n]);
        // Positions whose whole pattern is inside the buffer.
        let last_start = buf.len().saturating_sub(pat.len());
        if buf.len() >= pat.len() {
            for i in 0..=last_start {
                if pat
                    .iter()
                    .zip(&buf[i..])
                    .all(|(p, b)| p.is_none_or(|p| p == *b))
                {
                    if hits.len() >= cap {
                        return Ok((hits, true));
                    }
                    hits.push((base + i as u64, buf[i..i + pat.len()].to_vec()));
                }
            }
        }
        // Keep the tail that could start a match in the next chunk.
        let keep = (pat.len() - 1).min(buf.len());
        let drop = buf.len() - keep;
        base += drop as u64;
        buf.drain(..drop);
    }
}

pub fn search_bytes(ws: &Workspace, args: &Value, lim: &Limits) -> Result<String, ToolError> {
    let pat = byte_pattern(&required_str(args, "pattern")?)?;
    let start = ws.resolve(&str_arg(args, "path")?.unwrap_or_default())?;
    let page = page_args(args, lim.search_matches)?;
    let offset = page.offset;
    let name_glob = match str_arg(args, "glob")? {
        Some(g) if !g.is_empty() => Some(
            GlobBuilder::new(&g)
                .literal_separator(true)
                .build()
                .map_err(|e| ToolError::InvalidArguments(format!("invalid glob: {e}")))?
                .compile_matcher(),
        ),
        _ => None,
    };
    let single = ws.list(&start).is_err();
    let per_file = if single {
        lim.search_matches
    } else {
        lim.per_file_matches
    };
    let mut out = Out::new(lim.output_bytes);
    let mut total = 0usize;
    let mut passed = 0usize;
    let mut capped = false;
    let mut too_big = 0usize;
    let mut unreadable = 0usize;
    let mut examples: Vec<String> = Vec::new();
    let mut cut_files: Vec<String> = Vec::new();
    let mut scan = |rel: &Path, out: &mut Out, total: &mut usize, capped: &mut bool| -> bool {
        if let Some(g) = &name_glob {
            // A pattern without a slash matches the file name at any depth.
            let name = rel.file_name().map(Path::new).unwrap_or(rel);
            if !g.is_match(rel) && !g.is_match(name) {
                return true;
            }
        }
        let skip = |kind: &mut usize, examples: &mut Vec<String>| {
            *kind += 1;
            if examples.len() < 3 {
                examples.push(show(rel));
            }
        };
        let Ok((file, size)) = ws.open_file(rel) else {
            skip(&mut unreadable, &mut examples);
            return true;
        };
        if size > lim.bytes_file_bytes {
            skip(&mut too_big, &mut examples);
            return true;
        }
        let Ok((hits, more)) = find_bytes(file, &pat, per_file) else {
            skip(&mut unreadable, &mut examples);
            return true;
        };
        if more {
            cut_files.push(show(rel));
        }
        for (at, bytes) in hits {
            if passed < offset {
                passed += 1;
                continue;
            }
            if *total >= page.limit {
                *capped = true;
                return false;
            }
            let shown: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
            if !out.line(&format!("{}:0x{at:08x}: {}", show(rel), shown.join(" "))) {
                *capped = true;
                return false;
            }
            *total += 1;
        }
        true
    };
    let walked = if !single {
        walk(ws, &start, lim, &mut |path, kind| {
            kind != EntryKind::File || scan(path, &mut out, &mut total, &mut capped)
        })?
    } else {
        ws.open_file(&start)?;
        scan(&start, &mut out, &mut total, &mut capped);
        Walk {
            examined: 1,
            hit_examined: false,
            hit_depth: false,
        }
    };
    let mut notes: Vec<String> = page.note.clone().into_iter().collect();
    if total == 0 && !capped {
        notes.push(if offset > 0 {
            format!("(no more matches after offset {offset})")
        } else {
            "(no matches)".to_string()
        });
    }
    if capped {
        let end = offset + total;
        notes.push(format!(
            "[showing matches {}-{end}; there are more. Call search_bytes again with offset={end} for the next]",
            offset + 1
        ));
    }
    match cut_files.as_slice() {
        [] => {}
        [one] => notes.push(format!(
            "[{one} had more matches than the {per_file} shown; search that file on its own to see all of them]"
        )),
        many => notes.push(format!(
            "[{} files had more matches than the {per_file} shown (for example {}); search a file on its own to see all of its matches]",
            many.len(),
            many.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        )),
    }
    let skipped = too_big + unreadable;
    if skipped > 0 {
        let mut reasons = Vec::new();
        if too_big > 0 {
            reasons.push(format!(
                "{too_big} larger than {}",
                human_bytes(lim.bytes_file_bytes)
            ));
        }
        if unreadable > 0 {
            reasons.push(format!("{unreadable} unreadable"));
        }
        notes.push(format!(
            "[skipped {skipped} file{}: {}; for example {}]",
            if skipped == 1 { "" } else { "s" },
            reasons.join(", "),
            examples.join(", ")
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

    #[test]
    fn limits_in_a_table_override_only_what_it_names() {
        let l: Limits = toml::from_str("list_entries = 5\nrun_timeout_secs = 30\n").unwrap();
        assert_eq!(l.list_entries, 5);
        assert_eq!(l.run_timeout, std::time::Duration::from_secs(30));
        assert_eq!(l.search_matches, Limits::default().search_matches);
        let back: Limits = toml::from_str(&toml::to_string(&l).unwrap()).unwrap();
        assert_eq!(back, l);
    }

    #[test]
    fn absurd_limits_are_moved_into_a_range_the_tools_can_live_with() {
        let wild = Limits {
            list_entries: 0,
            read_default_lines: 0,
            read_max_lines: 0,
            output_bytes: 0,
            find_results: usize::MAX,
            depth: 0,
            examined: 0,
            search_matches: 0,
            per_file_matches: 0,
            line_chars: 0,
            read_file_bytes: 0,
            search_file_bytes: u64::MAX,
            bytes_file_bytes: 0,
            write_bytes: 0,
            edit_file_bytes: usize::MAX,
            undo_bytes: 0,
            run_timeout: std::time::Duration::ZERO,
            run_output_bytes: usize::MAX,
        }
        .clamped();
        assert!(wild.list_entries >= 1 && wild.depth >= 1 && wild.search_matches >= 1);
        assert!(wild.read_max_lines >= wild.read_default_lines);
        assert!(wild.output_bytes >= 1024 && wild.run_output_bytes <= 1024 * 1024);
        assert!(wild.find_results <= 5_000 && wild.edit_file_bytes <= 8 * 1024 * 1024);
        assert!(wild.search_file_bytes <= 256 * 1024 * 1024);
        assert!(wild.run_timeout >= std::time::Duration::from_secs(1));
        // Sensible values pass through untouched.
        assert_eq!(Limits::default().clamped(), Limits::default());
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
            out.ends_with(
                "[showing entries 1-200 of 251; call list_dir again with offset=200 to see the rest]"
            ),
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
    fn list_dir_pages_with_offset_until_the_end() {
        let (r, w) = ws();
        for i in 0..250 {
            write(&r, &format!("f{i:03}.txt"), "");
        }
        let second = list_dir(&w, &json!({"offset": 200}), &lim()).unwrap();
        assert_eq!(second.lines().next(), Some("f200.txt"));
        assert_eq!(second.lines().count(), 50, "no marker at the end: {second}");
        assert!(!second.contains("showing"), "{second}");
        let past = list_dir(&w, &json!({"offset": 900}), &lim()).unwrap();
        assert!(
            past.contains("past the end") && past.contains("250"),
            "{past}"
        );
    }

    #[test]
    fn find_files_pages_with_offset_and_the_pages_add_up() {
        let (r, w) = ws();
        for i in 0..250 {
            write(&r, &format!("d/f{i:03}.txt"), "");
        }
        let first = find_files(&w, &json!({"pattern": "*.txt"}), &lim()).unwrap();
        assert!(
            first.ends_with(
                "[showing results 1-200; there are more. Call find_files again with offset=200 for the next]"
            ),
            "{}",
            first.lines().last().unwrap()
        );
        let second = find_files(&w, &json!({"pattern": "*.txt", "offset": 200}), &lim()).unwrap();
        assert_eq!(second.lines().next(), Some("d/f200.txt"));
        assert_eq!(second.lines().count(), 50, "{second}");
        assert!(!second.contains("showing results"), "{second}");
    }

    #[test]
    fn search_text_pages_with_offset_and_says_when_a_file_had_more_than_shown() {
        let (r, w) = ws();
        for i in 0..30 {
            write(&r, &format!("f{i:02}.txt"), "needle\nneedle\n");
        }
        let small = Limits {
            search_matches: 25,
            ..lim()
        };
        let first = search_text(&w, &json!({"pattern": "needle"}), &small).unwrap();
        assert!(
            first.contains(
                "[showing matches 1-25; there are more. Call search_text again with offset=25 for the next]"
            ),
            "{first}"
        );
        let second = search_text(&w, &json!({"pattern": "needle", "offset": 25}), &lim()).unwrap();
        assert_eq!(second.lines().filter(|l| l.contains("needle")).count(), 35);
        // A file with more matching lines than the per-file limit says so.
        write(&r, "many.txt", &"needle\n".repeat(15));
        let per = search_text(
            &w,
            &json!({"pattern": "needle", "path": "many.txt"}),
            &lim(),
        )
        .unwrap();
        assert_eq!(
            per.lines().filter(|l| l.contains("needle")).count(),
            15,
            "a single file is not held to the per-file limit"
        );
        let all = search_text(
            &w,
            &json!({"pattern": "needle"}),
            &Limits {
                search_matches: 500,
                ..lim()
            },
        )
        .unwrap();
        assert!(
            all.contains("many.txt had more matching lines than the 10 shown"),
            "{all}"
        );
    }

    #[test]
    fn search_text_says_which_files_it_could_not_search_and_why() {
        let (r, w) = ws();
        write(&r, "a.txt", "needle");
        fs::write(r.path().join("rom.nes"), [0u8, 1, 2, 3, b'n', b'e']).unwrap();
        write(&r, "huge.txt", &"needle ".repeat(100));
        let tiny = Limits {
            search_file_bytes: 100,
            ..lim()
        };
        let out = search_text(&w, &json!({"pattern": "needle"}), &tiny).unwrap();
        assert!(out.contains("a.txt:1: needle"), "{out}");
        assert!(
            out.contains("[skipped 2 files: 1 binary, 1 larger than 100 bytes; for example rom.nes, huge.txt]")
                || out.contains("[skipped 2 files: 1 larger than 100 bytes, 1 binary; for example huge.txt, rom.nes]"),
            "{out}"
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
        assert!(out.ends_with(
            "[showing results 1-200; there are more. Call find_files again with offset=200 for the next]"
        ));
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
        assert_eq!(
            per_file
                .lines()
                .filter(|l| l.contains(":hit") || l.contains(": hit"))
                .count(),
            10
        );
        assert!(
            per_file.contains("many.txt had more matching lines than the 10 shown"),
            "{per_file}"
        );
        for i in 0..40 {
            write(&r, &format!("f{i:02}.txt"), &"hit\n".repeat(10));
        }
        let total = search_text(&w, &json!({"pattern": "hit"}), &lim()).unwrap();
        assert_eq!(total.lines().filter(|l| l.contains(": hit")).count(), 200);
        assert!(
            total.contains("[showing matches 1-200; there are more. Call search_text again with offset=200 for the next]"),
            "{total}"
        );
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
        // Only the readable file matched; the two it could not read are named.
        assert_eq!(out.lines().next(), Some("ok.txt:1: a needle here"));
        assert!(
            !out.contains(".git") && !out.contains(".env") && !out.contains("a.pem"),
            "{out}"
        );
        assert!(
            out.contains(
                "[skipped 2 files: 1 binary, 1 not UTF-8 text; for example bin.dat, latin1.txt]"
            ),
            "{out}"
        );
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
        assert!(out.contains("[showing matches 1-"), "{out}");
    }

    // ------------------------------------------------------------ search_bytes

    fn nes(extra: &[u8]) -> Vec<u8> {
        let mut v = vec![0x4e, 0x45, 0x53, 0x1a, 0x02, 0x01];
        v.extend_from_slice(extra);
        v
    }

    #[test]
    fn search_bytes_finds_a_pattern_across_files_with_offsets() {
        let (r, w) = ws();
        fs::create_dir_all(r.path().join("roms")).unwrap();
        fs::write(r.path().join("roms/a.nes"), nes(&[0; 8])).unwrap();
        fs::write(r.path().join("roms/b.nes"), nes(&[1; 8])).unwrap();
        fs::write(r.path().join("roms/c.bin"), [9u8; 32]).unwrap();
        let out = search_bytes(&w, &json!({"pattern": "4E 45 53 1A"}), &lim()).unwrap();
        assert_eq!(
            out,
            "roms/a.nes:0x00000000: 4e 45 53 1a\nroms/b.nes:0x00000000: 4e 45 53 1a"
        );
        let only =
            search_bytes(&w, &json!({"pattern": "4e45531a", "glob": "c.*"}), &lim()).unwrap();
        assert_eq!(only, "(no matches)");
    }

    #[test]
    fn search_bytes_wildcards_and_chunk_boundaries() {
        let (r, w) = ws();
        let mut data = vec![0u8; 70_000];
        // A pattern that straddles the 64 KiB read boundary.
        data[65_534..65_538].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        data[100..104].copy_from_slice(&[0xde, 0x00, 0xbe, 0xef]);
        fs::write(r.path().join("big.bin"), &data).unwrap();
        let out = search_bytes(
            &w,
            &json!({"pattern": "de ?? be ef", "path": "big.bin"}),
            &lim(),
        )
        .unwrap();
        assert_eq!(
            out,
            "big.bin:0x00000064: de 00 be ef\nbig.bin:0x0000fffe: de ad be ef"
        );
    }

    #[test]
    fn search_bytes_rejects_patterns_that_are_not_bytes() {
        let (_r, w) = ws();
        for bad in ["", "4", "4e 4", "zz", "4e-45", &"00 ".repeat(65)] {
            let e = search_bytes(&w, &json!({"pattern": bad}), &lim()).unwrap_err();
            assert!(
                matches!(e, ToolError::InvalidArguments(_)),
                "{bad:?}: {e:?}"
            );
        }
        assert!(
            search_bytes(&w, &json!({"pattern": "?? ??"}), &lim()).is_err(),
            "all wildcards matches everything"
        );
    }

    #[test]
    fn search_bytes_pages_and_notes_files_with_more_than_shown() {
        let (r, w) = ws();
        fs::write(r.path().join("a.bin"), vec![0xaa; 30]).unwrap();
        fs::write(r.path().join("b.bin"), vec![0xaa; 4]).unwrap();
        let small = Limits {
            search_matches: 12,
            ..lim()
        };
        let first = search_bytes(&w, &json!({"pattern": "aa"}), &small).unwrap();
        assert!(
            first.contains("a.bin had more matches than the 10 shown"),
            "{first}"
        );
        // a.bin shows 10, b.bin 2 of its 4 before the cap of 12 is reached.
        assert_eq!(
            first.lines().filter(|l| l.contains(": aa")).count(),
            12,
            "{first}"
        );
        assert!(first.contains("[showing matches 1-12; there are more. Call search_bytes again with offset=12 for the next]"), "{first}");
        let second = search_bytes(&w, &json!({"pattern": "aa", "offset": 12}), &lim()).unwrap();
        assert_eq!(
            second.lines().filter(|l| l.contains(": aa")).count(),
            2,
            "{second}"
        );
        // One named file is not held to the per-file limit.
        let single = search_bytes(&w, &json!({"pattern": "aa", "path": "a.bin"}), &lim()).unwrap();
        assert_eq!(single.lines().count(), 30);
    }

    #[test]
    fn search_bytes_says_which_files_were_too_large_or_protected_stays_unread() {
        let (r, w) = ws();
        fs::write(r.path().join("small.bin"), [1u8, 2, 3]).unwrap();
        fs::write(r.path().join("large.bin"), vec![1u8; 2000]).unwrap();
        fs::write(r.path().join(".env"), [1u8, 2, 3]).unwrap();
        let tiny = Limits {
            bytes_file_bytes: 1024,
            ..lim()
        };
        let out = search_bytes(&w, &json!({"pattern": "01 02 03"}), &tiny).unwrap();
        assert!(out.contains("small.bin:0x00000000: 01 02 03"), "{out}");
        assert!(!out.contains(".env"), "{out}");
        assert!(
            out.contains("[skipped 1 file: 1 larger than 1 KB; for example large.bin]"),
            "{out}"
        );
    }

    // ------------------------------------------------------- limit and sort

    fn sized(r: &TempDir, rel: &str, bytes: usize) {
        write(r, rel, &"x".repeat(bytes));
    }

    #[test]
    fn a_limit_argument_asks_for_fewer_and_cannot_pass_the_setting() {
        let (r, w) = ws();
        for i in 0..250 {
            write(&r, &format!("f{i:03}.txt"), "");
        }
        let few = list_dir(&w, &json!({"limit": 5}), &lim()).unwrap();
        assert_eq!(
            few.lines().filter(|l| l.ends_with(".txt")).count(),
            5,
            "{few}"
        );
        assert!(
            few.contains("[showing entries 1-5 of 250; call list_dir again with offset=5"),
            "{few}"
        );
        // Asking for more than the setting allows gives the setting, and says so.
        let many = list_dir(&w, &json!({"limit": 10_000}), &lim()).unwrap();
        assert_eq!(many.lines().filter(|l| l.ends_with(".txt")).count(), 200);
        assert!(
            many.contains("[limit 10000 reduced to 200, the most one call returns]"),
            "{many}"
        );
        for bad in [json!({"limit": 0}), json!({"limit": "ten"})] {
            assert!(matches!(
                list_dir(&w, &bad, &lim()),
                Err(ToolError::InvalidArguments(_))
            ));
        }
    }

    #[test]
    fn limit_applies_to_find_files_search_text_and_search_bytes() {
        let (r, w) = ws();
        for i in 0..20 {
            write(&r, &format!("d/f{i:02}.txt"), "needle\n");
        }
        let f = find_files(&w, &json!({"pattern": "*.txt", "limit": 3}), &lim()).unwrap();
        assert_eq!(f.lines().filter(|l| l.ends_with(".txt")).count(), 3, "{f}");
        assert!(f.contains("offset=3"), "{f}");
        let t = search_text(&w, &json!({"pattern": "needle", "limit": 4}), &lim()).unwrap();
        assert_eq!(t.lines().filter(|l| l.contains("needle")).count(), 4, "{t}");
        assert!(t.contains("offset=4"), "{t}");
        let b = search_bytes(&w, &json!({"pattern": "6e 65 65", "limit": 2}), &lim()).unwrap();
        assert_eq!(b.lines().filter(|l| l.contains(":0x")).count(), 2, "{b}");
        assert!(b.contains("offset=2"), "{b}");
    }

    #[test]
    fn list_dir_sorts_by_size_or_time_and_can_reverse() {
        let (r, w) = ws();
        sized(&r, "small.bin", 10);
        sized(&r, "big.bin", 5000);
        sized(&r, "mid.bin", 700);
        fs::create_dir(r.path().join("adir")).unwrap();
        let by_size = list_dir(&w, &json!({"sort": "size"}), &lim()).unwrap();
        let names: Vec<_> = by_size
            .lines()
            .map(|l| l.split_whitespace().next().unwrap())
            .collect();
        assert_eq!(
            names,
            ["big.bin", "mid.bin", "small.bin", "adir/"],
            "{by_size}"
        );
        assert!(by_size.contains("big.bin  (4.9 KB)"), "{by_size}");
        let smallest = list_dir(&w, &json!({"sort": "size", "reverse": true}), &lim()).unwrap();
        assert_eq!(
            smallest.lines().next().unwrap().split_whitespace().next(),
            Some("adir/")
        );
        // Newest first by modification time.
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86_400 * 365);
        fs::File::options()
            .write(true)
            .open(r.path().join("big.bin"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let by_time = list_dir(&w, &json!({"sort": "modified"}), &lim()).unwrap();
        assert_eq!(
            by_time.lines().last().unwrap().split_whitespace().next(),
            Some("big.bin"),
            "{by_time}"
        );
        assert!(by_time.contains("big.bin  (1971-01-01"), "{by_time}");
        // Paging follows the chosen order.
        let second = list_dir(
            &w,
            &json!({"sort": "size", "limit": 1, "offset": 1}),
            &lim(),
        )
        .unwrap();
        assert!(second.starts_with("mid.bin"), "{second}");
        assert!(matches!(
            list_dir(&w, &json!({"sort": "colour"}), &lim()),
            Err(ToolError::InvalidArguments(_))
        ));
    }

    #[test]
    fn find_files_can_return_the_largest_first_and_page_through_that_order() {
        let (r, w) = ws();
        sized(&r, "a/one.nes", 100);
        sized(&r, "b/two.nes", 900);
        sized(&r, "c/three.nes", 400);
        sized(&r, "c/skip.txt", 5000);
        let top = find_files(
            &w,
            &json!({"pattern": "*.nes", "sort": "size", "limit": 2}),
            &lim(),
        )
        .unwrap();
        let names: Vec<_> = top
            .lines()
            .filter(|l| l.contains(".nes"))
            .map(|l| l.split_whitespace().next().unwrap())
            .collect();
        assert_eq!(names, ["b/two.nes", "c/three.nes"], "{top}");
        assert!(top.contains("b/two.nes  (900 bytes)"), "{top}");
        assert!(top.contains("offset=2"), "{top}");
        let rest = find_files(
            &w,
            &json!({"pattern": "*.nes", "sort": "size", "limit": 2, "offset": 2}),
            &lim(),
        )
        .unwrap();
        assert!(rest.starts_with("a/one.nes"), "{rest}");
        assert!(!rest.contains("showing results"), "{rest}");
    }

    #[test]
    fn the_output_limit_follows_the_models_context_window() {
        let base = Limits::default();
        // A small window shrinks what one call may return; a big one keeps the setting.
        assert_eq!(base.for_window(4_096).output_bytes, 2_048);
        assert_eq!(base.for_window(8_192).output_bytes, 4_096);
        assert_eq!(base.for_window(131_072).output_bytes, base.output_bytes);
        assert_eq!(base.for_window(4_096).run_output_bytes, 2_048);
        // Never below 1 KB, and the other limits are not touched.
        assert_eq!(base.for_window(100).output_bytes, 1_024);
        assert_eq!(base.for_window(4_096).search_matches, base.search_matches);
        // A setting already lower than the share stays.
        let low = Limits {
            output_bytes: 1_500,
            ..base
        };
        assert_eq!(low.for_window(131_072).output_bytes, 1_500);
    }
}
