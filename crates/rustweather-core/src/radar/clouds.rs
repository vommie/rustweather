//! The clouds layer: Meteosat pictures from the DWD's map server, free and without a key. New pictures come every three hours and
//! with a delay. The layer's default time "current" can point at a picture that is gone, so every request names its time and
//! walks back from the newest slot. The picture is opaque, with land and sea in it: only what is bright stays, as clouds.

use chrono::{DateTime, DurationRound as _, TimeDelta, Utc};

use crate::error::{Error, Result};
use crate::http::Http;
use crate::map::{View, to_unit};

const LAYER: &str = "dwd:Satellite_meteosat_1km_euat_rgb_day_hrv_and_night_ir108_3h";
const SLOTS: i64 = 5;
/// Half the view's size: a cloud has soft edges, and the server takes seconds per picture.
pub const SCALE: u32 = 2;
const CLEAR: f32 = 0.42;
const OVERCAST: f32 = 0.78;

/// The three-hourly slots to try, newest first.
pub fn slots(now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let newest = now.duration_trunc(TimeDelta::hours(3)).unwrap_or(now);
    (0..SLOTS).map(|i| newest - TimeDelta::hours(3 * i)).collect()
}

fn url(view: &View, time: DateTime<Utc>) -> String {
    let half = 6_378_137.0 * std::f64::consts::PI;
    let scale = view.scale();
    let (cx, cy) = to_unit(view.lon, view.lat);
    let (dx, dy) = (
        f64::from(view.width) / 2.0 / scale,
        f64::from(view.height) / 2.0 / scale,
    );
    let (x0, x1) = (((cx - dx) * 2.0 - 1.0) * half, ((cx + dx) * 2.0 - 1.0) * half);
    let (y0, y1) = ((1.0 - (cy + dy) * 2.0) * half, (1.0 - (cy - dy) * 2.0) * half);
    format!(
        "https://maps.dwd.de/geoserver/dwd/wms?service=WMS&version=1.3.0&request=GetMap&layers={LAYER}&styles=&crs=EPSG:3857\
         &bbox={x0},{y0},{x1},{y1}&width={}&height={}&format=image/png&time={}",
        (view.width / SCALE).max(1),
        (view.height / SCALE).max(1),
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
pub async fn fetch(http: &Http, view: &View) -> Result<Clouds> {
    for time in slots(Utc::now()) {
        let bytes = http.bytes(&url(view, time)).await?;
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
        let view = View {
            lon: 10.0,
            lat: 51.0,
            zoom: 6.0,
            width: 800,
            height: 600,
        };
        let time = DateTime::parse_from_rfc3339("2026-10-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let text = url(&view, time);
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
