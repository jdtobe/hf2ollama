//! On-disk session store: one JSON file per session plus a derived index.
//!
//! The index (`index-v1.json`) holds metadata only and is rebuilt from the
//! session files whenever it is missing, unreadable or stale, so a crash
//! between the two writes loses nothing. Files that cannot be understood are
//! never deleted: a damaged file is renamed aside, a file from a newer Roost
//! is left untouched, and a transient read error changes nothing.

use crate::persistence::atomic_write;
use crate::session::{
    BranchOrigin, MessageContent, Session, SessionDocument, SessionId, SessionLifetime,
    SessionMode, SESSION_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

mod writer;
pub use writer::Writer;

const INDEX_FILE: &str = "index-v1.json";
const INDEX_VERSION: u32 = 1;
const LOCK_FILE: &str = ".lock";

/// Where a deleted session waits for an undo. Whatever is still here at the
/// next start is removed, so deleting still means gone.
const TRASH_DIR: &str = "trash";
const CORRUPT_INFIX: &str = ".json.corrupt-";
const PLACEHOLDER_TITLE: &str = "Unreadable session";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RowStatus {
    Ok,
    Quarantined,
    NewerSchema,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexRow {
    pub id: SessionId,
    pub title: String,
    pub mode: SessionMode,
    pub lifetime: SessionLifetime,
    pub pinned: bool,
    pub model: String,
    pub parent: Option<BranchOrigin>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    /// A run was in progress when this row was written.
    pub active: bool,
    pub status: RowStatus,
    pub file_mtime_ms: u64,
    pub file_len: u64,
    /// The start of the first user message on one line, for chats that have
    /// no title yet. `None` means an index written before previews existed.
    #[serde(default)]
    pub preview: Option<String>,
}

const PREVIEW_CHARS: usize = 80;

/// The first user message squeezed onto one line and cut at 80 characters.
fn preview_of(s: &Session) -> String {
    let first = s.messages.iter().find_map(|m| match &m.content {
        MessageContent::User { text, .. } => Some(text),
        _ => None,
    });
    let line = first
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    if line.chars().count() <= PREVIEW_CHARS {
        return line;
    }
    let cut: String = line.chars().take(PREVIEW_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

impl IndexRow {
    /// Equal apart from the file stamp, which streaming flushes change.
    fn same_content(&self, other: &IndexRow) -> bool {
        let strip = |r: &IndexRow| IndexRow {
            file_mtime_ms: 0,
            file_len: 0,
            ..r.clone()
        };
        strip(self) == strip(other)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreWarning {
    IndexRebuilt,
    Quarantined(SessionId),
    NewerSchema(SessionId),
    Unreadable(SessionId),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("another Roost is using this history")]
    Locked,
    #[error("history storage error: {0}")]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    #[error("session not found")]
    Missing,
    #[error("this session file is damaged and was set aside")]
    Quarantined,
    #[error("this session was made by a newer Roost")]
    NewerSchema,
    #[error("this session file could not be read: {0}")]
    Unreadable(String),
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    version: u32,
    rows: Vec<IndexRow>,
}

#[derive(Serialize)]
struct DocRef<'a> {
    schema_version: u32,
    session: &'a Session,
}

#[derive(Clone, Copy)]
struct Stamp {
    mtime_ms: u64,
    len: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let m = fs::metadata(path).ok()?;
    let mtime_ms = m
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some(Stamp {
        mtime_ms,
        len: m.len(),
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

enum Parsed {
    Ok(Box<Session>),
    Newer(Option<String>),
    Corrupt,
    Unreadable(String),
}

fn parse_file(path: &Path, id: SessionId) -> Parsed {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => return Parsed::Unreadable(e.to_string()),
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Parsed::Corrupt;
    };
    match value.get("schema_version").and_then(|v| v.as_u64()) {
        Some(n) if n > u64::from(SESSION_SCHEMA_VERSION) => {
            let title = value
                .pointer("/session/title")
                .and_then(|t| t.as_str())
                .map(str::to_string);
            return Parsed::Newer(title);
        }
        None => return Parsed::Corrupt,
        Some(_) => {}
    }
    let Ok(doc) = serde_json::from_value::<SessionDocument>(value) else {
        return Parsed::Corrupt;
    };
    match doc.into_current() {
        Ok(s) if s.id == id => Parsed::Ok(Box::new(s)),
        _ => Parsed::Corrupt,
    }
}

fn row_of(s: &Session, st: Stamp) -> IndexRow {
    IndexRow {
        id: s.id,
        title: s.title.clone(),
        mode: s.mode,
        lifetime: s.lifetime,
        pinned: s.pinned,
        model: s.model.clone(),
        parent: s.parent,
        created_at_ms: s.created_at_ms,
        updated_at_ms: s.updated_at_ms,
        active: !s.is_idle(),
        status: RowStatus::Ok,
        file_mtime_ms: st.mtime_ms,
        file_len: st.len,
        preview: Some(preview_of(s)),
    }
}

/// The index row for a session that is not (or not only) on disk.
pub(crate) fn live_row(s: &Session) -> IndexRow {
    row_of(
        s,
        Stamp {
            mtime_ms: 0,
            len: 0,
        },
    )
}

fn placeholder(id: SessionId, status: RowStatus, title: Option<String>, st: Stamp) -> IndexRow {
    IndexRow {
        id,
        title: title.unwrap_or_else(|| PLACEHOLDER_TITLE.into()),
        mode: SessionMode::Chat,
        lifetime: SessionLifetime::Persisted,
        pinned: false,
        model: String::new(),
        parent: None,
        created_at_ms: st.mtime_ms,
        updated_at_ms: st.mtime_ms,
        active: false,
        status,
        file_mtime_ms: st.mtime_ms,
        file_len: st.len,
        preview: Some(String::new()),
    }
}

pub struct Store {
    dir: PathBuf,
    rows: BTreeMap<SessionId, IndexRow>,
    _lock: File,
}

fn io<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError::Io(e.to_string())
}

impl Store {
    fn path(&self, id: SessionId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Opens (creating if needed) the store in `dir`, takes its lock and
    /// reconciles the index with the files. Sessions left mid-run by a crash
    /// are settled here, once.
    pub fn open(dir: &Path) -> Result<(Store, Vec<StoreWarning>), StoreError> {
        fs::create_dir_all(dir).map_err(io)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(LOCK_FILE))
            .map_err(io)?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(StoreError::Locked),
            Err(TryLockError::Error(e)) => return Err(io(e)),
        }
        let _ = fs::remove_dir_all(dir.join(TRASH_DIR));
        let mut store = Store {
            dir: dir.to_path_buf(),
            rows: BTreeMap::new(),
            _lock: lock,
        };
        let mut warnings = Vec::new();
        let index_path = dir.join(INDEX_FILE);
        let index_existed = index_path.exists();
        let old: Option<BTreeMap<SessionId, IndexRow>> = fs::read(&index_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<IndexFile>(&b).ok())
            .filter(|f| f.version == INDEX_VERSION)
            .map(|f| f.rows.into_iter().map(|r| (r.id, r)).collect());
        let index_ok = old.is_some();
        let old = old.unwrap_or_default();

        let mut live: BTreeMap<SessionId, Stamp> = BTreeMap::new();
        let mut aside: BTreeMap<SessionId, Stamp> = BTreeMap::new();
        for entry in fs::read_dir(dir).map_err(io)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(st) = stamp(&entry.path()) else {
                continue;
            };
            if let Some(stem) = name.strip_suffix(".json") {
                if let Ok(id) = stem.parse::<SessionId>() {
                    live.insert(id, st);
                }
            } else if let Some((stem, _)) = name.split_once(CORRUPT_INFIX) {
                if let Ok(id) = stem.parse::<SessionId>() {
                    aside.insert(id, st);
                }
            }
        }

        let mut changed = !index_ok && (index_existed || !live.is_empty() || !aside.is_empty());
        if changed && index_existed {
            warnings.push(StoreWarning::IndexRebuilt);
        }
        for (id, st) in &live {
            let reusable = old.get(id).filter(|r| {
                r.status == RowStatus::Ok
                    && r.preview.is_some()
                    && !r.active
                    && r.file_mtime_ms == st.mtime_ms
                    && r.file_len == st.len
            });
            if let Some(r) = reusable {
                store.rows.insert(*id, r.clone());
                continue;
            }
            changed = true;
            let row = store.classify(*id, &mut warnings);
            store.rows.insert(*id, row);
        }
        for (id, st) in &aside {
            if live.contains_key(id) {
                continue;
            }
            match old.get(id) {
                Some(r) if r.status == RowStatus::Quarantined => {
                    store.rows.insert(*id, r.clone());
                }
                _ => {
                    changed = true;
                    store
                        .rows
                        .insert(*id, placeholder(*id, RowStatus::Quarantined, None, *st));
                }
            }
        }
        if old.keys().any(|id| !store.rows.contains_key(id)) {
            changed = true;
        }
        if changed {
            store.write_index()?;
        }
        Ok((store, warnings))
    }

    /// Reads one file at startup and decides what it is; may rename a damaged
    /// file aside and settles a crash-left session.
    fn classify(&self, id: SessionId, warnings: &mut Vec<StoreWarning>) -> IndexRow {
        let path = self.path(id);
        let st = stamp(&path).unwrap_or(Stamp {
            mtime_ms: 0,
            len: 0,
        });
        match parse_file(&path, id) {
            Parsed::Ok(mut s) => {
                if !s.is_idle() && s.normalize_after_crash() {
                    if let Ok(bytes) = Self::encode(&s) {
                        let _ = atomic_write(&path, &bytes);
                    }
                }
                row_of(&s, stamp(&path).unwrap_or(st))
            }
            Parsed::Newer(title) => {
                warnings.push(StoreWarning::NewerSchema(id));
                placeholder(id, RowStatus::NewerSchema, title, st)
            }
            Parsed::Unreadable(_) => {
                warnings.push(StoreWarning::Unreadable(id));
                placeholder(id, RowStatus::Unreadable, None, st)
            }
            Parsed::Corrupt => {
                warnings.push(StoreWarning::Quarantined(id));
                let aside = self.dir.join(format!("{id}{CORRUPT_INFIX}{}", now_ms()));
                match fs::rename(&path, &aside) {
                    Ok(()) => {
                        let st = stamp(&aside).unwrap_or(st);
                        placeholder(id, RowStatus::Quarantined, None, st)
                    }
                    Err(_) => placeholder(id, RowStatus::Unreadable, None, st),
                }
            }
        }
    }

    fn encode(s: &Session) -> Result<Vec<u8>, StoreError> {
        serde_json::to_vec_pretty(&DocRef {
            schema_version: SESSION_SCHEMA_VERSION,
            session: s,
        })
        .map_err(io)
    }

    fn write_index(&self) -> Result<(), StoreError> {
        let file = IndexFile {
            version: INDEX_VERSION,
            rows: self.rows.values().cloned().collect(),
        };
        let bytes = serde_json::to_vec_pretty(&file).map_err(io)?;
        atomic_write(&self.dir.join(INDEX_FILE), &bytes).map_err(io)
    }

    /// Newest activity first.
    pub fn rows(&self) -> Vec<IndexRow> {
        let mut v: Vec<IndexRow> = self.rows.values().cloned().collect();
        v.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        v
    }

    pub fn load(&mut self, id: SessionId) -> Result<Session, LoadError> {
        if let Some(r) = self.rows.get(&id) {
            match r.status {
                RowStatus::Quarantined => return Err(LoadError::Quarantined),
                RowStatus::NewerSchema => return Err(LoadError::NewerSchema),
                _ => {}
            }
        }
        let path = self.path(id);
        if !path.exists() {
            return Err(LoadError::Missing);
        }
        match parse_file(&path, id) {
            Parsed::Ok(s) => Ok(*s),
            Parsed::Newer(_) => Err(LoadError::NewerSchema),
            Parsed::Unreadable(e) => Err(LoadError::Unreadable(e)),
            Parsed::Corrupt => {
                let mut w = Vec::new();
                let row = self.classify(id, &mut w);
                self.rows.insert(id, row);
                let _ = self.write_index();
                Err(LoadError::Quarantined)
            }
        }
    }

    /// Writes the session file, then the index when the row's content changed
    /// (mid-stream flushes only change the file stamp and skip the index).
    pub fn save(&mut self, s: &Session) -> Result<(), StoreError> {
        let path = self.path(s.id);
        atomic_write(&path, &Self::encode(s)?).map_err(io)?;
        let st = stamp(&path).ok_or_else(|| StoreError::Io("session file vanished".into()))?;
        let row = row_of(s, st);
        let unchanged = self
            .rows
            .get(&s.id)
            .is_some_and(|old| old.same_content(&row));
        self.rows.insert(s.id, row);
        if !unchanged {
            self.write_index()?;
        }
        Ok(())
    }

    fn trash_path(&self, id: SessionId) -> PathBuf {
        self.dir.join(TRASH_DIR).join(format!("{id}.json"))
    }

    /// Moves the session file into the trash, where it waits for `restore` or
    /// `purge`, and removes any set-aside copies and the row. A file that
    /// cannot be moved is removed instead, so a delete always deletes.
    pub fn delete(&mut self, id: SessionId) -> Result<(), StoreError> {
        let from = self.path(id);
        let to = self.trash_path(id);
        let moved = fs::create_dir_all(self.dir.join(TRASH_DIR))
            .and_then(|()| fs::rename(&from, &to))
            .is_ok();
        if !moved {
            let _ = fs::remove_file(&from);
        }
        if let Ok(entries) = fs::read_dir(&self.dir) {
            let prefix = format!("{id}{CORRUPT_INFIX}");
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = fs::remove_file(e.path());
                }
            }
        }
        if self.rows.remove(&id).is_some() {
            self.write_index()?;
        }
        Ok(())
    }

    /// Brings a deleted session back from the trash. An error when it is not
    /// there any more (purged, or never kept).
    pub fn restore(&mut self, id: SessionId) -> Result<(), StoreError> {
        let to = self.path(id);
        if to.exists() {
            return Ok(());
        }
        fs::rename(self.trash_path(id), &to).map_err(io)?;
        let mut warnings = Vec::new();
        let row = self.classify(id, &mut warnings);
        self.rows.insert(id, row);
        self.write_index()
    }

    /// Removes a deleted session from the trash for good.
    pub fn purge(&mut self, id: SessionId) {
        let _ = fs::remove_file(self.trash_path(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{AssistantStatus, RunId, TurnId};
    use tempfile::TempDir;

    fn chat(title: &str) -> Session {
        let mut s = Session::new_chat("m", SessionLifetime::Persisted, 1_000);
        s.rename(title).unwrap();
        s
    }

    fn open(dir: &Path) -> (Store, Vec<StoreWarning>) {
        Store::open(dir).unwrap()
    }

    fn file_of(dir: &Path, id: SessionId) -> PathBuf {
        dir.join(format!("{id}.json"))
    }

    #[test]
    fn save_then_load_roundtrips() {
        let d = TempDir::new().unwrap();
        let (mut st, _) = open(d.path());
        let mut s = chat("One");
        s.push_user("hello", 2_000);
        st.save(&s).unwrap();
        assert_eq!(st.load(s.id).unwrap(), s);
        assert_eq!(st.rows()[0].title, "One");
    }

    #[test]
    fn saving_one_session_does_not_touch_other_files() {
        let d = TempDir::new().unwrap();
        let (mut st, _) = open(d.path());
        let (a, b) = (chat("A"), chat("B"));
        st.save(&a).unwrap();
        st.save(&b).unwrap();
        let before = fs::read(file_of(d.path(), a.id)).unwrap();
        let mtime = stamp(&file_of(d.path(), a.id)).unwrap().mtime_ms;
        let mut b2 = b.clone();
        b2.push_user("more", 5_000);
        st.save(&b2).unwrap();
        assert_eq!(fs::read(file_of(d.path(), a.id)).unwrap(), before);
        assert_eq!(stamp(&file_of(d.path(), a.id)).unwrap().mtime_ms, mtime);
    }

    #[test]
    fn missing_index_is_rebuilt_from_files() {
        let d = TempDir::new().unwrap();
        let s = chat("Kept");
        {
            let (mut st, _) = open(d.path());
            st.save(&s).unwrap();
        }
        fs::remove_file(d.path().join(INDEX_FILE)).unwrap();
        let (st, _) = open(d.path());
        assert_eq!(st.rows().len(), 1);
        assert_eq!(st.rows()[0].title, "Kept");
        assert!(d.path().join(INDEX_FILE).exists());
    }

    #[test]
    fn unsupported_index_version_is_rebuilt_with_a_warning() {
        let d = TempDir::new().unwrap();
        let s = chat("Kept");
        {
            let (mut st, _) = open(d.path());
            st.save(&s).unwrap();
        }
        fs::write(d.path().join(INDEX_FILE), br#"{"version":99,"rows":[]}"#).unwrap();
        let (st, w) = open(d.path());
        assert!(w.contains(&StoreWarning::IndexRebuilt));
        assert_eq!(st.rows().len(), 1);
    }

    #[test]
    fn stale_index_row_is_refreshed_by_file_stamp() {
        let d = TempDir::new().unwrap();
        let mut s = chat("Old");
        {
            let (mut st, _) = open(d.path());
            st.save(&s).unwrap();
        }
        s.rename("A much longer new title").unwrap();
        atomic_write(&file_of(d.path(), s.id), &Store::encode(&s).unwrap()).unwrap();
        let (st, _) = open(d.path());
        assert_eq!(st.rows()[0].title, "A much longer new title");
    }

    #[test]
    fn row_without_a_file_is_dropped() {
        let d = TempDir::new().unwrap();
        let s = chat("Gone");
        {
            let (mut st, _) = open(d.path());
            st.save(&s).unwrap();
        }
        fs::remove_file(file_of(d.path(), s.id)).unwrap();
        let (st, _) = open(d.path());
        assert!(st.rows().is_empty());
    }

    #[test]
    fn a_corrupt_file_is_quarantined_and_others_still_list() {
        let d = TempDir::new().unwrap();
        let (good, bad) = (chat("Good"), chat("Bad"));
        {
            let (mut st, _) = open(d.path());
            st.save(&good).unwrap();
            st.save(&bad).unwrap();
        }
        fs::write(file_of(d.path(), bad.id), b"{ not json").unwrap();
        let (st, w) = open(d.path());
        assert!(w.contains(&StoreWarning::Quarantined(bad.id)));
        assert!(!file_of(d.path(), bad.id).exists());
        let rows = st.rows();
        assert_eq!(rows.len(), 2);
        let status = |id| rows.iter().find(|r| r.id == id).unwrap().status;
        assert_eq!(status(good.id), RowStatus::Ok);
        assert_eq!(status(bad.id), RowStatus::Quarantined);
    }

    #[test]
    fn quarantined_rows_survive_an_index_rebuild() {
        let d = TempDir::new().unwrap();
        let bad = chat("Bad");
        {
            let (mut st, _) = open(d.path());
            st.save(&bad).unwrap();
        }
        fs::write(file_of(d.path(), bad.id), b"garbage").unwrap();
        drop(open(d.path()));
        fs::remove_file(d.path().join(INDEX_FILE)).unwrap();
        let (st, _) = open(d.path());
        assert_eq!(st.rows()[0].status, RowStatus::Quarantined);
    }

    #[test]
    fn a_newer_schema_file_is_left_untouched_and_marked() {
        let d = TempDir::new().unwrap();
        let id = SessionId::new();
        let bytes = br#"{"schema_version":99,"session":{"title":"From the future","weird":true}}"#;
        fs::write(file_of(d.path(), id), bytes).unwrap();
        let (st, w) = open(d.path());
        assert!(w.contains(&StoreWarning::NewerSchema(id)));
        assert_eq!(fs::read(file_of(d.path(), id)).unwrap(), bytes);
        assert_eq!(st.rows()[0].status, RowStatus::NewerSchema);
        assert_eq!(st.rows()[0].title, "From the future");
    }

    #[cfg(unix)]
    #[test]
    fn a_transient_read_error_does_not_rename() {
        use std::os::unix::fs::PermissionsExt;
        let d = TempDir::new().unwrap();
        let s = chat("Locked");
        {
            let (mut st, _) = open(d.path());
            st.save(&s).unwrap();
        }
        let p = file_of(d.path(), s.id);
        fs::write(&p, Store::encode(&chat("Changed")).unwrap()).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&p).is_ok() {
            return; // running as root: permissions do not block reads
        }
        let (st, _) = open(d.path());
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(p.exists());
        assert_eq!(st.rows()[0].status, RowStatus::Unreadable);
    }

    #[test]
    fn pin_survives_an_index_rebuild() {
        let d = TempDir::new().unwrap();
        let mut s = chat("Pinned");
        s.set_pinned(true);
        {
            let (mut st, _) = open(d.path());
            st.save(&s).unwrap();
        }
        fs::remove_file(d.path().join(INDEX_FILE)).unwrap();
        let (st, _) = open(d.path());
        assert!(st.rows()[0].pinned);
    }

    #[test]
    fn a_crash_left_streaming_session_is_normalized_on_open() {
        let d = TempDir::new().unwrap();
        let mut s = chat("Crashed");
        s.push_user("q", 2_000);
        let id = s.begin_assistant(RunId::new(), TurnId::new(), 2_001);
        s.append_assistant(id, "part", "");
        {
            let (mut st, _) = open(d.path());
            st.save(&s).unwrap();
        }
        let (mut st, _) = open(d.path());
        let loaded = st.load(s.id).unwrap();
        assert!(loaded.is_idle());
        assert!(matches!(
            &loaded.messages[1].content,
            crate::session::MessageContent::Assistant { status: AssistantStatus::Interrupted, text, .. }
                if text == "part"
        ));
        assert!(!st.rows()[0].active);
    }

    #[test]
    fn a_second_open_on_the_same_dir_is_locked() {
        let d = TempDir::new().unwrap();
        let first = open(d.path());
        assert!(matches!(Store::open(d.path()), Err(StoreError::Locked)));
        drop(first);
        assert!(Store::open(d.path()).is_ok());
    }

    #[test]
    fn delete_removes_the_file_set_aside_copies_and_the_row() {
        let d = TempDir::new().unwrap();
        let s = chat("Doomed");
        let (mut st, _) = open(d.path());
        st.save(&s).unwrap();
        fs::write(d.path().join(format!("{}{CORRUPT_INFIX}1", s.id)), b"old").unwrap();
        st.delete(s.id).unwrap();
        assert!(st.rows().is_empty());
        let left: Vec<_> = fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(&s.id.to_string()))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_deleted_session_waits_in_the_trash_and_comes_back_on_restore() {
        let d = TempDir::new().unwrap();
        let s = chat("Doomed");
        let (mut st, _) = open(d.path());
        st.save(&s).unwrap();
        st.delete(s.id).unwrap();
        assert!(st.rows().is_empty());
        assert!(!file_of(d.path(), s.id).exists());
        assert!(d
            .path()
            .join("trash")
            .join(format!("{}.json", s.id))
            .exists());
        st.restore(s.id).unwrap();
        assert!(file_of(d.path(), s.id).exists());
        assert_eq!(st.rows().len(), 1);
        assert_eq!(st.load(s.id).unwrap().title, "Doomed");
    }

    #[test]
    fn a_purged_session_cannot_be_restored() {
        let d = TempDir::new().unwrap();
        let s = chat("Gone");
        let (mut st, _) = open(d.path());
        st.save(&s).unwrap();
        st.delete(s.id).unwrap();
        st.purge(s.id);
        assert!(st.restore(s.id).is_err());
        assert!(st.rows().is_empty());
        assert!(!d
            .path()
            .join("trash")
            .join(format!("{}.json", s.id))
            .exists());
    }

    #[test]
    fn whatever_is_left_in_the_trash_is_removed_at_the_next_start() {
        let d = TempDir::new().unwrap();
        let s = chat("Left behind");
        {
            let (mut st, _) = open(d.path());
            st.save(&s).unwrap();
            st.delete(s.id).unwrap();
        }
        let (mut st, _) = open(d.path());
        assert!(!d.path().join("trash").exists());
        assert!(st.restore(s.id).is_err());
    }

    #[test]
    fn unrelated_files_are_ignored() {
        let d = TempDir::new().unwrap();
        fs::write(d.path().join("notes.txt"), b"x").unwrap();
        fs::write(d.path().join("readme.json"), b"{}").unwrap();
        let (st, w) = open(d.path());
        assert!(st.rows().is_empty());
        assert!(w.is_empty());
    }

    #[test]
    fn rows_carry_a_one_line_preview_of_the_first_user_message() {
        let d = TempDir::new().unwrap();
        let (mut st, _) = open(d.path());
        let mut s = Session::new_chat("m", SessionLifetime::Persisted, 1_000);
        s.push_user("  How do I\n   make   sourdough?\n\nAlso rye.  ", 2_000);
        s.push_user("a later message", 3_000);
        st.save(&s).unwrap();
        assert_eq!(
            st.rows()[0].preview.as_deref(),
            Some("How do I make sourdough? Also rye.")
        );
        let mut long = Session::new_chat("m", SessionLifetime::Persisted, 1_000);
        long.push_user(&"word ".repeat(60), 2_000);
        st.save(&long).unwrap();
        let p = st
            .rows()
            .into_iter()
            .find(|r| r.id == long.id)
            .unwrap()
            .preview
            .unwrap();
        assert_eq!(p.chars().count(), 80);
        assert!(p.ends_with('…'));
    }

    #[test]
    fn an_empty_chat_has_an_empty_preview() {
        let d = TempDir::new().unwrap();
        let (mut st, _) = open(d.path());
        st.save(&chat("Empty")).unwrap();
        assert_eq!(st.rows()[0].preview.as_deref(), Some(""));
    }

    #[test]
    fn index_rows_from_before_previews_existed_are_refreshed_on_open() {
        let d = TempDir::new().unwrap();
        let mut s = chat("Old");
        s.push_user("what was asked", 2_000);
        {
            let (mut st, _) = open(d.path());
            st.save(&s).unwrap();
        }
        // Strip the field, as an index written by an earlier build would be.
        let path = d.path().join(INDEX_FILE);
        let mut index: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for row in index["rows"].as_array_mut().unwrap() {
            row.as_object_mut().unwrap().remove("preview");
        }
        fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();
        let (st, _) = open(d.path());
        assert_eq!(st.rows()[0].preview.as_deref(), Some("what was asked"));
    }
}
