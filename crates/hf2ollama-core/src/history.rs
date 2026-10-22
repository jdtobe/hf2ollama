//! Record of which Hugging Face repo each imported Ollama model came from.

use crate::error::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    path: PathBuf,
    entries: BTreeMap<String, String>,
}

impl History {
    /// Loads history from `path`; a missing file yields an empty history bound to `path`.
    pub fn load(_path: &Path) -> Result<History> {
        todo!()
    }

    pub fn record(&mut self, _ollama_name: &str, _repo_id: &str) {
        todo!()
    }

    pub fn source_of(&self, _ollama_name: &str) -> Option<&str> {
        todo!()
    }

    pub fn forget(&mut self, _ollama_name: &str) {
        todo!()
    }

    /// Saves atomically: temp file in the same directory, then rename.
    pub fn save(&self) -> Result<()> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn record_then_source_of() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = History::load(&dir.path().join("history.json")).unwrap();
        h.record("llama3:8b", "meta-llama/Llama-3-8B");
        assert_eq!(h.source_of("llama3:8b"), Some("meta-llama/Llama-3-8B"));
        assert_eq!(h.source_of("other"), None);
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("history.json");
        let mut h = History::load(&path).unwrap();
        h.record("a", "org/a");
        h.record("b", "org/b");
        h.save().unwrap();
        let loaded = History::load(&path).unwrap();
        assert_eq!(loaded, h);
        assert_eq!(loaded.source_of("b"), Some("org/b"));
    }

    #[test]
    fn forget_removes_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = History::load(&dir.path().join("history.json")).unwrap();
        h.record("a", "org/a");
        h.forget("a");
        assert_eq!(h.source_of("a"), None);
    }

    #[test]
    fn load_missing_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::load(&dir.path().join("absent.json")).unwrap();
        assert_eq!(h.source_of("anything"), None);
    }

    #[test]
    fn load_corrupt_is_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(matches!(History::load(&path), Err(Error::Parse(_))));
    }
}
