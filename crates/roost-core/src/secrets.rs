//! Hugging Face token storage. The token is never logged or put in errors.

use crate::error::{Error, Result};
use std::sync::Mutex;

pub trait TokenStore: Send + Sync {
    fn get(&self) -> Result<Option<String>>;
    fn set(&self, token: &str) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

const SERVICE: &str = "hf2ollama";
const ACCOUNT: &str = "huggingface-token";

/// OS keychain-backed store (service `hf2ollama`, account `huggingface-token`).
#[derive(Debug, Default)]
pub struct KeychainStore;

// Keyring error text describes the failure, never the secret.
fn keyring_err(e: keyring::Error) -> Error {
    Error::Io(format!("keychain: {e}"))
}

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(keyring_err)
}

impl TokenStore for KeychainStore {
    fn get(&self) -> Result<Option<String>> {
        match entry()?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_err(e)),
        }
    }

    fn set(&self, token: &str) -> Result<()> {
        entry()?.set_password(token).map_err(keyring_err)
    }

    fn clear(&self) -> Result<()> {
        match entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_err(e)),
        }
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemoryStore {
    token: Mutex<Option<String>>,
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore")
            .field("token", &"<redacted>")
            .finish()
    }
}

impl MemoryStore {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.token.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl TokenStore for MemoryStore {
    fn get(&self) -> Result<Option<String>> {
        Ok(self.slot().clone())
    }

    fn set(&self, token: &str) -> Result<()> {
        *self.slot() = Some(token.to_string());
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        *self.slot() = None;
        Ok(())
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
