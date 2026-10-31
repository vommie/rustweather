//! RustWeather: the temperature in your tray and a weather radar that leaves nothing to wish for.

mod alerts;
mod app;
mod desktop;
mod hover;
mod instance;
mod logging;
mod models;
mod presence;
mod radar_ui;
mod settings_ui;
mod timeline_ui;
mod traffic_ui;
mod tray;
mod warning_ui;
mod weather;
mod window_state;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

/// The code Slint generates, which is not held to this project's lints.
#[allow(clippy::all, clippy::unwrap_used, clippy::expect_used, clippy::panic, unsafe_code)]
mod ui {
    slint::include_modules!();
}
pub use ui::*;

/// The temperature in your tray and a weather radar.
#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Cli {
    /// Minutes between weather updates.
    #[arg(short, long)]
    pub interval: Option<u32>,
    /// A wetterdienst.de station page, the fallback source.
    #[arg(short, long)]
    pub url: Option<String>,
    /// Log detail: ERROR, WARN, INFO, DEBUG.
    #[arg(short, long)]
    pub log_level: Option<String>,
    /// Allow several running instances.
    #[arg(short = 'm', long)]
    pub allow_multiple: bool,
    /// Tile color of the tray icon, for example #1c2128.
    #[arg(long)]
    pub bg_color: Option<String>,
    /// Digit color of the tray icon.
    #[arg(long)]
    pub fg_color: Option<String>,
    /// Keep all files under this folder (test environment).
    #[arg(long, hide = true)]
    pub home: Option<PathBuf>,
    /// Open a page: now, forecast, radar, alerts, settings.
    #[arg(long)]
    pub open: Option<String>,
    /// Desktop integration without the window: install, uninstall or status.
    #[arg(long, value_name = "ACTION")]
    pub desktop: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match app::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("RustWeather could not start: {e}");
            ExitCode::FAILURE
        }
    }
}
