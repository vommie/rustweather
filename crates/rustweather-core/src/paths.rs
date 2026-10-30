//! Where RustWeather keeps its files, following the XDG base directories.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The folders of one profile. `--home` (tests, test environment) puts all of them under one folder.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
}

impl Paths {
    /// `~/.config/rustweather`, `~/.cache/rustweather`, `~/.local/state/rustweather`.
    pub fn standard() -> Result<Self> {
        let dirs = directories::ProjectDirs::from("", "", "rustweather").ok_or(Error::NoHome)?;
        let state = dirs
            .state_dir()
            .map_or_else(|| dirs.data_local_dir().join("state"), Path::to_path_buf);
        Ok(Self {
            config: dirs.config_dir().to_path_buf(),
            cache: dirs.cache_dir().to_path_buf(),
            state,
        })
    }

    pub fn under(home: &Path) -> Self {
        Self {
            config: home.join("config"),
            cache: home.join("cache"),
            state: home.join("state"),
        }
    }

    pub fn create_all(&self) -> Result<()> {
        for dir in [&self.config, &self.cache, &self.state, &self.logs(), &self.backups()] {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        Ok(())
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    /// API keys live apart from the settings, readable by the owner only.
    pub fn secrets_file(&self) -> PathBuf {
        self.config.join("secrets.toml")
    }

    pub fn ui_state_file(&self) -> PathBuf {
        self.config.join("ui-state.toml")
    }

    pub fn backups(&self) -> PathBuf {
        self.config.join("backups")
    }

    pub fn logs(&self) -> PathBuf {
        self.state.join("logs")
    }
}
