//! The DWD radar composite "RV": a 1100 × 1200 km grid every 5 minutes with a 2-hour forecast, fetched through Bright Sky
//! as one compressed answer. Values are 0.01 mm per 5 minutes.

use std::io::Read as _;
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use chrono::{DateTime, TimeDelta, Utc};
use proj4rs::Proj;
use rayon::prelude::*;
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::http::Http;
use crate::map::View;

pub const WIDTH: usize = 1100;
pub const HEIGHT: usize = 1200;
const SERVICE: &str = "Bright Sky";
const PROJECTION: &str = "+proj=stere +lat_0=90 +lat_ts=60 +lon_0=10 +a=6378137 +b=6356752.3142451802 +no_defs \
                          +x_0=543196.83521776402 +y_0=3622588.8619310018";

#[derive(Clone)]
pub struct Frame {
    pub time: DateTime<Utc>,
    pub values: Arc<Vec<u16>>,
}

#[derive(Deserialize)]
struct Answer {
    radar: Vec<RawFrame>,
}

#[derive(Deserialize)]
struct RawFrame {
    timestamp: DateTime<Utc>,
    precipitation_5: String,
}

fn decode(raw: &RawFrame) -> Result<Frame> {
    let zipped = base64::engine::general_purpose::STANDARD
        .decode(&raw.precipitation_5)
        .map_err(|e| Error::format(SERVICE, e))?;
    let mut bytes = Vec::with_capacity(WIDTH * HEIGHT * 2);
    flate2::read::ZlibDecoder::new(zipped.as_slice())
        .read_to_end(&mut bytes)
        .map_err(|e| Error::format(SERVICE, e))?;
    if bytes.len() != WIDTH * HEIGHT * 2 {
        return Err(Error::format(SERVICE, format!("unexpected grid size {}", bytes.len())));
    }
    let values = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b))
        .collect();
    Ok(Frame {
        time: raw.timestamp,
        values: Arc::new(values),
    })
}

pub fn parse(body: &[u8]) -> Result<Vec<Frame>> {
    let answer: Answer = serde_json::from_slice(body).map_err(|e| Error::format(SERVICE, e))?;
    answer.radar.par_iter().map(decode).collect()
}

/// Two hours back to two hours ahead. The raw answer is kept in the cache, so the next start shows rain at once.
pub async fn fetch(http: &Http, cache_file: &Path) -> Result<Vec<Frame>> {
    let now = Utc::now();
    let format = "%Y-%m-%dT%H:%M:00Z";
    let (from, to) = (
        (now - TimeDelta::hours(2)).format(format),
        (now + TimeDelta::hours(2)).format(format),
    );
    let url = format!("https://api.brightsky.dev/radar?format=compressed&date={from}&last_date={to}");
    let body = http.bytes(&url).await?;
    let frames = parse(&body)?;
    if let Err(e) = crate::store::write_atomic(cache_file, &body) {
        tracing::warn!(error = %e, "[Radar] Could not cache the grid");
    }
    Ok(frames)
}

pub fn load_cached(cache_file: &Path) -> Option<Vec<Frame>> {
    parse(&std::fs::read(cache_file).ok()?).ok().filter(|f| !f.is_empty())
}

/// Longitude and latitude to grid columns and rows (row 0 is the northern edge).
pub struct Projector {
    grid: Proj,
    geo: Proj,
}

impl Projector {
    pub fn new() -> Result<Self> {
        let parse = |text: &str| Proj::from_proj_string(text).map_err(|e| Error::format("Projection", e));
        Ok(Self {
            grid: parse(PROJECTION)?,
            geo: parse("+proj=longlat +ellps=WGS84 +no_defs")?,
        })
    }

    pub fn project(&self, lon: f64, lat: f64) -> Option<(f32, f32)> {
        let mut point = (lon.to_radians(), lat.to_radians(), 0.0);
        proj4rs::transform::transform(&self.geo, &self.grid, &mut point).ok()?;
        // The false northing puts the grid's northern edge at y = 0; rows run south in kilometers.
        Some(((point.0 / 1000.0) as f32, (-point.1 / 1000.0) as f32))
    }
}

/// For every pixel of a view its place in the grid. The projection runs on a coarse mesh, the rest is interpolated.
pub struct Lut {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<[f32; 2]>,
}

const MESH: usize = 24;

impl Lut {
    pub fn new(view: &View, projector: &Projector) -> Self {
        let (w, h) = (view.width as usize, view.height as usize);
        let (mw, mh) = (w / MESH + 2, h / MESH + 2);
        let mesh: Vec<[f32; 2]> = (0..mh)
            .flat_map(|my| (0..mw).map(move |mx| (mx, my)))
            .map(|(mx, my)| {
                let (lon, lat) = view.lonlat_of((mx * MESH) as f64, (my * MESH) as f64);
                projector.project(lon, lat).map_or([f32::NAN; 2], |(x, y)| [x, y])
            })
            .collect();
        let mut cells = vec![[f32::NAN; 2]; w * h];
        cells.par_chunks_mut(w.max(1)).enumerate().for_each(|(y, row)| {
            let (my, ky) = (y / MESH, (y % MESH) as f32 / MESH as f32);
            for (x, cell) in row.iter_mut().enumerate() {
                let (mx, kx) = (x / MESH, (x % MESH) as f32 / MESH as f32);
                let at = |mx: usize, my: usize| mesh[my * mw + mx];
                let (a, b, c, d) = (at(mx, my), at(mx + 1, my), at(mx, my + 1), at(mx + 1, my + 1));
                for n in 0..2 {
                    let top = a[n] + (b[n] - a[n]) * kx;
                    let bottom = c[n] + (d[n] - c[n]) * kx;
                    cell[n] = top + (bottom - top) * ky;
                }
            }
        });
        Self {
            width: w,
            height: h,
            cells,
        }
    }
}

#[inline]
pub fn sample(values: &[u16], x: f32, y: f32) -> f32 {
    if !(x >= 0.0 && y >= 0.0 && x < (WIDTH - 1) as f32 && y < (HEIGHT - 1) as f32) {
        return 0.0;
    }
    let (x0, y0) = (x as usize, y as usize);
    let (kx, ky) = (x - x0 as f32, y - y0 as f32);
    let at = |x: usize, y: usize| values.get(y * WIDTH + x).map_or(0.0, |v| f32::from(*v));
    let top = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * kx;
    let bottom = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * kx;
    top + (bottom - top) * ky
}

/// mm/h of a grid value.
pub fn rate(value: f32) -> f32 {
    value * 0.12
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_matches_the_grid() {
        let p = Projector::new().unwrap();
        // Bright Sky's own grid positions, read on 2026-10-30 (`bbox` + `latlon_position`, see tests/live.rs).
        for (lon, lat, col, row) in [
            (13.5, 52.51, 790.27, 417.01),
            (11.58, 48.14, 668.92, 935.41),
            (6.96, 50.94, 318.88, 601.2),
        ] {
            let (x, y) = p.project(lon, lat).unwrap();
            assert!((x - col).abs() < 0.1 && (y - row).abs() < 0.1, "{lon},{lat}: {x},{y}");
        }
    }

    #[test]
    fn sampling_outside_is_dry() {
        let values = vec![100u16; WIDTH * HEIGHT];
        assert_eq!(sample(&values, -1.0, 5.0), 0.0);
        assert_eq!(sample(&values, 5.5, 5.5), 100.0);
        assert_eq!(sample(&values, f32::NAN, 5.5), 0.0);
    }
}
