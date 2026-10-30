//! Everything the user can set. Unknown or missing keys fall back to their defaults, so old files keep working.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::paths::Paths;
use crate::store;

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub schema: u32,
    pub general: General,
    pub places: Vec<Place>,
    pub active_place: usize,
    pub tray: Tray,
    pub radar: Radar,
    pub cache: Cache,
    pub alerts: Alerts,
    pub units: Units,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            general: General::default(),
            places: vec![Place::default()],
            active_place: 0,
            tray: Tray::default(),
            radar: Radar::default(),
            cache: Cache::default(),
            alerts: Alerts::default(),
            units: Units::default(),
        }
    }
}

impl Settings {
    pub fn place(&self) -> Place {
        self.places
            .get(self.active_place)
            .or(self.places.first())
            .cloned()
            .unwrap_or_default()
    }

    /// Loads the settings; a file from an older schema is backed up, then rewritten.
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.settings_file();
        let mut settings: Self = store::load(&file)?;
        if settings.schema < SCHEMA {
            store::backup(&file, &paths.backups(), "before-migration")?;
            settings.schema = SCHEMA;
            settings.save(&file)?;
        }
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        store::save(path, self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct General {
    /// Minutes between weather fetches.
    pub refresh_minutes: u32,
    pub theme: ThemeMode,
    pub log_level: String,
    /// A wetterdienst.de station page, the fallback source of the weather.
    pub station_url: String,
}

impl Default for General {
    fn default() -> Self {
        Self {
            refresh_minutes: 10,
            theme: ThemeMode::System,
            log_level: "info".into(),
            station_url: "https://www.wetterdienst.de/Deutschlandwetter/Berlin/Aktuell/103840".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

impl Default for Place {
    fn default() -> Self {
        Self {
            name: "Berlin".into(),
            lat: 52.52,
            lon: 13.405,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Tray {
    /// RustWeather's card while the pointer rests on the icon, instead of the panel's tooltip.
    pub hover_card: bool,
    pub show_rain_dot: bool,
    pub show_alert: bool,
    pub show_stale: bool,
    pub show_trend: bool,
    /// Overrides the tile color; empty = color of the temperature.
    pub background: String,
    /// Overrides the digit color; empty = white.
    pub foreground: String,
    pub line_feels_like: bool,
    pub line_min_max: bool,
    pub line_rain: bool,
    pub line_alerts: bool,
    pub line_wind: bool,
    pub line_humidity: bool,
    pub line_sun: bool,
    pub line_age: bool,
    pub line_next: bool,
}

impl Default for Tray {
    fn default() -> Self {
        Self {
            hover_card: false,
            show_rain_dot: true,
            show_alert: true,
            show_stale: true,
            show_trend: true,
            background: String::new(),
            foreground: String::new(),
            line_feels_like: true,
            line_min_max: true,
            line_rain: true,
            line_alerts: true,
            line_wind: true,
            line_humidity: true,
            line_sun: true,
            line_age: true,
            line_next: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RadarSource {
    #[default]
    Auto,
    Dwd,
    RainViewer,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Basemap {
    #[default]
    CartoDark,
    CartoVoyager,
    Own,
    Osm,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RainPalette {
    #[default]
    Classic,
    Calm,
    Blue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Radar {
    pub source: RadarSource,
    pub basemap: Basemap,
    pub palette: RainPalette,
    /// 0..=100
    pub opacity: u32,
    /// Milliseconds per 5-minute frame.
    pub frame_ms: u32,
    pub smooth: bool,
    pub loop_playback: bool,
    pub labels_above_rain: bool,
}

impl Default for Radar {
    fn default() -> Self {
        Self {
            source: RadarSource::Auto,
            basemap: Basemap::CartoDark,
            palette: RainPalette::Classic,
            opacity: 85,
            frame_ms: 300,
            smooth: true,
            loop_playback: true,
            labels_above_rain: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Cache {
    pub max_mb: u32,
    pub radar_hours: u32,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            max_mb: 300,
            radar_hours: 24,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Alerts {
    pub paused: bool,
    pub rain: bool,
    pub rain_end: bool,
    pub dwd: bool,
    pub thunderstorm: bool,
    pub frost: bool,
    pub heat: bool,
    /// 1 = every warning .. 4 = only extreme.
    pub min_level: u8,
    pub lead_minutes: u32,
    pub quiet_at_night: bool,
    pub quiet_from_hour: u32,
    pub quiet_to_hour: u32,
    pub severe_ignores_quiet: bool,
    pub quiet_when_fullscreen: bool,
}

impl Default for Alerts {
    fn default() -> Self {
        Self {
            paused: false,
            rain: true,
            rain_end: true,
            dwd: true,
            thunderstorm: true,
            frost: true,
            heat: true,
            min_level: 2,
            lead_minutes: 20,
            quiet_at_night: true,
            quiet_from_hour: 22,
            quiet_to_hour: 7,
            severe_ignores_quiet: true,
            quiet_when_fullscreen: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TempUnit {
    #[default]
    Celsius,
    Fahrenheit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WindUnit {
    #[default]
    Kmh,
    Ms,
    Beaufort,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Units {
    pub temperature: TempUnit,
    pub wind: WindUnit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_gives_defaults_and_roundtrips() {
        let s: Settings = toml::from_str("").unwrap();
        assert_eq!(s, Settings::default());
        let text = toml::to_string_pretty(&s).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), s);
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let s: Settings = toml::from_str("[radar]\nopacity = 40\n").unwrap();
        assert_eq!(s.radar.opacity, 40);
        assert_eq!(s.radar.frame_ms, Radar::default().frame_ms);
    }
}
