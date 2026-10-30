//! Finding places by name, through Open-Meteo's geocoder.

use serde::Deserialize;

use crate::error::Result;
use crate::http::Http;
use crate::settings::Place;

#[derive(Deserialize, Default)]
struct Answer {
    #[serde(default)]
    results: Vec<Hit>,
}

#[derive(Deserialize)]
struct Hit {
    name: String,
    latitude: f64,
    longitude: f64,
    admin1: Option<String>,
    country: Option<String>,
}

pub async fn search(http: &Http, text: &str) -> Result<Vec<Place>> {
    let query: String = text
        .bytes()
        .flat_map(|b| {
            if b.is_ascii_alphanumeric() {
                vec![char::from(b)]
            } else {
                format!("%{b:02X}").chars().collect()
            }
        })
        .collect();
    let url = format!("https://geocoding-api.open-meteo.com/v1/search?name={query}&count=8&language=en");
    let answer: Answer = http.json(&url).await?;
    Ok(answer
        .results
        .into_iter()
        .map(|h| {
            let area = [h.admin1, h.country]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ");
            let name = if area.is_empty() {
                h.name
            } else {
                format!("{}, {area}", h.name)
            };
            Place {
                name,
                lat: h.latitude,
                lon: h.longitude,
            }
        })
        .collect())
}
