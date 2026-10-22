//! Record of which Hugging Face repo each imported Ollama model came from.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    path: PathBuf,
    entries: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    imports: BTreeMap<String, String>,
}

impl History {
    /// Loads history from `path`; a missing file yields an empty history bound to `path`.
    pub fn load(path: &Path) -> Result<History> {
        let entries = match std::fs::read_to_string(path) {
            Ok(text) => {
                serde_json::from_str::<Stored>(&text)
                    .map_err(|e| Error::Parse(e.to_string()))?
                    .imports
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(Error::Io(e.to_string())),
        };
        Ok(History {
            path: path.to_path_buf(),
            entries,
        })
    }

    pub fn record(&mut self, ollama_name: &str, repo_id: &str) {
        self.entries
            .insert(ollama_name.to_string(), repo_id.to_string());
    }

    pub fn source_of(&self, ollama_name: &str) -> Option<&str> {
        self.entries.get(ollama_name).map(String::as_str)
    }

    pub fn forget(&mut self, ollama_name: &str) {
        self.entries.remove(ollama_name);
    }

    /// Saves atomically: temp file in the same directory, then rename.
    pub fn save(&self) -> Result<()> {
        let stored = Stored {
            imports: self.entries.clone(),
        };
        let text =
            serde_json::to_string_pretty(&stored).map_err(|e| Error::Parse(e.to_string()))?;
        let io = |e: std::io::Error| Error::Io(e.to_string());
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let mut tmp = self.path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, text).map_err(io)?;
        std::fs::rename(&tmp, &self.path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            io(e)
        })
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
