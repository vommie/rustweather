//! The wind layer: wind from the DWD's model (`model.rs`), one request an hour for the whole world, shown as particles that flow across the map.
//! Open-Meteo counts every point of a request against its daily limit, so it only steps in when the DWD fails and nothing usable is left.

use std::f32::consts::PI;
use std::path::Path;

use chrono::{DateTime, TimeDelta, Utc};
use serde::Deserialize;

use super::model::Model;

use crate::error::{Error, Result};
use crate::http::Http;
use crate::map::View;

const COVERAGE: &str = "dwd__Icon_reg025_fd_sl_UV10M";
/// A model grid from a few hours ago beats asking for many single points.
const STILL_USABLE: TimeDelta = TimeDelta::hours(6);
const KMH_PER_MS: f32 = 3.6;
const COLUMNS: usize = 9;
const ROWS: usize = 7;
/// The picture has half the view's size: the trails are soft anyway, and it costs a quarter of the work.
pub const SCALE: u32 = 2;
/// Screen pixels (of the half-size picture) a particle moves per frame at 1 km/h.
const SPEED: f32 = 0.09;
const FADE: u16 = 236;
const LIFETIME: u32 = 90;

#[derive(Deserialize)]
struct Point {
    current: Current,
}

#[derive(Deserialize)]
struct Current {
    wind_speed_10m: Option<f32>,
    wind_direction_10m: Option<f32>,
}

/// East and north components in km/h from speed and the direction the wind comes from.
fn components(speed: f32, from_degrees: f32) -> (f32, f32) {
    let to = (from_degrees + 180.0) * PI / 180.0;
    (speed * to.sin(), speed * to.cos())
}

/// Where the wind comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    /// The DWD's whole world: east and north in m/s.
    Model(Model),
    /// Open-Meteo's points around one view.
    Patch(Patch),
}

impl Field {
    /// East and north in km/h at a place; `None` where nothing is known.
    #[inline]
    pub fn at(&self, lon: f64, lat: f64) -> Option<(f32, f32)> {
        match self {
            Self::Model(model) => Some((
                model.sample(0, lon, lat)? * KMH_PER_MS,
                model.sample(1, lon, lat)? * KMH_PER_MS,
            )),
            Self::Patch(patch) => patch.at(lon, lat),
        }
    }

    /// True when nothing has to be asked for.
    pub fn enough(&self, view: &View, hour: DateTime<Utc>, fresh: bool) -> bool {
        match self {
            Self::Model(model) => model.hour == hour,
            Self::Patch(patch) => fresh && patch.covers(view),
        }
    }

    /// True while this can stay on screen even though a newer one failed, so Open-Meteo is not needed.
    pub fn usable(&self, hour: DateTime<Utc>) -> bool {
        matches!(self, Self::Model(model) if hour - model.hour < STILL_USABLE)
    }
}

/// What a round of asking brought.
#[derive(Debug, Default)]
pub struct Loaded {
    pub field: Option<Field>,
    pub dwd_failed: bool,
    pub meteo_limited: bool,
    pub problem: Option<Error>,
}

/// The DWD first; Open-Meteo only when the DWD failed and the caller allows it.
pub async fn load(http: &Http, file: &Path, view: &View, hour: DateTime<Utc>, dwd: bool, meteo: bool) -> Loaded {
    let mut loaded = Loaded::default();
    if dwd {
        match Model::load(http, file, COVERAGE, hour).await {
            Ok(model) => {
                loaded.field = Some(Field::Model(model));
                return loaded;
            }
            Err(error) => {
                tracing::warn!(%error, "[Radar] Wind from the DWD failed");
                loaded.dwd_failed = true;
                loaded.problem = Some(error);
            }
        }
    }
    if meteo {
        match fetch(http, view).await {
            Ok(patch) => {
                loaded.field = Some(Field::Patch(patch));
                loaded.problem = None;
            }
            Err(error) => {
                loaded.meteo_limited = matches!(error, Error::RateLimited { .. });
                // The DWD is the real source: its problem is the one worth telling.
                loaded.problem.get_or_insert(error);
            }
        }
    }
    loaded
}

/// Wind on a regular longitude/latitude grid, covering a bit more than the view it was fetched for.
#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    west: f64,
    north: f64,
    step_lon: f64,
    step_lat: f64,
    /// East, north in km/h, row by row from the north.
    cells: Vec<(f32, f32)>,
    pub zoom: f64,
}

impl Patch {
    /// True while `view` still lies inside the grid at about the same scale.
    pub fn covers(&self, view: &View) -> bool {
        let (west, north) = view.lonlat_of(0.0, 0.0);
        let (east, south) = view.lonlat_of(f64::from(view.width), f64::from(view.height));
        let span_lon = self.step_lon * (COLUMNS - 1) as f64;
        let span_lat = self.step_lat * (ROWS - 1) as f64;
        west >= self.west
            && east <= self.west + span_lon
            && north <= self.north
            && south >= self.north - span_lat
            && (view.zoom - self.zoom).abs() < 1.0
    }

    /// Wind at a place, interpolated; `None` outside the grid.
    #[inline]
    pub fn at(&self, lon: f64, lat: f64) -> Option<(f32, f32)> {
        let fx = (lon - self.west) / self.step_lon;
        let fy = (self.north - lat) / self.step_lat;
        if fx < 0.0 || fy < 0.0 || fx > (COLUMNS - 1) as f64 || fy > (ROWS - 1) as f64 {
            return None;
        }
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(COLUMNS - 1), (y0 + 1).min(ROWS - 1));
        let (kx, ky) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let get = |x: usize, y: usize| self.cells.get(y * COLUMNS + x).copied().unwrap_or_default();
        let (a, b, c, d) = (get(x0, y0), get(x1, y0), get(x0, y1), get(x1, y1));
        let mix = |a: f32, b: f32, k: f32| a + (b - a) * k;
        Some((
            mix(mix(a.0, b.0, kx), mix(c.0, d.0, kx), ky),
            mix(mix(a.1, b.1, kx), mix(c.1, d.1, kx), ky),
        ))
    }
}

/// Fetches the grid around a view, with a quarter of the view as a margin, so short pans need no new request.
pub async fn fetch(http: &Http, view: &View) -> Result<Patch> {
    let (west, north) = view.lonlat_of(0.0, 0.0);
    let (east, south) = view.lonlat_of(f64::from(view.width), f64::from(view.height));
    let (margin_lon, margin_lat) = ((east - west) / 4.0, (north - south) / 4.0);
    let (west, east) = ((west - margin_lon).max(-180.0), (east + margin_lon).min(180.0));
    let (north, south) = ((north + margin_lat).min(85.0), (south - margin_lat).max(-85.0));
    let step_lon = (east - west) / (COLUMNS - 1) as f64;
    let step_lat = (north - south) / (ROWS - 1) as f64;
    let mut lats = Vec::with_capacity(COLUMNS * ROWS);
    let mut lons = Vec::with_capacity(COLUMNS * ROWS);
    for row in 0..ROWS {
        for column in 0..COLUMNS {
            lats.push(format!("{:.3}", north - step_lat * row as f64));
            lons.push(format!("{:.3}", west + step_lon * column as f64));
        }
    }
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=wind_speed_10m,wind_direction_10m",
        lats.join(","),
        lons.join(",")
    );
    let points: Vec<Point> = http.json(&url).await?;
    if points.len() != COLUMNS * ROWS {
        return Err(Error::format("Open-Meteo", format!("{} wind points", points.len())));
    }
    let cells = points
        .iter()
        .map(|p| {
            components(
                p.current.wind_speed_10m.unwrap_or(0.0),
                p.current.wind_direction_10m.unwrap_or(0.0),
            )
        })
        .collect();
    Ok(Patch {
        west,
        north,
        step_lon,
        step_lat,
        cells,
        zoom: view.zoom,
    })
}

struct Particle {
    x: f32,
    y: f32,
    age: u32,
}

/// Small and fast random numbers: the particles only need to look scattered.
fn next_random(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state as f32) / (u32::MAX as f32)
}

/// Calm, fresh, strong, stormy: from the ink of the theme to yellow and red.
fn color(kmh: f32, dark: bool) -> [u8; 3] {
    let calm: [u8; 3] = if dark { [235, 238, 245] } else { [60, 64, 80] };
    let stops = [(0.0, calm), (30.0, calm), (55.0, [250, 204, 21]), (90.0, [239, 68, 68])];
    let upper = stops.iter().position(|s| s.0 >= kmh).unwrap_or(stops.len() - 1).max(1);
    let (a, b) = (stops[upper - 1], stops[upper]);
    let k = ((kmh - a.0) / (b.0 - a.0)).clamp(0.0, 1.0);
    let mix = |n: usize| (f32::from(a.1[n]) + (f32::from(b.1[n]) - f32::from(a.1[n])) * k) as u8;
    [mix(0), mix(1), mix(2)]
}

/// The moving particles and their fading trails, as straight RGBA.
pub struct Flow {
    pub width: u32,
    pub height: u32,
    pixels: Vec<u8>,
    particles: Vec<Particle>,
    seed: u32,
}

impl Flow {
    pub fn new(view: &View) -> Self {
        let (width, height) = ((view.width / SCALE).max(1), (view.height / SCALE).max(1));
        let count = ((width * height) / 420).clamp(100, 1800) as usize;
        let mut seed = 0x9e37_79b9;
        let particles = (0..count)
            .map(|i| Particle {
                x: next_random(&mut seed) * width as f32,
                y: next_random(&mut seed) * height as f32,
                age: i as u32 % LIFETIME,
            })
            .collect();
        Self {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
            particles,
            seed,
        }
    }

    fn plot(&mut self, x: f32, y: f32, rgb: [u8; 3]) {
        if x < 0.0 || y < 0.0 || x >= self.width as f32 || y >= self.height as f32 {
            return;
        }
        let at = (y as usize * self.width as usize + x as usize) * 4;
        if let Some(pixel) = self.pixels.get_mut(at..at + 4) {
            pixel[..3].copy_from_slice(&rgb);
            pixel[3] = 230;
        }
    }

    /// One frame: the trails fade, every particle moves with the wind under it.
    pub fn step(&mut self, view: &View, field: &Field, dark: bool) {
        for alpha in self.pixels.as_chunks_mut::<4>().0 {
            alpha[3] = ((u16::from(alpha[3]) * FADE) >> 8) as u8;
        }
        let (width, height) = (self.width as f32, self.height as f32);
        let mut particles = std::mem::take(&mut self.particles);
        for p in &mut particles {
            let (lon, lat) = view.lonlat_of(f64::from(p.x * SCALE as f32), f64::from(p.y * SCALE as f32));
            let wind = field.at(lon, lat);
            p.age += 1;
            let Some((east, north)) = wind else {
                p.age = LIFETIME;
                self.respawn(p, width, height);
                continue;
            };
            let (dx, dy) = (east * SPEED, -north * SPEED);
            let speed = (east * east + north * north).sqrt();
            let rgb = color(speed, dark);
            let reach = dx.abs().max(dy.abs()).ceil().max(1.0);
            for i in 0..reach as u32 {
                let k = i as f32 / reach;
                self.plot(p.x + dx * k, p.y + dy * k, rgb);
            }
            p.x += dx;
            p.y += dy;
            if p.age >= LIFETIME || p.x < 0.0 || p.y < 0.0 || p.x >= width || p.y >= height {
                self.respawn(p, width, height);
            }
        }
        self.particles = particles;
    }

    fn respawn(&mut self, p: &mut Particle, width: f32, height: f32) {
        p.x = next_random(&mut self.seed) * width;
        p.y = next_random(&mut self.seed) * height;
        p.age = (next_random(&mut self.seed) * 30.0) as u32;
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        View {
            lon: 10.0,
            lat: 51.0,
            zoom: 6.0,
            width: 400,
            height: 300,
        }
    }

    fn patch(east: f32, north: f32) -> Patch {
        Patch {
            west: 0.0,
            north: 60.0,
            step_lon: 3.0,
            step_lat: 3.0,
            cells: vec![(east, north); COLUMNS * ROWS],
            zoom: 6.0,
        }
    }

    fn uniform(east: f32, north: f32) -> Field {
        Field::Patch(patch(east, north))
    }

    fn world(hour: DateTime<Utc>) -> Field {
        let mut model = crate::radar::model::uniform(360, 181, vec![10.0; 360 * 181 * 2]);
        model.hour = hour;
        Field::Model(model)
    }

    #[test]
    fn the_model_is_in_meters_per_second_and_covers_the_world() {
        let field = world(DateTime::UNIX_EPOCH);
        assert_eq!(field.at(10.0, 51.0), Some((36.0, 36.0)));
        assert_eq!(field.at(-170.0, -60.0), Some((36.0, 36.0)));
    }

    #[test]
    fn a_grid_of_this_hour_needs_nothing_wherever_the_map_goes() {
        let hour = DateTime::UNIX_EPOCH;
        let far = View { lon: 170.0, ..view() };
        assert!(world(hour).enough(&far, hour, false));
        assert!(!world(hour).enough(&far, hour + TimeDelta::hours(1), true));
    }

    #[test]
    fn open_meteo_is_only_needed_without_a_recent_grid() {
        let hour = DateTime::UNIX_EPOCH;
        assert!(world(hour).usable(hour + TimeDelta::hours(5)));
        assert!(!world(hour).usable(hour + TimeDelta::hours(7)));
        assert!(!uniform(1.0, 1.0).usable(hour));
    }

    #[test]
    fn open_meteo_points_last_as_long_as_they_fit_the_view() {
        let (hour, field) = (DateTime::UNIX_EPOCH, uniform(1.0, 1.0));
        assert!(field.enough(&view(), hour, true));
        assert!(!field.enough(&view(), hour, false));
        assert!(!field.enough(&View { lon: 100.0, ..view() }, hour, true));
    }

    #[test]
    fn west_wind_blows_east() {
        let (east, north) = components(20.0, 270.0);
        assert!((east - 20.0).abs() < 0.01 && north.abs() < 0.01);
        let (east, north) = components(10.0, 0.0);
        assert!(east.abs() < 0.01 && (north + 10.0).abs() < 0.01);
    }

    #[test]
    fn field_interpolates_and_ends_at_its_edge() {
        let mut f = patch(0.0, 0.0);
        f.cells[1] = (9.0, 0.0);
        assert_eq!(f.at(1.5, 60.0), Some((4.5, 0.0)));
        assert_eq!(f.at(-1.0, 50.0), None);
        assert!(f.covers(&view()));
        assert!(!f.covers(&View { zoom: 8.0, ..view() }));
        assert!(!f.covers(&View { lon: 30.0, ..view() }));
    }

    #[test]
    fn particles_draw_trails_that_fade() {
        let (v, f) = (view(), uniform(30.0, 0.0));
        let mut flow = Flow::new(&v);
        let before: Vec<f32> = flow.particles.iter().map(|p| p.x).collect();
        flow.step(&v, &f, true);
        assert!(flow.pixels().as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        let moved = flow
            .particles
            .iter()
            .zip(&before)
            .filter(|(p, x)| (p.x - **x - 30.0 * SPEED).abs() < 0.01)
            .count();
        assert!(moved > flow.particles.len() / 2);
        let dense = flow.pixels().to_vec();
        for _ in 0..200 {
            flow.step(&v, &uniform(0.0, 0.0), true);
        }
        let old_trails = dense.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count();
        assert!(old_trails > 0 && flow.pixels().len() == dense.len());
    }

    #[test]
    fn strong_wind_is_colored() {
        assert_eq!(color(10.0, true), [235, 238, 245]);
        assert_eq!(color(90.0, true), [239, 68, 68]);
        assert_ne!(color(10.0, true), color(10.0, false));
    }
}
