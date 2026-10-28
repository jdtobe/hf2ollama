//! Hugging Face token storage. The token is never logged or put in errors.

use crate::error::{Error, Result};
use std::sync::Mutex;

pub trait TokenStore: Send + Sync {
    fn get(&self) -> Result<Option<String>>;
    fn set(&self, token: &str) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

const ACCOUNT: &str = "huggingface-token";
const ROOST_SERVICE: &str = "roost";
const LEGACY_SERVICE: &str = "hf2ollama";

/// OS keychain-backed store (account `huggingface-token`).
#[derive(Debug)]
pub struct KeychainStore {
    service: &'static str,
}

impl KeychainStore {
    /// The current Roost keychain service.
    pub fn roost() -> KeychainStore {
        KeychainStore {
            service: ROOST_SERVICE,
        }
    }

    /// The legacy hf2ollama keychain service. Only use this for an explicit import.
    pub fn legacy() -> KeychainStore {
        KeychainStore {
            service: LEGACY_SERVICE,
        }
    }

    fn entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(self.service, ACCOUNT).map_err(keyring_err)
    }
}

impl Default for KeychainStore {
    fn default() -> Self {
        Self::roost()
    }
}

// Keyring error text describes the failure, never the secret.
fn keyring_err(e: keyring::Error) -> Error {
    Error::Io(format!("keychain: {e}"))
}

impl TokenStore for KeychainStore {
    fn get(&self) -> Result<Option<String>> {
        match self.entry()?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_err(e)),
        }
    }

    fn set(&self, token: &str) -> Result<()> {
        self.entry()?.set_password(token).map_err(keyring_err)
    }

    fn clear(&self) -> Result<()> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_err(e)),
        }
    }
}

/// Outcome of an explicit legacy credential import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyTokenMigration {
    Imported,
    /// The current store already holds a token, so nothing was changed.
    AlreadyPresent,
    NotFound,
}

/// Copies a legacy token into the current store only when the current store is
/// empty. The legacy credential is never changed or deleted.
pub fn promote_legacy_token(
    current: &dyn TokenStore,
    legacy: &dyn TokenStore,
) -> Result<LegacyTokenMigration> {
    let read = |store: &dyn TokenStore| {
        store
            .get()
            .map_err(|_| Error::Io("could not access the keychain".into()))
    };
    if read(current)?.is_some() {
        return Ok(LegacyTokenMigration::AlreadyPresent);
    }
    let Some(token) = read(legacy)? else {
        return Ok(LegacyTokenMigration::NotFound);
    };
    current
        .set(&token)
        .map_err(|_| Error::Io("could not write to the keychain".into()))?;
    if read(current)?.as_deref() != Some(token.as_str()) {
        return Err(Error::Io(
            "keychain did not retain imported credential".into(),
        ));
    }
    Ok(LegacyTokenMigration::Imported)
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

    struct FailingWriteStore;

    impl TokenStore for FailingWriteStore {
        fn get(&self) -> Result<Option<String>> {
            Ok(None)
        }

        fn set(&self, _: &str) -> Result<()> {
            Err(Error::Io("keychain write denied".into()))
        }

        fn clear(&self) -> Result<()> {
            Ok(())
        }
    }

    struct TokenInErrorStore;

    impl TokenStore for TokenInErrorStore {
        fn get(&self) -> Result<Option<String>> {
            Err(Error::Io("legacy-token".into()))
        }

        fn set(&self, _: &str) -> Result<()> {
            Ok(())
        }

        fn clear(&self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn promotion_keeps_the_current_token_when_one_is_already_stored() {
        let current = MemoryStore::default();
        let legacy = MemoryStore::default();
        current.set("current-token").unwrap();
        legacy.set("legacy-token").unwrap();

        assert_eq!(
            promote_legacy_token(&current, &legacy).unwrap(),
            LegacyTokenMigration::AlreadyPresent
        );
        assert_eq!(current.get().unwrap().as_deref(), Some("current-token"));
        assert_eq!(legacy.get().unwrap().as_deref(), Some("legacy-token"));
    }

    #[test]
    fn promotion_writes_verifies_and_keeps_the_legacy_token() {
        let current = MemoryStore::default();
        let legacy = MemoryStore::default();
        legacy.set("legacy-token").unwrap();

        assert_eq!(
            promote_legacy_token(&current, &legacy).unwrap(),
            LegacyTokenMigration::Imported
        );
        assert_eq!(current.get().unwrap().as_deref(), Some("legacy-token"));
        assert_eq!(legacy.get().unwrap().as_deref(), Some("legacy-token"));
    }

    #[test]
    fn failed_promotion_keeps_legacy_and_does_not_leak_the_token() {
        let current = FailingWriteStore;
        let legacy = MemoryStore::default();
        legacy.set("legacy-token").unwrap();

        let error = promote_legacy_token(&current, &legacy).unwrap_err();
        assert_eq!(legacy.get().unwrap().as_deref(), Some("legacy-token"));
        assert!(!error.to_string().contains("legacy-token"));
        assert!(!format!("{error:?}").contains("legacy-token"));
    }

    #[test]
    fn promotion_redacts_token_text_from_store_errors() {
        let error = promote_legacy_token(&TokenInErrorStore, &MemoryStore::default()).unwrap_err();
        assert!(!error.to_string().contains("legacy-token"));
        assert!(!format!("{error:?}").contains("legacy-token"));
    }
}
