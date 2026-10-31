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
    #[serde(default)]
    apparent_temperature: Vec<Option<f32>>,
    #[serde(default)]
    snowfall: Vec<Option<f32>>,
    #[serde(default)]
    wind_speed_10m: Vec<Option<f32>>,
    #[serde(default)]
    wind_gusts_10m: Vec<Option<f32>>,
    #[serde(default)]
    relative_humidity_2m: Vec<Option<f32>>,
    #[serde(default)]
    cloud_cover: Vec<Option<f32>>,
    #[serde(default)]
    surface_pressure: Vec<Option<f32>>,
    #[serde(default)]
    uv_index: Vec<Option<f32>>,
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
    #[serde(default)]
    precipitation_probability_max: Vec<Option<f32>>,
    #[serde(default)]
    wind_gusts_10m_max: Vec<Option<f32>>,
    #[serde(default)]
    sunshine_duration: Vec<Option<f32>>,
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
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&timezone=auto&past_days=2&forecast_days=16\
         &current=apparent_temperature,uv_index,weather_code\
         &hourly=temperature_2m,apparent_temperature,precipitation,precipitation_probability,snowfall,weather_code,wind_speed_10m,\
         wind_gusts_10m,relative_humidity_2m,cloud_cover,surface_pressure,uv_index\
         &daily=temperature_2m_min,temperature_2m_max,precipitation_sum,precipitation_probability_max,wind_gusts_10m_max,\
         sunshine_duration,weather_code,sunrise,sunset",
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
            feels: at(&a.hourly.apparent_temperature, i),
            precipitation: at(&a.hourly.precipitation, i),
            probability: a.hourly.precipitation_probability.get(i).copied().flatten(),
            snow: at(&a.hourly.snowfall, i),
            wind: at(&a.hourly.wind_speed_10m, i),
            gust: at(&a.hourly.wind_gusts_10m, i),
            humidity: at(&a.hourly.relative_humidity_2m, i),
            cloud: at(&a.hourly.cloud_cover, i),
            pressure: at(&a.hourly.surface_pressure, i),
            uv: at(&a.hourly.uv_index, i),
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
            probability: a.daily.precipitation_probability_max.get(i).copied().flatten(),
            gust: at(&a.daily.wind_gusts_10m_max, i),
            sunshine_hours: at(&a.daily.sunshine_duration, i) / 3600.0,
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

/// The likely temperatures of the coming hours: the spread of the model's many runs, without the wildest tenth on each side.
pub async fn spread(http: &Http, place: &Place) -> Result<Vec<(DateTime<Local>, f32, f32)>> {
    let url = format!(
        "https://ensemble-api.open-meteo.com/v1/ensemble?latitude={}&longitude={}&timezone=auto&forecast_days=16\
         &models=gfs_seamless&hourly=temperature_2m",
        place.lat, place.lon
    );
    let answer: serde_json::Value = http.json(&url).await?;
    Ok(read_spread(&answer))
}

fn read_spread(answer: &serde_json::Value) -> Vec<(DateTime<Local>, f32, f32)> {
    let Some(hourly) = answer["hourly"].as_object() else {
        return Vec::new();
    };
    let runs: Vec<&Vec<serde_json::Value>> = hourly
        .iter()
        .filter(|(name, _)| name.starts_with("temperature_2m"))
        .filter_map(|(_, values)| values.as_array())
        .collect();
    let times = hourly.get("time").and_then(|t| t.as_array());
    times
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, time)| {
            let time = local_time(time.as_str()?)?;
            let mut values: Vec<f32> = runs
                .iter()
                .filter_map(|r| r.get(i)?.as_f64())
                .map(|v| v as f32)
                .collect();
            if values.len() < 3 {
                return None;
            }
            values.sort_by(f32::total_cmp);
            let cut = values.len() / 10;
            Some((time, values[cut], values[values.len() - 1 - cut]))
        })
        .collect()
}

async fn air(http: &Http, place: &Place) -> Result<AirBlock> {
    let url = format!(
        "https://air-quality-api.open-meteo.com/v1/air-quality?latitude={}&longitude={}\
         &current=european_aqi,pm2_5,alder_pollen,birch_pollen,grass_pollen,mugwort_pollen,olive_pollen,ragweed_pollen",
        place.lat, place.lon
    );
    Ok(http.json::<AirAnswer>(&url).await?.current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spread_leaves_out_the_wildest_runs() {
        let mut hourly = serde_json::Map::new();
        hourly.insert("time".into(), serde_json::json!(["2026-10-31T12:00"]));
        for (i, value) in [0.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 30.0]
            .iter()
            .enumerate()
        {
            hourly.insert(format!("temperature_2m_member{i:02}"), serde_json::json!([value]));
        }
        let answer = serde_json::json!({ "hourly": hourly });
        let spread = read_spread(&answer);
        assert_eq!((spread[0].1, spread[0].2), (5.0, 13.0));
    }

    #[test]
    fn no_runs_no_spread() {
        assert!(read_spread(&serde_json::json!({})).is_empty());
    }
}
