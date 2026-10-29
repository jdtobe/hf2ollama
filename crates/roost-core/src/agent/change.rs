//! Changes an agent makes to files: planned first (so the user can see a
//! diff before saying yes), then applied. A plan reads the file and works out
//! the new text; nothing is written until `apply`.

use super::workspace::{is_sensitive, ToolError, Workspace};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Most text a single write may carry.
pub const MAX_WRITE_BYTES: usize = 256 * 1024;
/// Files longer than this (in lines) get a coarse diff instead of an exact one.
const DIFF_LINE_LIMIT: usize = 2_000;
/// Changed lines shown in a preview, with two lines of context around them.
pub const PREVIEW_CHANGED_LINES: usize = 40;

/// A change to one file: the text before (`None` if the file is new) and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Workspace-relative path, as shown to the user.
    pub path: String,
    pub before: Option<String>,
    pub after: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineKind {
    Same,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

impl Change {
    /// Lines added and removed.
    pub fn stats(&self) -> (u32, u32) {
        let lines = diff(self.before.as_deref().unwrap_or(""), &self.after);
        let count = |k| lines.iter().filter(|l| l.kind == k).count() as u32;
        (count(LineKind::Added), count(LineKind::Removed))
    }

    /// The changed lines with two lines of context, at most
    /// `PREVIEW_CHANGED_LINES` changed ones; `more` says how many were left out.
    pub fn preview(&self) -> (Vec<DiffLine>, usize) {
        let all = diff(self.before.as_deref().unwrap_or(""), &self.after);
        let changed: Vec<usize> = all
            .iter()
            .enumerate()
            .filter(|(_, l)| l.kind != LineKind::Same)
            .map(|(i, _)| i)
            .collect();
        let mut keep = vec![false; all.len()];
        let mut shown = 0usize;
        for &i in &changed {
            if shown >= PREVIEW_CHANGED_LINES {
                break;
            }
            shown += 1;
            let from = i.saturating_sub(2);
            let to = (i + 3).min(all.len());
            keep[from..to].fill(true);
        }
        let lines = all
            .into_iter()
            .zip(keep)
            .filter(|(_, k)| *k)
            .map(|(l, _)| l)
            .collect();
        (lines, changed.len().saturating_sub(shown))
    }
}

/// What an approval card shows about a pending change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangePreview {
    pub path: String,
    /// The file does not exist yet.
    pub created: bool,
    pub added: u32,
    pub removed: u32,
    pub lines: Vec<DiffLine>,
    /// Changed lines not shown in `lines`.
    pub more: usize,
}

impl Change {
    pub fn card(&self) -> ChangePreview {
        let (added, removed) = self.stats();
        let (lines, more) = self.preview();
        ChangePreview {
            path: self.path.clone(),
            created: self.before.is_none(),
            added,
            removed,
            lines,
            more,
        }
    }
}

/// A stable fingerprint of text (FNV-1a), to notice a file changed since.
pub fn fingerprint(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A line diff. Exact for files up to `DIFF_LINE_LIMIT` lines; beyond that
/// the whole old text counts as removed and the whole new text as added.
pub fn diff(before: &str, after: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let line = |kind, text: &str| DiffLine {
        kind,
        text: text.to_string(),
    };
    if a.len() > DIFF_LINE_LIMIT || b.len() > DIFF_LINE_LIMIT {
        return a
            .iter()
            .map(|t| line(LineKind::Removed, t))
            .chain(b.iter().map(|t| line(LineKind::Added, t)))
            .collect();
    }
    // Longest common subsequence table, filled from the end.
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(n + m);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(line(LineKind::Same, a[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(line(LineKind::Removed, a[i]));
            i += 1;
        } else {
            out.push(line(LineKind::Added, b[j]));
            j += 1;
        }
    }
    while i < n {
        out.push(line(LineKind::Removed, a[i]));
        i += 1;
    }
    while j < m {
        out.push(line(LineKind::Added, b[j]));
        j += 1;
    }
    out
}

fn required(args: &Value, name: &str) -> Result<String, ToolError> {
    match args.get(name) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(ToolError::InvalidArguments(format!(
            "{name} must be a string"
        ))),
        None => Err(ToolError::InvalidArguments(format!("{name} is required"))),
    }
}

fn target(ws: &Workspace, path: &str) -> Result<std::path::PathBuf, ToolError> {
    let rel = ws.resolve(path)?;
    if rel.as_os_str().is_empty() {
        return Err(ToolError::InvalidPath(
            "give a file path, not the folder".into(),
        ));
    }
    if is_sensitive(&rel) {
        return Err(ToolError::Denied(format!(
            "{} is protected and cannot be written",
            super::tools::show(&rel)
        )));
    }
    Ok(rel)
}

/// Works out the change a write tool call would make. `Ok(None)` when `name`
/// is not a write tool.
pub fn plan(ws: &Workspace, name: &str, args: &Value) -> Result<Option<Change>, ToolError> {
    match name {
        "write_file" => {
            let path = required(args, "path")?;
            let content = required(args, "content")?;
            if content.len() > MAX_WRITE_BYTES {
                return Err(ToolError::InvalidArguments(format!(
                    "content is {} bytes; one write may carry at most {}",
                    content.len(),
                    MAX_WRITE_BYTES
                )));
            }
            let rel = target(ws, &path)?;
            let before = ws.read_text_for_edit(&rel)?;
            Ok(Some(Change {
                path: super::tools::show(&rel),
                before,
                after: content,
            }))
        }
        "edit_file" => {
            let path = required(args, "path")?;
            let old = required(args, "old_text")?;
            let new = required(args, "new_text")?;
            if old.is_empty() {
                return Err(ToolError::InvalidArguments(
                    "old_text must not be empty; use write_file to create a file".into(),
                ));
            }
            let rel = target(ws, &path)?;
            let Some(before) = ws.read_text_for_edit(&rel)? else {
                return Err(ToolError::NotFound(super::tools::show(&rel)));
            };
            let found = before.matches(&old).count();
            if found != 1 {
                return Err(ToolError::InvalidArguments(if found == 0 {
                    "old_text was not found in the file; read it again and copy the text exactly"
                        .to_string()
                } else {
                    format!(
                        "old_text appears {found} times; include more of the surrounding text so it matches once"
                    )
                }));
            }
            let after = before.replacen(&old, &new, 1);
            if after.len() > MAX_WRITE_BYTES * 4 {
                return Err(ToolError::InvalidArguments(
                    "the result would be too large".into(),
                ));
            }
            Ok(Some(Change {
                path: super::tools::show(&rel),
                before: Some(before),
                after,
            }))
        }
        _ => Ok(None),
    }
}

/// Writes a planned change and describes it for the model.
pub fn apply(ws: &Workspace, change: &Change) -> Result<String, ToolError> {
    ws.write_text(Path::new(&change.path), &change.after)?;
    let (added, removed) = change.stats();
    let what = if change.before.is_some() {
        "Changed"
    } else {
        "Created"
    };
    Ok(format!(
        "{what} {} (+{added} -{removed} lines).",
        change.path
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn ws() -> (TempDir, Workspace) {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        let w = Workspace::open(root.path()).unwrap();
        (root, w)
    }

    #[test]
    fn a_diff_marks_added_removed_and_unchanged_lines() {
        let d = diff("a\nb\nc\n", "a\nB\nc\nd\n");
        let kinds: Vec<_> = d.iter().map(|l| (l.kind, l.text.as_str())).collect();
        assert_eq!(
            kinds,
            [
                (LineKind::Same, "a"),
                (LineKind::Removed, "b"),
                (LineKind::Added, "B"),
                (LineKind::Same, "c"),
                (LineKind::Added, "d"),
            ]
        );
        let c = Change {
            path: "f".into(),
            before: Some("a\nb\nc\n".into()),
            after: "a\nB\nc\nd\n".into(),
        };
        assert_eq!(c.stats(), (2, 1));
    }

    #[test]
    fn a_huge_file_gets_a_coarse_diff_not_a_slow_one() {
        let big: String = (0..DIFF_LINE_LIMIT + 5)
            .map(|i| format!("l{i}\n"))
            .collect();
        let d = diff(&big, "x\n");
        assert_eq!(
            d.iter().filter(|l| l.kind == LineKind::Removed).count(),
            DIFF_LINE_LIMIT + 5
        );
        assert_eq!(d.iter().filter(|l| l.kind == LineKind::Added).count(), 1);
    }

    #[test]
    fn a_preview_keeps_context_and_says_how_much_it_left_out() {
        let before: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let after: String = (0..200).map(|i| format!("LINE {i}\n")).collect();
        let c = Change {
            path: "f".into(),
            before: Some(before),
            after,
        };
        let (lines, more) = c.preview();
        let changed = lines.iter().filter(|l| l.kind != LineKind::Same).count();
        assert!(changed <= PREVIEW_CHANGED_LINES + 2, "{changed}");
        assert!(more > 300, "{more} changed lines left out");
    }

    #[test]
    fn write_file_plans_a_new_file_and_an_overwrite() {
        let (_r, w) = ws();
        let new = plan(
            &w,
            "write_file",
            &json!({"path": "sub/new.txt", "content": "hi\n"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(new.before, None);
        assert_eq!(new.after, "hi\n");
        assert_eq!(new.path, "sub/new.txt");
        let over = plan(
            &w,
            "write_file",
            &json!({"path": "a.txt", "content": "x\n"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(over.before.as_deref(), Some("one\ntwo\nthree\n"));
    }

    #[test]
    fn write_file_refuses_protected_names_big_content_and_the_folder_itself() {
        let (_r, w) = ws();
        for path in [".env", "config/secrets.json", "id.pem", ".ssh/config"] {
            let e = plan(&w, "write_file", &json!({"path": path, "content": "x"})).unwrap_err();
            assert!(matches!(e, ToolError::Denied(_)), "{path}: {e:?}");
        }
        let big = "x".repeat(MAX_WRITE_BYTES + 1);
        assert!(matches!(
            plan(&w, "write_file", &json!({"path": "b.txt", "content": big})).unwrap_err(),
            ToolError::InvalidArguments(_)
        ));
        assert!(plan(&w, "write_file", &json!({"path": "", "content": "x"})).is_err());
        assert!(plan(&w, "write_file", &json!({"path": "../x", "content": "x"})).is_err());
        assert!(
            plan(&w, "write_file", &json!({"path": "a.txt"})).is_err(),
            "content is required"
        );
    }

    #[test]
    fn edit_file_replaces_exactly_one_match() {
        let (_r, w) = ws();
        let c = plan(
            &w,
            "edit_file",
            &json!({"path": "a.txt", "old_text": "two", "new_text": "2"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(c.after, "one\n2\nthree\n");
        let none = plan(
            &w,
            "edit_file",
            &json!({"path": "a.txt", "old_text": "zzz", "new_text": "2"}),
        )
        .unwrap_err();
        assert!(none.to_string().contains("not found"), "{none}");
        fs::write(w.root().join("dup.txt"), "x x").unwrap();
        let many = plan(
            &w,
            "edit_file",
            &json!({"path": "dup.txt", "old_text": "x", "new_text": "y"}),
        )
        .unwrap_err();
        assert!(many.to_string().contains("2 times"), "{many}");
        let missing = plan(
            &w,
            "edit_file",
            &json!({"path": "gone.txt", "old_text": "x", "new_text": "y"}),
        )
        .unwrap_err();
        assert!(matches!(missing, ToolError::NotFound(_)));
    }

    #[test]
    fn other_tools_have_no_plan() {
        let (_r, w) = ws();
        assert_eq!(
            plan(&w, "read_file", &json!({"path": "a.txt"})).unwrap(),
            None
        );
    }

    #[test]
    fn applying_a_plan_writes_the_file_and_reports_it() {
        let (r, w) = ws();
        let c = plan(
            &w,
            "write_file",
            &json!({"path": "deep/er/new.txt", "content": "a\nb\n"}),
        )
        .unwrap()
        .unwrap();
        let msg = apply(&w, &c).unwrap();
        assert_eq!(msg, "Created deep/er/new.txt (+2 -0 lines).");
        assert_eq!(
            fs::read_to_string(r.path().join("deep/er/new.txt")).unwrap(),
            "a\nb\n"
        );
        let c = plan(
            &w,
            "write_file",
            &json!({"path": "a.txt", "content": "one\nTWO\nthree\n"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(apply(&w, &c).unwrap(), "Changed a.txt (+1 -1 lines).");
    }

    #[test]
    fn the_fingerprint_changes_with_the_text() {
        assert_eq!(fingerprint("abc"), fingerprint("abc"));
        assert_ne!(fingerprint("abc"), fingerprint("abd"));
    }
}
