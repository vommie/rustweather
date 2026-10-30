//! Atomic reading and writing of the TOML files, with a backup before anything older is rewritten.

use std::io::Write as _;
use std::path::Path;

use atomic_write_file::AtomicWriteFile;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Error, Result};

const KEEP_BACKUPS: usize = 10;

/// Reads a TOML file; a missing file gives the defaults.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|source| Error::Toml {
            path: path.to_path_buf(),
            source,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(Error::io(path, e)),
    }
}

pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    write_atomic(path, toml::to_string_pretty(value)?.as_bytes())
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = AtomicWriteFile::open(path).map_err(|e| Error::io(path, e))?;
    file.write_all(bytes).map_err(|e| Error::io(path, e))?;
    file.commit().map_err(|e| Error::io(path, e))
}

/// Copies `path` into `backups`, newest ten kept per file. Runs before every migration.
pub fn backup(path: &Path, backups: &Path, tag: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let target = backups.join(format!("{name}.{tag}.{stamp}"));
    std::fs::copy(path, &target).map_err(|e| Error::io(&target, e))?;
    let mut old: Vec<_> = std::fs::read_dir(backups)
        .map_err(|e| Error::io(backups, e))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&name)))
        .collect();
    old.sort();
    let surplus = old.len().saturating_sub(KEEP_BACKUPS);
    for stale in old.into_iter().take(surplus) {
        let _ = std::fs::remove_file(stale);
    }
    Ok(())
}
