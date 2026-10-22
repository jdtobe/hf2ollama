//! Hugging Face token storage. The token is never logged or put in errors.

use crate::error::Result;
use std::sync::Mutex;

pub trait TokenStore: Send + Sync {
    fn get(&self) -> Result<Option<String>>;
    fn set(&self, token: &str) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

/// OS keychain-backed store (service `hf2ollama`, account `huggingface-token`).
#[derive(Debug, Default)]
pub struct KeychainStore;

impl TokenStore for KeychainStore {
    fn get(&self) -> Result<Option<String>> {
        todo!()
    }
    fn set(&self, _token: &str) -> Result<()> {
        todo!()
    }
    fn clear(&self) -> Result<()> {
        todo!()
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemoryStore {
    #[allow(dead_code)]
    token: Mutex<Option<String>>,
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore")
            .field("token", &"<redacted>")
            .finish()
    }
}

impl TokenStore for MemoryStore {
    fn get(&self) -> Result<Option<String>> {
        todo!()
    }
    fn set(&self, _token: &str) -> Result<()> {
        todo!()
    }
    fn clear(&self) -> Result<()> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_set_get_clear() {
        let s = MemoryStore::default();
        assert_eq!(s.get().unwrap(), None);
        s.set("hf_secret").unwrap();
        assert_eq!(s.get().unwrap(), Some("hf_secret".to_string()));
        s.clear().unwrap();
        assert_eq!(s.get().unwrap(), None);
    }

    #[test]
    fn memory_store_debug_redacts_token() {
        let s = MemoryStore::default();
        s.set("hf_secret").unwrap();
        assert!(!format!("{s:?}").contains("hf_secret"));
    }
}
