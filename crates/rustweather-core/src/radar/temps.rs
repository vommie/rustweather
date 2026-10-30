//! Temperatures across the map. The labels belong to cities (`cities.rs`). The heatmap needs evenly spaced values, so it has a grid that
//! follows the zoom. Everything is kept for a while, so each value is only fetched once.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::cities;
use crate::error::{Error, Result};
use crate::http::Http;
use crate::map::{Extent, View, from_unit};

/// Wanted distance between two grid points, in pixels.
const SPACING: f64 = 96.0;
const MAX_LEVEL: i32 = 12;
/// One request; each point counts toward Open-Meteo's daily limit, so a batch stays small.
pub const BATCH: usize = 80;
pub const MAX_AGE: Duration = Duration::from_secs(30 * 60);
/// Fine enough for soft color bands; the screen stretches the picture and smooths it further.
const PIXELS_PER_CELL: usize = 16;
/// A pixel needs at least this share of its four neighbors to be colored.
const MIN_WEIGHT: f32 = 0.4;

/// Zoom level, column, row.
pub type Cell = (i32, i64, i64);

/// A city's number in `cities.rs`.
pub type CityId = u32;

/// The grid level for a view: cells of about `SPACING` pixels.
pub fn level(view: &View) -> i32 {
    ((view.scale() / SPACING).log2().round() as i32).clamp(1, MAX_LEVEL)
}

fn center(cell: Cell) -> (f64, f64) {
    let n = f64::from(1u32 << cell.0);
    from_unit((cell.1 as f64 + 0.5) / n, (cell.2 as f64 + 0.5) / n)
}

/// Level, cells per world, and the columns and rows of the view with `extra` cells around it.
fn span(view: &View, extra: i64) -> (i32, i64, i64, i64, i64, i64) {
    let z = level(view);
    let n = 1i64 << z;
    let cells = n as f64;
    let e = view.extent(0.0);
    (
        z,
        n,
        (e.x0 * cells).floor() as i64 - extra,
        (e.x1 * cells).floor() as i64 + extra,
        ((e.y0 * cells).floor() as i64 - extra).max(0),
        ((e.y1 * cells).floor() as i64 + extra).min(n - 1),
    )
}

fn from_middle(view: &View, extra: i64) -> Vec<Cell> {
    let (z, n, x0, x1, y0, y1) = span(view, extra);
    let (mx, my) = ((x0 + x1) / 2, (y0 + y1) / 2);
    let mut out: Vec<Cell> = (y0..=y1)
        .flat_map(|y| (x0..=x1).map(move |x| (z, x.rem_euclid(n), y)))
        .collect();
    out.sort_by_key(|c| (c.1 - mx.rem_euclid(n)).abs() + (c.2 - my).abs());
    out.dedup();
    out
}

/// The cells the heatmap needs: it blends between cell centers, so one more on every side.
pub fn around(view: &View) -> Vec<Cell> {
    from_middle(view, 1)
}

/// A colored picture of the temperature and the stretch of the map it shows.
pub struct Heat {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub extent: Extent,
    /// How much of the picture is known, 0..=1.
    pub coverage: f32,
}

/// What is known, and what the view still lacks.
#[derive(Default)]
pub struct Store {
    values: HashMap<Cell, (f32, Instant)>,
    cities: HashMap<CityId, (f32, Instant)>,
}

impl Store {
    pub fn missing(&self, cells: &[Cell]) -> Vec<Cell> {
        cells
            .iter()
            .filter(|c| self.values.get(c).is_none_or(|(_, at)| at.elapsed() >= MAX_AGE))
            .take(BATCH)
            .copied()
            .collect()
    }

    /// The cities of a view without a value that is still good.
    pub fn missing_cities(&self, cities: &[CityId]) -> Vec<CityId> {
        cities
            .iter()
            .filter(|c| self.cities.get(c).is_none_or(|(_, at)| at.elapsed() >= MAX_AGE))
            .take(BATCH)
            .copied()
            .collect()
    }

    pub fn insert_cities(&mut self, found: Vec<(CityId, f32)>) {
        let now = Instant::now();
        self.cities.retain(|_, (_, at)| at.elapsed() < MAX_AGE * 2);
        self.cities
            .extend(found.into_iter().map(|(city, value)| (city, (value, now))));
    }

    pub fn insert(&mut self, found: Vec<(Cell, f32)>) {
        let now = Instant::now();
        self.values.retain(|_, (_, at)| at.elapsed() < MAX_AGE * 2);
        self.values
            .extend(found.into_iter().map(|(cell, value)| (cell, (value, now))));
    }

    /// The labels of a view: city, pixel position and temperature.
    pub fn labels(&self, view: &View) -> Vec<(&'static str, f32, f32, f32)> {
        cities::visible(view)
            .into_iter()
            .filter_map(|id| {
                let (value, _) = self.cities.get(&id)?;
                let (name, x, y) = cities::on_view(id, view)?;
                Some((name, x as f32, y as f32, *value))
            })
            .collect()
    }

    /// Blends the known cells of a view into a picture. A gap takes its color from the cells next to it, and stays empty when they are too few.
    pub fn heat(&self, view: &View) -> Option<Heat> {
        let (z, n, x0, x1, y0, y1) = span(view, 1);
        let (columns, rows) = (usize::try_from(x1 - x0 + 1).ok()?, usize::try_from(y1 - y0 + 1).ok()?);
        if columns < 2 || rows < 2 {
            return None;
        }
        let known: Vec<Option<f32>> = (y0..=y1)
            .flat_map(|y| (x0..=x1).map(move |x| (z, x.rem_euclid(n), y)))
            .map(|cell| self.values.get(&cell).map(|(value, _)| *value))
            .collect();
        let found = known.iter().flatten().count();
        if found == 0 {
            return None;
        }
        let (width, height) = ((columns - 1) * PIXELS_PER_CELL + 1, (rows - 1) * PIXELS_PER_CELL + 1);
        let mut rgba = vec![0u8; width * height * 4];
        for (row, line) in rgba.chunks_exact_mut(width * 4).enumerate() {
            let (j, ty) = (
                row / PIXELS_PER_CELL,
                (row % PIXELS_PER_CELL) as f32 / PIXELS_PER_CELL as f32,
            );
            for (column, pixel) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (i, tx) = (
                    column / PIXELS_PER_CELL,
                    (column % PIXELS_PER_CELL) as f32 / PIXELS_PER_CELL as f32,
                );
                let (right, below) = ((i + 1).min(columns - 1), (j + 1).min(rows - 1));
                let corners = [
                    (i, j, (1.0 - tx) * (1.0 - ty)),
                    (right, j, tx * (1.0 - ty)),
                    (i, below, (1.0 - tx) * ty),
                    (right, below, tx * ty),
                ];
                let (mut sum, mut weight) = (0.0, 0.0);
                for (x, y, share) in corners {
                    if let Some(value) = known[y * columns + x] {
                        sum += value * share;
                        weight += share;
                    }
                }
                if weight >= MIN_WEIGHT {
                    let color = crate::color::tile(sum / weight);
                    pixel.copy_from_slice(&[color.0, color.1, color.2, 255]);
                }
            }
        }
        // The first pixel sits on the first cell's center, half a pixel inside the picture.
        let cells = n as f64;
        let half = 0.5 / (PIXELS_PER_CELL as f64 * cells);
        Some(Heat {
            width: width as u32,
            height: height as u32,
            rgba,
            extent: Extent {
                x0: (x0 as f64 + 0.5) / cells - half,
                y0: (y0 as f64 + 0.5) / cells - half,
                x1: (x1 as f64 + 0.5) / cells + half,
                y1: (y1 as f64 + 0.5) / cells + half,
            },
            coverage: found as f32 / known.len() as f32,
        })
    }
}

/// The temperature at each place, in the order asked. Longitude first.
async fn fetch_places(http: &Http, places: &[(f64, f64)]) -> Result<Vec<Option<f32>>> {
    let lats: Vec<String> = places.iter().map(|p| format!("{:.3}", p.1)).collect();
    let lons: Vec<String> = places.iter().map(|p| format!("{:.3}", p.0)).collect();
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m",
        lats.join(","),
        lons.join(",")
    );
    let answer: Value = http.json(&url).await?;
    // One place comes back as an object, several as a list.
    let list = match answer {
        Value::Array(list) => list,
        single => vec![single],
    };
    if list.len() != places.len() {
        return Err(Error::format("Open-Meteo", format!("{} temperatures", list.len())));
    }
    Ok(list
        .iter()
        .map(|point| point["current"]["temperature_2m"].as_f64().map(|v| v as f32))
        .collect())
}

pub async fn fetch(http: &Http, cells: &[Cell]) -> Result<Vec<(Cell, f32)>> {
    let places: Vec<(f64, f64)> = cells.iter().map(|c| center(*c)).collect();
    let values = fetch_places(http, &places).await?;
    Ok(cells
        .iter()
        .zip(values)
        .filter_map(|(cell, value)| Some((*cell, value?)))
        .collect())
}

pub async fn fetch_cities(http: &Http, ids: &[CityId]) -> Result<Vec<(CityId, f32)>> {
    let places: Vec<(f64, f64)> = ids.iter().filter_map(|id| cities::place(*id)).collect();
    let values = fetch_places(http, &places).await?;
    Ok(ids
        .iter()
        .zip(values)
        .filter_map(|(id, value)| Some((*id, value?)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(zoom: f64) -> View {
        View {
            lon: 10.0,
            lat: 51.0,
            zoom,
            width: 1000,
            height: 700,
        }
    }

    #[test]
    fn the_grid_stays_small_at_every_zoom() {
        for zoom in [4.0, 5.3, 6.7, 8.0, 10.0] {
            let count = around(&view(zoom)).len();
            assert!((30..=400).contains(&count), "zoom {zoom}: {count} points");
        }
    }

    #[test]
    fn a_city_label_stays_on_its_place_when_the_map_moves() {
        let v = view(6.0);
        let mut store = Store::default();
        store.insert_cities(cities::visible(&v).into_iter().map(|c| (c, 9.0)).collect());
        let a = store.labels(&v)[0];
        let moved = v.panned(30.0, 10.0);
        let hit = store
            .labels(&moved)
            .iter()
            .any(|b| b.0 == a.0 && (b.1 - a.1 - 30.0).abs() < 0.5 && (b.2 - a.2 - 10.0).abs() < 0.5);
        assert!(hit);
    }

    #[test]
    fn the_heatmap_covers_the_view_and_blends_between_cells() {
        let v = view(6.0);
        let mut store = Store::default();
        let cells = around(&v);
        store.insert(cells.iter().map(|c| (*c, (c.1 - cells[0].1) as f32)).collect());
        let heat = store.heat(&v).unwrap();
        let e = v.extent(0.0);
        assert!(heat.extent.x0 < e.x0 && heat.extent.x1 > e.x1 && heat.extent.y0 < e.y0 && heat.extent.y1 > e.y1);
        assert!((heat.coverage - 1.0).abs() < f32::EPSILON);
        assert_eq!(heat.rgba.len(), (heat.width * heat.height * 4) as usize);
        assert!(heat.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        // Between two cells 1 degree apart lies a color neither of them has.
        let line = &heat.rgba[..heat.width as usize * 4];
        let distinct: std::collections::HashSet<&[u8; 4]> = line.as_chunks::<4>().0.iter().collect();
        assert!(distinct.len() > 20);
    }

    #[test]
    fn the_heatmap_stays_empty_where_nothing_is_known() {
        let v = view(6.0);
        let mut store = Store::default();
        assert!(store.heat(&v).is_none());
        store.insert(vec![(around(&v)[0], 5.0)]);
        let heat = store.heat(&v).unwrap();
        assert!(heat.coverage < 0.1);
        assert!(heat.rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 0));
    }

    #[test]
    fn only_what_is_missing_gets_fetched() {
        let v = view(6.0);
        let cells = around(&v);
        let mut store = Store::default();
        assert_eq!(store.missing(&cells).len(), cells.len().min(BATCH));
        store.insert(cells.iter().map(|c| (*c, 1.0)).collect());
        assert!(store.missing(&cells).is_empty());
    }
}
