//! The one HTTP client of the app: connections are reused, every failure becomes an `Error` that reads as a sentence.

use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;

use crate::error::{Error, Result};

/// Most answers come in moments; this also bounds a whole download.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Says what really went wrong: the log gets the cause, and a slow server is not called unreachable.
fn failure(host: &str, error: reqwest::Error) -> Error {
    let timeout = error.is_timeout();
    tracing::debug!(host, error = %error.without_url(), "[Http] Request failed");
    if timeout {
        Error::Timeout { host: host.to_owned() }
    } else {
        Error::Offline { host: host.to_owned() }
    }
}

#[derive(Clone)]
pub struct Http {
    client: Client,
}

fn host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', '?']).next())
        .unwrap_or(url)
        .to_owned()
}

impl Http {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!(
                "RustWeather/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/vommie/rustweather)"
            ))
            .connect_timeout(Duration::from_secs(15))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .map_err(|e| Error::format("HTTP client", e))?;
        Ok(Self { client })
    }

    /// Fetches a body. The URL can carry an API key, so it never appears in an error or a log.
    pub async fn bytes(&self, url: &str) -> Result<Vec<u8>> {
        self.bytes_within(url, TIMEOUT).await
    }

    /// For answers that take a server long to prepare. A failed connection is tried once more: a name lookup can stall for seconds
    /// and work the next moment.
    pub async fn bytes_within(&self, url: &str, limit: Duration) -> Result<Vec<u8>> {
        match self.fetch(url, limit).await {
            Err(Error::Offline { host }) => {
                tracing::debug!(%host, "[Http] Trying the connection once more");
                tokio::time::sleep(Duration::from_secs(1)).await;
                self.fetch(url, limit).await
            }
            other => other,
        }
    }

    async fn fetch(&self, url: &str, limit: Duration) -> Result<Vec<u8>> {
        let name = host(url);
        let response = self
            .client
            .get(url)
            .timeout(limit)
            .send()
            .await
            .map_err(|e| failure(&name, e))?;
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after_secs = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok());
            return Err(Error::RateLimited {
                host: name,
                retry_after_secs,
            });
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(Error::BadKey { service: name });
        }
        if !status.is_success() {
            return Err(Error::Http {
                host: name,
                status: status.as_u16(),
            });
        }
        let body = response.bytes().await.map_err(|e| failure(&name, e))?;
        Ok(body.to_vec())
    }

    pub async fn json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let body = self.bytes(url).await?;
        serde_json::from_slice(&body).map_err(|e| Error::format(&host(url), e))
    }

    pub async fn text(&self, url: &str) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.bytes(url).await?).into_owned())
    }
}
