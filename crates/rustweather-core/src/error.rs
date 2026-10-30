//! The error type of the core crate. Every variant reads as a sentence, because it can reach the status line.

use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no home folder found")]
    NoHome,
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: {source}")]
    Toml { path: PathBuf, source: toml::de::Error },
    #[error("could not serialize: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("no connection to {host}")]
    Offline { host: String },
    #[error("{host} answered with HTTP {status}")]
    Http { host: String, status: u16 },
    #[error("{host} allows no more requests right now")]
    RateLimited {
        host: String,
        retry_after_secs: Option<u64>,
    },
    #[error("the key for {service} was rejected")]
    BadKey { service: String },
    #[error("{service} sent data this version cannot read: {detail}")]
    Format { service: String, detail: String },
}

impl Error {
    pub fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    pub fn format(service: &str, detail: impl std::fmt::Display) -> Self {
        Self::Format {
            service: service.to_owned(),
            detail: detail.to_string(),
        }
    }

    /// True when waiting and trying again can help.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Offline { .. } | Self::RateLimited { .. } | Self::Http { status: 500.., .. }
        )
    }
}
