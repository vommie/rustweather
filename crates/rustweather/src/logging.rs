//! Logs go to stderr and to daily files in the state folder. Keys never reach them: URLs are not logged.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _, fmt};

pub fn init(dir: &Path, level: &str) -> WorkerGuard {
    // A week of logs, like everything else the app keeps.
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("rustweather.log")
        .max_log_files(7)
        .build(dir)
        .unwrap_or_else(|_| tracing_appender::rolling::daily(dir, "rustweather.log"));
    let (file, guard) = tracing_appender::non_blocking(appender);
    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level.to_lowercase()));
    let _ = tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr).with_filter(filter()))
        .with(fmt::layer().with_ansi(false).with_writer(file).with_filter(filter()))
        .try_init();
    guard
}
