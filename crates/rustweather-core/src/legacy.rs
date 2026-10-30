//! One-time import of `weather_tray.py`'s `cfg/weather_tray.json`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::settings::{Basemap, Settings};

#[derive(Deserialize, Default)]
#[serde(default)]
struct Old {
    url: Option<String>,
    radar_map_style: Option<String>,
    radar_anim_speed: Option<u32>,
    radar_loop: Option<bool>,
    radar_cache_max_age_hours: Option<u32>,
    radar_cache_max_size_mb: Option<u32>,
}

/// Places the old script usually lived in.
pub fn candidates() -> Vec<PathBuf> {
    let home = directories::BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .unwrap_or_default();
    [
        "cfg",
        "claude/cfg",
        "bin/cfg",
        "scripts/cfg",
        ".local/bin/cfg",
        "Scripts/cfg",
    ]
    .iter()
    .map(|dir| home.join(dir).join("weather_tray.json"))
    .collect()
}

/// Applies an old file to `settings`. Returns false when it cannot be read.
pub fn import(path: &Path, settings: &mut Settings) -> bool {
    let Some(old) = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Old>(&t).ok())
    else {
        return false;
    };
    if let Some(url) = old.url {
        settings.general.station_url = url;
    }
    match old.radar_map_style.as_deref() {
        Some("light") => settings.radar.basemap = Basemap::CartoVoyager,
        Some("osm") => settings.radar.basemap = Basemap::Osm,
        Some("dark") => settings.radar.basemap = Basemap::CartoDark,
        _ => {}
    }
    settings.radar.frame_ms = old.radar_anim_speed.unwrap_or(settings.radar.frame_ms);
    settings.radar.loop_playback = old.radar_loop.unwrap_or(settings.radar.loop_playback);
    settings.cache.radar_hours = old.radar_cache_max_age_hours.unwrap_or(settings.cache.radar_hours);
    settings.cache.max_mb = old.radar_cache_max_size_mb.unwrap_or(settings.cache.max_mb);
    true
}
