//! Open-Meteo: forecast, UV, air quality and pollen. No key needed.

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone as _};
use serde::Deserialize;

use crate::error::Result;
use crate::http::Http;
use crate::model::{Day, Forecast, Hour};
use crate::settings::Place;

#[derive(Deserialize, Default)]
struct Answer {
    #[serde(default)]
    current: CurrentBlock,
    #[serde(default)]
    hourly: Hourly,
    #[serde(default)]
    daily: Daily,
}

#[derive(Deserialize, Default)]
struct CurrentBlock {
    apparent_temperature: Option<f32>,
    uv_index: Option<f32>,
    weather_code: Option<u8>,
}

#[derive(Deserialize, Default)]
struct Hourly {
    #[serde(default)]
    time: Vec<String>,
    #[serde(default)]
    temperature_2m: Vec<Option<f32>>,
    #[serde(default)]
    precipitation: Vec<Option<f32>>,
    #[serde(default)]
    precipitation_probability: Vec<Option<f32>>,
    #[serde(default)]
    weather_code: Vec<Option<u8>>,
}

#[derive(Deserialize, Default)]
struct Daily {
    #[serde(default)]
    time: Vec<String>,
    #[serde(default)]
    temperature_2m_min: Vec<Option<f32>>,
    #[serde(default)]
    temperature_2m_max: Vec<Option<f32>>,
    #[serde(default)]
    precipitation_sum: Vec<Option<f32>>,
    #[serde(default)]
    weather_code: Vec<Option<u8>>,
    #[serde(default)]
    sunrise: Vec<String>,
    #[serde(default)]
    sunset: Vec<String>,
}

fn local_time(text: &str) -> Option<DateTime<Local>> {
    let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M").ok()?;
    Local.from_local_datetime(&naive).earliest()
}

fn at<T: Copy + Default>(values: &[Option<T>], index: usize) -> T {
    values.get(index).copied().flatten().unwrap_or_default()
}

#[derive(Deserialize, Default)]
struct AirAnswer {
    #[serde(default)]
    current: AirBlock,
}

#[derive(Deserialize, Default)]
struct AirBlock {
    european_aqi: Option<f32>,
    pm2_5: Option<f32>,
    alder_pollen: Option<f32>,
    birch_pollen: Option<f32>,
    grass_pollen: Option<f32>,
    mugwort_pollen: Option<f32>,
    olive_pollen: Option<f32>,
    ragweed_pollen: Option<f32>,
}

pub async fn forecast(http: &Http, place: &Place) -> Result<Forecast> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&timezone=auto&past_days=1&forecast_days=7\
         &current=apparent_temperature,uv_index,weather_code\
         &hourly=temperature_2m,precipitation,precipitation_probability,weather_code\
         &daily=temperature_2m_min,temperature_2m_max,precipitation_sum,weather_code,sunrise,sunset",
        place.lat, place.lon
    );
    let a: Answer = http.json(&url).await?;
    let hours = a
        .hourly
        .time
        .iter()
        .enumerate()
        .map(|(i, t)| Hour {
            time: local_time(t),
            temperature: at(&a.hourly.temperature_2m, i),
            precipitation: at(&a.hourly.precipitation, i),
            probability: at(&a.hourly.precipitation_probability, i),
            code: at(&a.hourly.weather_code, i),
        })
        .collect();
    let days = a
        .daily
        .time
        .iter()
        .enumerate()
        .map(|(i, t)| Day {
            date: NaiveDate::parse_from_str(t, "%Y-%m-%d").ok(),
            min: at(&a.daily.temperature_2m_min, i),
            max: at(&a.daily.temperature_2m_max, i),
            precipitation: at(&a.daily.precipitation_sum, i),
            code: at(&a.daily.weather_code, i),
            sunrise: a.daily.sunrise.get(i).and_then(|s| local_time(s)),
            sunset: a.daily.sunset.get(i).and_then(|s| local_time(s)),
        })
        .collect();
    let mut forecast = Forecast {
        feels_like: a.current.apparent_temperature,
        uv: a.current.uv_index,
        code: a.current.weather_code.unwrap_or_default(),
        hours,
        days,
        ..Forecast::default()
    };
    // Air quality is an extra: the forecast stays useful when this service is down.
    match air(http, place).await {
        Ok(block) => {
            forecast.air_quality = block.european_aqi;
            forecast.pm25 = block.pm2_5;
            forecast.pollen = [
                ("Alder", block.alder_pollen),
                ("Birch", block.birch_pollen),
                ("Grass", block.grass_pollen),
                ("Mugwort", block.mugwort_pollen),
                ("Olive", block.olive_pollen),
                ("Ragweed", block.ragweed_pollen),
            ]
            .into_iter()
            .filter_map(|(name, value)| value.filter(|v| *v > 0.0).map(|v| (name.to_owned(), v)))
            .collect();
        }
        Err(e) => tracing::warn!(error = %e, "[Sources] Air quality is unavailable"),
    }
    Ok(forecast)
}

async fn air(http: &Http, place: &Place) -> Result<AirBlock> {
    let url = format!(
        "https://air-quality-api.open-meteo.com/v1/air-quality?latitude={}&longitude={}\
         &current=european_aqi,pm2_5,alder_pollen,birch_pollen,grass_pollen,mugwort_pollen,olive_pollen,ragweed_pollen",
        place.lat, place.lon
    );
    Ok(http.json::<AirAnswer>(&url).await?.current)
}
