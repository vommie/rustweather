//! Bright Sky (api.brightsky.dev): DWD open data as JSON, no key needed.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::archive::Measured;
use crate::error::Result;
use crate::http::Http;
use crate::model::{Alert, Current};
use crate::settings::Place;

#[derive(Deserialize)]
struct CurrentAnswer {
    weather: Weather,
    #[serde(default)]
    sources: Vec<Source>,
}

#[derive(Deserialize)]
struct Source {
    #[serde(default)]
    station_name: Option<String>,
}

#[derive(Deserialize)]
struct Weather {
    timestamp: Option<DateTime<Utc>>,
    temperature: Option<f32>,
    dew_point: Option<f32>,
    relative_humidity: Option<f32>,
    condition: Option<String>,
    visibility: Option<f32>,
    precipitation_60: Option<f32>,
    pressure_msl: Option<f32>,
    wind_speed_10: Option<f32>,
    wind_direction_10: Option<f32>,
    wind_gust_speed_10: Option<f32>,
    cloud_cover: Option<f32>,
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

pub async fn current(http: &Http, place: &Place) -> Result<Current> {
    let url = format!(
        "https://api.brightsky.dev/current_weather?lat={}&lon={}",
        place.lat, place.lon
    );
    let answer: CurrentAnswer = http.json(&url).await?;
    let w = answer.weather;
    Ok(Current {
        station: answer
            .sources
            .first()
            .and_then(|s| s.station_name.clone())
            .unwrap_or_default(),
        source: "DWD".into(),
        measured_at: w.timestamp,
        temperature: w.temperature,
        dew_point: w.dew_point,
        humidity: w.relative_humidity,
        condition: w.condition.as_deref().map(capitalized).unwrap_or_default(),
        visibility_km: w.visibility.map(|m| m / 1000.0),
        precipitation_mm: w.precipitation_60,
        pressure: w.pressure_msl,
        wind_kmh: w.wind_speed_10,
        wind_dir: w.wind_direction_10,
        gust_kmh: w.wind_gust_speed_10,
        cloud_cover: w.cloud_cover,
    })
}

#[derive(Deserialize, Default)]
struct HistoryAnswer {
    #[serde(default)]
    weather: Vec<HistoryHour>,
}

#[derive(Deserialize)]
struct HistoryHour {
    timestamp: DateTime<Utc>,
    temperature: Option<f32>,
    precipitation: Option<f32>,
    wind_speed: Option<f32>,
    wind_gust_speed: Option<f32>,
    relative_humidity: Option<f32>,
    cloud_cover: Option<f32>,
    pressure_msl: Option<f32>,
}

/// What stations measured from one moment to another, hour by hour.
pub async fn history(
    http: &Http,
    place: &Place,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<(DateTime<Utc>, Measured)>> {
    let url = format!(
        "https://api.brightsky.dev/weather?lat={}&lon={}&date={}&last_date={}",
        place.lat,
        place.lon,
        from.format("%Y-%m-%dT%H:00:00%%2B00:00"),
        to.format("%Y-%m-%dT%H:00:00%%2B00:00")
    );
    let answer: HistoryAnswer = http.json(&url).await?;
    Ok(answer
        .weather
        .into_iter()
        .map(|h| {
            (
                h.timestamp,
                Measured {
                    temperature: h.temperature,
                    rain: h.precipitation,
                    wind: h.wind_speed,
                    gust: h.wind_gust_speed,
                    humidity: h.relative_humidity,
                    cloud: h.cloud_cover,
                    pressure: h.pressure_msl,
                },
            )
        })
        .collect())
}

#[derive(Deserialize)]
struct AlertsAnswer {
    #[serde(default)]
    alerts: Vec<RawAlert>,
}

#[derive(Deserialize)]
struct RawAlert {
    #[serde(default)]
    alert_id: String,
    severity: Option<String>,
    event_en: Option<String>,
    event_de: Option<String>,
    headline_en: Option<String>,
    description_en: Option<String>,
    instruction_en: Option<String>,
    onset: Option<DateTime<Utc>>,
    expires: Option<DateTime<Utc>>,
}

fn level(severity: Option<&str>) -> u8 {
    match severity {
        Some("extreme") => 4,
        Some("severe") => 3,
        Some("moderate") => 2,
        _ => 1,
    }
}

pub async fn alerts(http: &Http, place: &Place) -> Result<Vec<Alert>> {
    let url = format!("https://api.brightsky.dev/alerts?lat={}&lon={}", place.lat, place.lon);
    let answer: AlertsAnswer = http.json(&url).await?;
    Ok(answer
        .alerts
        .into_iter()
        .map(|a| Alert {
            id: a.alert_id,
            level: level(a.severity.as_deref()),
            event: a.event_en.or(a.event_de).unwrap_or_default(),
            headline: a.headline_en.unwrap_or_default(),
            description: a.description_en.unwrap_or_default(),
            instruction: a.instruction_en.unwrap_or_default(),
            onset: a.onset,
            expires: a.expires,
        })
        .collect())
}
