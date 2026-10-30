//! The clouds layer: Meteosat pictures from the DWD's map server, free and without a key. New pictures come every three hours.
//! The server answers the same request sometimes with the picture and sometimes with "no such time", and its default time
//! "current" fails the same way. So every request names its time, is repeated a few times, and only then falls back to an older
//! slot. The picture is opaque, with land and sea in it: only what is bright stays, as clouds.

use std::time::Duration;

use chrono::{DateTime, DurationRound as _, TimeDelta, Utc};

use crate::error::{Error, Result};
use crate::http::Http;
use crate::map::Extent;

const LAYER: &str = "dwd:Satellite_meteosat_1km_euat_rgb_day_hrv_and_night_ir108_3h";
const SLOTS: i64 = 5;
const TRIES: usize = 4;
/// The server renders a satellite picture on request, which takes seconds and sometimes a minute.
const PICTURE_TIMEOUT: Duration = Duration::from_secs(90);
const CLEAR: f32 = 0.42;
const OVERCAST: f32 = 0.78;

/// The three-hourly slots to try, newest first.
pub fn slots(now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let newest = now.duration_trunc(TimeDelta::hours(3)).unwrap_or(now);
    (0..SLOTS).map(|i| newest - TimeDelta::hours(3 * i)).collect()
}

fn url(extent: &Extent, width: u32, height: u32, time: DateTime<Utc>) -> String {
    format!(
        "https://maps.dwd.de/geoserver/dwd/wms?service=WMS&version=1.3.0&request=GetMap&layers={LAYER}&styles=&crs=EPSG:3857\
         &bbox={}&width={width}&height={height}&format=image/png&time={}",
        extent.bbox_3857(),
        time.format("%Y-%m-%dT%H:%M:00.000Z")
    )
}

/// Turns the satellite picture into white clouds on nothing.
fn keep_clouds(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let [r, g, b, _] = pixel.map(|v| f32::from(v) / 255.0);
        let light = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let cover = ((light - CLEAR) / (OVERCAST - CLEAR)).clamp(0.0, 1.0);
        *pixel = [245, 247, 250, (cover * 230.0) as u8];
    }
}

pub struct Clouds {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub time: DateTime<Utc>,
}

/// The newest picture there is. The server answers "no such time" with an XML text and HTTP 200, so only a picture counts.
pub async fn fetch(http: &Http, extent: &Extent, width: u32, height: u32) -> Result<Clouds> {
    for time in slots(Utc::now()) {
        for _ in 0..TRIES {
            let bytes = http
                .bytes_within(&url(extent, width, height, time), PICTURE_TIMEOUT)
                .await?;
            if let Ok(image) = image::load_from_memory(&bytes) {
                let image = image.into_rgba8();
                let (width, height) = image.dimensions();
                let mut rgba = image.into_raw();
                keep_clouds(&mut rgba);
                return Ok(Clouds {
                    width,
                    height,
                    rgba,
                    time,
                });
            }
        }
    }
    Err(Error::format("DWD", "no satellite picture in the last hours"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_walk_back_in_steps_of_three_hours() {
        let now = DateTime::parse_from_rfc3339("2026-10-30T16:20:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let slots = slots(now);
        assert_eq!(slots[0].to_rfc3339(), "2026-10-30T15:00:00+00:00");
        assert_eq!(slots[1].to_rfc3339(), "2026-10-30T12:00:00+00:00");
        assert_eq!(slots.len(), 5);
    }

    #[test]
    fn every_request_names_its_time() {
        let extent = Extent {
            x0: 0.5,
            y0: 0.3,
            x1: 0.6,
            y1: 0.4,
        };
        let time = DateTime::parse_from_rfc3339("2026-10-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let text = url(&extent, 400, 300, time);
        assert!(text.contains("&time=2026-10-30T12:00:00.000Z"));
        assert!(text.contains("&width=400&height=300"));
    }

    #[test]
    fn only_bright_pixels_stay() {
        let mut pixels = [10, 10, 10, 255, 100, 110, 60, 255, 240, 240, 240, 255];
        keep_clouds(&mut pixels);
        assert_eq!(pixels[3], 0, "dark sea");
        assert_eq!(pixels[7], 0, "green land");
        assert_eq!(pixels[11], 230, "thick cloud");
    }
}
