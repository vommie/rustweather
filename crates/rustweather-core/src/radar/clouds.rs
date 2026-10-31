//! The clouds layer, from EUMETSAT's free map service. A plain satellite picture does not do: low clouds are about as warm as the ground, so in
//! infrared they look like it and a brightness filter throws them away. So three sources make one picture: the cloud mask says where clouds
//! are, day and night and low ones too, the infrared picture of the same moment gives them their structure, and beyond the satellite's disk
//! the world ring of all weather satellites stands in, where only the cold clouds can be told from the ground.

use std::time::Duration;

use chrono::{DateTime, Utc};
use image::{GrayImage, RgbaImage, imageops};

use crate::error::{Error, Result};
use crate::http::Http;

const SERVICE: &str = "https://view.eumetsat.int/geoserver/ows?service=WMS&version=1.3.0&request=GetMap&styles=&crs=EPSG:3857\
    &format=image/png&transparent=true";
const MASK: &str = "msg_fes:clm";
const INFRARED: &str = "msg_fes:ir108";
const RING: &str = "mumi:worldcloudmap_ir108";
/// The mask comes every quarter of an hour, some minutes late.
const STEP_MINUTES: i64 = 15;
const SLOTS: i64 = 8;
const TRIES: usize = 3;
const PICTURE_TIMEOUT: Duration = Duration::from_secs(90);
/// How far from the satellite's point over 0° east the picture is still good. Beyond it the ring is needed.
const DISK_LONGITUDE: f64 = 60.0;
const CLEAR: f32 = 0.42;
const OVERCAST: f32 = 0.78;
/// The infrared brightness of the lowest and of the highest clouds.
const WARMEST: f32 = 0.22;
const COLDEST: f32 = 0.75;
/// Low clouds show the map a little, high ones hardly.
const LOW_ALPHA: f32 = 150.0;
const HIGH_ALPHA: f32 = 235.0;
const LOW_SHADE: f32 = 200.0;
const HIGH_SHADE: f32 = 247.0;
/// The mask works in squares of about this many meters: the edges of clouds are softened over one of them.
const MASK_METERS: f32 = 3000.0;
const EARTH_METERS: f32 = 40_075_000.0;

/// The times to try, newest first.
pub fn slots(now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let step = STEP_MINUTES * 60;
    let start = now.timestamp() - STEP_MINUTES * 60;
    let newest = start - start.rem_euclid(step);
    (0..SLOTS)
        .filter_map(|i| DateTime::from_timestamp(newest - i * step, 0))
        .collect()
}

/// The time as the server wants it.
pub fn stamp(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%dT%H:%M:00.000Z").to_string()
}

fn template(layer: &str, size: u32, area: &str, time: Option<DateTime<Utc>>) -> String {
    let when = time.map_or(String::new(), |t| format!("&time={}", stamp(t)));
    format!("{SERVICE}&layers={layer}&bbox={area}&width={size}&height={size}{when}")
}

/// The first source of a tile, and the others.
pub fn sources(time: DateTime<Utc>) -> (String, Vec<String>) {
    (
        template(MASK, 512, "{bbox}", Some(time)),
        vec![
            template(INFRARED, 512, "{bbox}", Some(time)),
            template(RING, 512, "{bbox}", None),
        ],
    )
}

/// True when a tile reaches beyond what the satellite sees well, so it needs the ring.
pub fn beyond_disk(z: i32, x: i64) -> bool {
    let n = f64::from(1u32 << z.clamp(0, 30));
    let (west, east) = (x as f64 / n * 360.0 - 180.0, (x + 1) as f64 / n * 360.0 - 180.0);
    west < -DISK_LONGITUDE || east > DISK_LONGITUDE
}

fn light(pixel: [u8; 4]) -> f32 {
    let [r, g, b, _] = pixel.map(|v| f32::from(v) / 255.0);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn is_cloud(pixel: [u8; 4]) -> bool {
    // White means cloud; clear land is green and clear sea blue.
    pixel[3] > 0 && pixel[0] > 200 && pixel[1] > 200 && pixel[2] > 200
}

/// Makes one picture of the sources, at tile level `z`.
pub fn stack(mask: &RgbaImage, infrared: Option<&RgbaImage>, ring: Option<&RgbaImage>, z: i32) -> RgbaImage {
    let (width, height) = mask.dimensions();
    let cover = GrayImage::from_fn(width, height, |x, y| {
        image::Luma([if is_cloud(mask.get_pixel(x, y).0) { 255 } else { 0 }])
    });
    let meters_per_pixel = EARTH_METERS / (width.max(1) as f32 * (1u32 << z.clamp(0, 30)) as f32);
    let soft = imageops::fast_blur(&cover, (MASK_METERS / meters_per_pixel).clamp(0.8, 16.0));
    RgbaImage::from_fn(width, height, |x, y| {
        if mask.get_pixel(x, y).0[3] == 0 {
            let cold = ring
                .filter(|r| r.dimensions() == (width, height))
                .map(|r| r.get_pixel(x, y).0);
            let amount = cold
                .filter(|p| p[3] > 0)
                .map_or(0.0, |p| ((light(p) - CLEAR) / (OVERCAST - CLEAR)).clamp(0.0, 1.0));
            return image::Rgba([245, 247, 250, (amount * 230.0) as u8]);
        }
        let amount = f32::from(soft.get_pixel(x, y).0[0]) / 255.0;
        let height_share = infrared.filter(|i| i.dimensions() == (width, height)).map_or(0.5, |i| {
            ((light(i.get_pixel(x, y).0) - WARMEST) / (COLDEST - WARMEST)).clamp(0.0, 1.0)
        });
        let shade = (LOW_SHADE + (HIGH_SHADE - LOW_SHADE) * height_share) as u8;
        let alpha = amount * (LOW_ALPHA + (HIGH_ALPHA - LOW_ALPHA) * height_share);
        image::Rgba([shade, shade, shade.saturating_add(3), alpha as u8])
    })
}

/// The time of the newest mask there is. A time the server does not have comes back as text with HTTP 200, so only a picture counts.
pub async fn latest(http: &Http) -> Result<DateTime<Utc>> {
    for time in slots(Utc::now()) {
        let url = template(MASK, 16, "700000,6000000,1500000,7300000", Some(time));
        for _ in 0..TRIES {
            let bytes = http.bytes_within(&url, PICTURE_TIMEOUT).await?;
            if image::load_from_memory(&bytes).is_ok() {
                return Ok(time);
            }
        }
    }
    Err(Error::format("EUMETSAT", "no cloud picture in the last hours"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    fn filled(color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(32, 32, image::Rgba(color))
    }

    #[test]
    fn slots_walk_back_in_quarters_and_leave_the_satellite_time() {
        let slots = slots(time("2026-10-30T16:20:00Z"));
        assert_eq!(slots[0].to_rfc3339(), "2026-10-30T16:00:00+00:00");
        assert_eq!(slots[1].to_rfc3339(), "2026-10-30T15:45:00+00:00");
    }

    #[test]
    fn mask_and_infrared_ask_for_the_same_moment() {
        let (mask, others) = sources(time("2026-10-30T12:00:00Z"));
        assert!(mask.contains("&time=2026-10-30T12:00:00.000Z") && mask.contains(MASK));
        assert!(others[0].contains("&time=2026-10-30T12:00:00.000Z") && others[0].contains(INFRARED));
        assert!(others[1].contains(RING) && !others[1].contains("&time="));
    }

    #[test]
    fn a_low_cloud_stays_although_it_is_as_dark_as_the_ground() {
        // What the old brightness filter threw away: a mid gray infrared picture under a white mask.
        let picture = stack(&filled([255, 255, 255, 255]), Some(&filled([90, 90, 90, 255])), None, 6);
        let pixel = picture.get_pixel(16, 16).0;
        assert!(pixel[3] > 120, "{pixel:?}");
    }

    #[test]
    fn high_clouds_are_brighter_and_thicker_than_low_ones() {
        let mask = filled([255, 255, 255, 255]);
        let low = stack(&mask, Some(&filled([70, 70, 70, 255])), None, 6);
        let high = stack(&mask, Some(&filled([230, 230, 230, 255])), None, 6);
        assert!(high.get_pixel(16, 16).0[0] > low.get_pixel(16, 16).0[0]);
        assert!(high.get_pixel(16, 16).0[3] > low.get_pixel(16, 16).0[3]);
    }

    #[test]
    fn clear_land_and_sea_stay_empty() {
        for clear in [[0, 192, 0, 255], [0, 0, 255, 255]] {
            let picture = stack(&filled(clear), Some(&filled([230, 230, 230, 255])), None, 6);
            assert_eq!(picture.get_pixel(16, 16).0[3], 0);
        }
    }

    #[test]
    fn beyond_the_disk_the_ring_shows_its_cold_clouds() {
        let outside = filled([0, 0, 0, 0]);
        let cold = stack(&outside, None, Some(&filled([240, 240, 240, 255])), 2);
        let warm = stack(&outside, None, Some(&filled([60, 60, 60, 255])), 2);
        assert!(cold.get_pixel(16, 16).0[3] > 200);
        assert_eq!(warm.get_pixel(16, 16).0[3], 0);
        assert_eq!(stack(&outside, None, None, 2).get_pixel(16, 16).0[3], 0);
    }

    #[test]
    fn only_tiles_at_the_edge_need_the_ring() {
        assert!(!beyond_disk(4, 8), "over Europe");
        assert!(beyond_disk(4, 1), "over America");
        assert!(beyond_disk(0, 0), "the whole world");
    }
}
