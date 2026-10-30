//! RainViewer: worldwide, two hours back, no forecast, zoom up to 7, one color scheme, 100 requests a minute.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::Result;
use crate::http::Http;
use crate::map::tiles::{Layer, Post};

#[derive(Deserialize)]
struct Answer {
    host: String,
    radar: Radar,
}

#[derive(Deserialize)]
struct Radar {
    #[serde(default)]
    past: Vec<Item>,
    #[serde(default)]
    nowcast: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    time: i64,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub time: DateTime<Utc>,
    pub layer: Layer,
}

pub async fn frames(http: &Http) -> Result<Vec<Frame>> {
    let answer: Answer = http.json("https://api.rainviewer.com/public/weather-maps.json").await?;
    let host = answer.host;
    Ok(answer
        .radar
        .past
        .into_iter()
        .chain(answer.radar.nowcast)
        .filter_map(|item| {
            let time = DateTime::from_timestamp(item.time, 0)?;
            let id = format!("radar-rv/{}", item.path.trim_matches('/').replace('/', "-"));
            // 512 px tiles: one request covers what four 256 px tiles would.
            let url = format!("{host}{}/512/{{z}}/{{x}}/{{y}}/2/1_1.png", item.path);
            Some(Frame {
                time,
                layer: Layer {
                    post: Post::None,
                    id,
                    url,
                    // Counted as 1024: one level coarser than the 512 px pictures would allow. Half the sharpness, a quarter of the requests,
                    // and 100 requests a minute must cover 13 frames.
                    tile_size: 1024,
                    max_zoom: 7,
                },
            })
        })
        .collect())
}
