//! API keys. Entered in Settings, kept in `secrets.toml` with mode 0600, never in the settings file and never logged.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::store;

pub const CARTO: &str = "carto";

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Secrets {
    #[serde(default)]
    keys: BTreeMap<String, String>,
}

// Keys must not reach a log through a `{:?}`.
impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secrets({} keys)", self.keys.len())
    }
}

impl Secrets {
    pub fn load(path: &Path) -> Result<Self> {
        store::load(path)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        store::save(path, self)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| Error::io(path, e))
    }

    pub fn get(&self, service: &str) -> Option<&str> {
        self.keys.get(service).map(String::as_str).filter(|k| !k.is_empty())
    }

    pub fn set(&mut self, service: &str, key: &str) {
        let key = key.trim();
        if key.is_empty() {
            self.keys.remove(service);
        } else {
            self.keys.insert(service.to_owned(), key.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_is_private_and_debug_hides_keys() {
        let dir = std::env::temp_dir().join(format!("rw-secrets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secrets.toml");
        let mut s = Secrets::default();
        s.set(CARTO, " abc ");
        s.save(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(Secrets::load(&path).unwrap().get(CARTO), Some("abc"));
        assert!(!format!("{s:?}").contains("abc"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
