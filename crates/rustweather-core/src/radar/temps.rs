//! The temperature layer: values on a grid that follows the zoom, so labels keep their distance: zooming out shows fewer of them, zooming in
//! more. A cell's place is fixed on the map, so its label moves with the map and each value is only fetched once.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::http::Http;
use crate::map::{View, from_unit};

/// Wanted distance between two labels, in pixels.
const SPACING: f64 = 96.0;
const MAX_LEVEL: i32 = 12;
/// One request; each point counts toward Open-Meteo's daily limit, so a batch stays small.
pub const BATCH: usize = 80;
pub const MAX_AGE: Duration = Duration::from_secs(30 * 60);

/// Zoom level, column, row.
pub type Cell = (i32, i64, i64);

/// The grid level for a view: cells of about `SPACING` pixels.
pub fn level(view: &View) -> i32 {
    ((view.scale() / SPACING).log2().round() as i32).clamp(1, MAX_LEVEL)
}

fn center(cell: Cell) -> (f64, f64) {
    let n = f64::from(1u32 << cell.0);
    from_unit((cell.1 as f64 + 0.5) / n, (cell.2 as f64 + 0.5) / n)
}

/// The cells on screen, from the middle outward.
pub fn visible(view: &View) -> Vec<Cell> {
    let z = level(view);
    let n = 1i64 << z;
    let cells = n as f64;
    let e = view.extent(0.0);
    let (x0, x1) = ((e.x0 * cells).floor() as i64, (e.x1 * cells).floor() as i64);
    let (y0, y1) = (
        ((e.y0 * cells).floor() as i64).max(0),
        ((e.y1 * cells).floor() as i64).min(n - 1),
    );
    let (mx, my) = ((x0 + x1) / 2, (y0 + y1) / 2);
    let mut out: Vec<Cell> = (y0..=y1)
        .flat_map(|y| (x0..=x1).map(move |x| (z, x.rem_euclid(n), y)))
        .collect();
    out.sort_by_key(|c| (c.1 - mx.rem_euclid(n)).abs() + (c.2 - my).abs());
    out.dedup();
    out
}

/// What is known, and what the view still lacks.
#[derive(Default)]
pub struct Store {
    values: HashMap<Cell, (f32, Instant)>,
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

    pub fn insert(&mut self, found: Vec<(Cell, f32)>) {
        let now = Instant::now();
        self.values.retain(|_, (_, at)| at.elapsed() < MAX_AGE * 2);
        self.values
            .extend(found.into_iter().map(|(cell, value)| (cell, (value, now))));
    }

    /// The labels of a view: pixel position and temperature.
    pub fn labels(&self, view: &View) -> Vec<(f32, f32, f32)> {
        visible(view)
            .into_iter()
            .filter_map(|cell| {
                let (value, _) = self.values.get(&cell)?;
                let (lon, lat) = center(cell);
                // A cell of the wrapped map can lie one world to the side.
                let (cx, _) = crate::map::to_unit(view.lon, view.lat);
                let (ux, _) = crate::map::to_unit(lon, lat);
                let shift = (cx - ux).round();
                let x = (ux + shift - cx) * view.scale() + f64::from(view.width) / 2.0;
                let (_, y) = view.pixel_of(lon, lat);
                Some((x as f32, y as f32, *value))
            })
            .collect()
    }
}

pub async fn fetch(http: &Http, cells: &[Cell]) -> Result<Vec<(Cell, f32)>> {
    let places: Vec<(f64, f64)> = cells.iter().map(|c| center(*c)).collect();
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
    if list.len() != cells.len() {
        return Err(Error::format("Open-Meteo", format!("{} temperatures", list.len())));
    }
    Ok(cells
        .iter()
        .zip(list)
        .filter_map(|(cell, point)| Some((*cell, point["current"]["temperature_2m"].as_f64()? as f32)))
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
    fn labels_keep_their_distance_at_every_zoom() {
        for zoom in [4.0, 5.3, 6.7, 8.0, 10.0] {
            let count = visible(&view(zoom)).len();
            assert!((30..=240).contains(&count), "zoom {zoom}: {count} labels");
        }
    }

    #[test]
    fn a_cell_stays_on_its_place_when_the_map_moves() {
        let v = view(6.0);
        let mut store = Store::default();
        store.insert(visible(&v).into_iter().map(|c| (c, 9.0)).collect());
        let a = store.labels(&v)[0];
        let moved = v.panned(30.0, 10.0);
        let hit = store
            .labels(&moved)
            .iter()
            .any(|b| (b.0 - a.0 - 30.0).abs() < 0.5 && (b.1 - a.1 - 10.0).abs() < 0.5);
        assert!(hit);
    }

    #[test]
    fn only_what_is_missing_gets_fetched() {
        let v = view(6.0);
        let cells = visible(&v);
        let mut store = Store::default();
        assert_eq!(store.missing(&cells).len(), cells.len().min(BATCH));
        store.insert(cells.iter().map(|c| (*c, 1.0)).collect());
        assert!(store.missing(&cells).is_empty());
    }
}
