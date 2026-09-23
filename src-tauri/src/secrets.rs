//! `SecretStore`: the only place the OpenRouter API key touches (spec FR-2,
//! NFR-3). `KeyringStore` puts it in the OS credential store (macOS
//! Keychain / Windows Credential Manager); `MemoryStore` is an in-process
//! stand-in for tests, which must never exercise the real keyring.

use std::sync::Mutex;

use thiserror::Error;

const SERVICE: &str = "bistec-architect";
const USER: &str = "openrouter-api-key";

/// Errors from a `SecretStore` operation.
#[derive(Debug, Error)]
pub enum SecretError {
  #[error("credential store error: {0}")]
  Keyring(#[from] keyring::Error),
}

/// Stores (and only stores) the OpenRouter API key. The UI can set it,
/// clear it, or ask whether one is set — it can never read the value back
/// through a Tauri command (spec FR-2); only trusted Rust code that holds a
/// `&dyn SecretStore` calls `get`.
pub trait SecretStore: Send + Sync {
  fn set(&self, key: &str) -> Result<(), SecretError>;
  fn get(&self) -> Result<Option<String>, SecretError>;
  fn clear(&self) -> Result<(), SecretError>;

  fn has(&self) -> Result<bool, SecretError> {
    Ok(self.get()?.is_some())
  }
}

/// The real `SecretStore`, backed by the OS credential store via the
/// `keyring` crate (service `"bistec-architect"`, user `"openrouter-api-key"`).
pub struct KeyringStore {
  entry: keyring::Entry,
}

impl KeyringStore {
  pub fn new() -> Result<Self, SecretError> {
    let entry = keyring::Entry::new(SERVICE, USER)?;
    Ok(KeyringStore { entry })
  }
}

impl SecretStore for KeyringStore {
  fn set(&self, key: &str) -> Result<(), SecretError> {
    self.entry.set_password(key)?;
    Ok(())
  }

  fn get(&self) -> Result<Option<String>, SecretError> {
    match self.entry.get_password() {
      Ok(password) => Ok(Some(password)),
      Err(keyring::Error::NoEntry) => Ok(None),
      Err(err) => Err(err.into()),
    }
  }

  fn clear(&self) -> Result<(), SecretError> {
    match self.entry.delete_credential() {
      Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
      Err(err) => Err(err.into()),
    }
  }
}

/// An in-process `SecretStore` for tests: never touches the OS credential
/// store.
#[derive(Default)]
pub struct MemoryStore {
  value: Mutex<Option<String>>,
}

impl SecretStore for MemoryStore {
  fn set(&self, key: &str) -> Result<(), SecretError> {
    *self.value.lock().expect("MemoryStore mutex poisoned") = Some(key.to_string());
    Ok(())
  }

  fn get(&self) -> Result<Option<String>, SecretError> {
    Ok(self.value.lock().expect("MemoryStore mutex poisoned").clone())
  }

  fn clear(&self) -> Result<(), SecretError> {
    *self.value.lock().expect("MemoryStore mutex poisoned") = None;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn memory_store_set_get_has_clear_round_trip() {
    let store = MemoryStore::default();
    assert_eq!(store.get().unwrap(), None);
    assert!(!store.has().unwrap());

    store.set("sk-or-test-SECRET").unwrap();
    assert_eq!(store.get().unwrap(), Some("sk-or-test-SECRET".to_string()));
    assert!(store.has().unwrap());

    store.clear().unwrap();
    assert_eq!(store.get().unwrap(), None);
    assert!(!store.has().unwrap());
  }

  #[test]
  fn memory_store_set_overwrites_previous_value() {
    let store = MemoryStore::default();
    store.set("first").unwrap();
    store.set("second").unwrap();
    assert_eq!(store.get().unwrap(), Some("second".to_string()));
  }

  #[test]
  fn memory_store_as_trait_object_never_exposes_the_key_through_ipc_shaped_calls() {
    // A `has()`-only view is exactly the shape a Tauri command may safely
    // expose (spec AC-3): it can prove a key is set without returning it.
    let store: &dyn SecretStore = &MemoryStore::default();
    store.set("sk-or-test-SECRET").unwrap();
    assert!(store.has().unwrap());
  }
}
