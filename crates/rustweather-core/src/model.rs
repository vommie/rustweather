//! The weather as the app sees it, whichever service it came from. Everything is metric here; units are for display only.

use chrono::{DateTime, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Current {
    pub station: String,
    pub source: String,
    pub measured_at: Option<DateTime<Utc>>,
    pub temperature: Option<f32>,
    pub dew_point: Option<f32>,
    pub humidity: Option<f32>,
    pub condition: String,
    pub visibility_km: Option<f32>,
    pub precipitation_mm: Option<f32>,
    pub pressure: Option<f32>,
    pub wind_kmh: Option<f32>,
    pub wind_dir: Option<f32>,
    pub gust_kmh: Option<f32>,
    pub cloud_cover: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hour {
    pub time: Option<DateTime<Local>>,
    pub temperature: f32,
    pub feels: f32,
    pub precipitation: f32,
    /// `None` where the service gives no chance: far ahead.
    pub probability: Option<f32>,
    pub snow: f32,
    pub wind: f32,
    pub gust: f32,
    pub humidity: f32,
    pub cloud: f32,
    pub pressure: f32,
    pub uv: f32,
    pub code: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Day {
    pub date: Option<NaiveDate>,
    pub min: f32,
    pub max: f32,
    pub precipitation: f32,
    pub probability: Option<f32>,
    pub gust: f32,
    pub sunshine_hours: f32,
    pub code: u8,
    pub sunrise: Option<DateTime<Local>>,
    pub sunset: Option<DateTime<Local>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Forecast {
    pub feels_like: Option<f32>,
    pub uv: Option<f32>,
    pub code: u8,
    /// Includes the past two days, for the history curve.
    pub hours: Vec<Hour>,
    pub days: Vec<Day>,
    pub air_quality: Option<f32>,
    pub pm25: Option<f32>,
    pub pollen: Vec<(String, f32)>,
}

impl Forecast {
    pub fn today(&self) -> Option<&Day> {
        let today = Local::now().date_naive();
        self.days.iter().find(|d| d.date == Some(today))
    }

    /// The change of the next three hours, for the small arrow on the tray icon.
    pub fn trend(&self, now_temp: f32) -> Trend {
        let horizon = Local::now() + chrono::TimeDelta::hours(3);
        let later = self.hours.iter().find(|h| h.time.is_some_and(|t| t >= horizon));
        match later.map(|h| h.temperature - now_temp) {
            Some(d) if d >= 1.0 => Trend::Up,
            Some(d) if d <= -1.0 => Trend::Down,
            _ => Trend::Steady,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Trend {
    Up,
    Down,
    #[default]
    Steady,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Alert {
    pub id: String,
    /// 1 = minor .. 4 = extreme.
    pub level: u8,
    pub event: String,
    pub headline: String,
    pub description: String,
    pub instruction: String,
    pub onset: Option<DateTime<Utc>>,
    pub expires: Option<DateTime<Utc>>,
}

/// What the radar says about rain at the user's place.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum RainOutlook {
    #[default]
    Unknown,
    Dry,
    Coming {
        in_minutes: u32,
        for_minutes: Option<u32>,
        peak_mmh: f32,
    },
    Raining {
        ends_in_minutes: Option<u32>,
        peak_mmh: f32,
    },
}

/// The words for a WMO weather code.
pub fn condition(code: u8) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51..=57 => "Drizzle",
        61..=67 => "Rain",
        71..=77 => "Snow",
        80..=82 => "Showers",
        85 | 86 => "Snow showers",
        95..=99 => "Thunderstorm",
        _ => "Unknown",
    }
}

pub fn rain_word(mmh: f32) -> &'static str {
    match mmh {
        x if x < 0.8 => "light",
        x if x < 4.0 => "moderate",
        x if x < 12.0 => "heavy",
        _ => "very heavy",
    }
}
