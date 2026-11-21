//! One rain picture for all of Europe: the DWD within the reach of its radars, RainViewer elsewhere, blended over a strip at
//! the edge. Before, the view center picked one source, so the rain of the neighbors vanished as soon as it crossed into
//! Germany. The DWD's grid says "dry" where no radar sees, so it may only count where a radar does.

use chrono::{DateTime, TimeDelta, Utc};
use rayon::prelude::*;

use crate::map::{View, from_unit};

/// The DWD's 17 weather radars, longitude and latitude.
const RADARS: [(f64, f64); 17] = [
    (6.7483, 53.5640),  // Borkum
    (10.0469, 54.0044), // Boostedt
    (13.7687, 51.1246), // Dresden
    (12.4029, 49.5407), // Eisberg
    (6.9671, 51.4056),  // Essen
    (8.0036, 47.8736),  // Feldberg
    (8.8022, 51.3112),  // Flechtdorf
    (9.6945, 52.4601),  // Hannover
    (12.1018, 48.1747), // Isen
    (10.2192, 48.0421), // Memmingen
    (11.1350, 50.5001), // Neuhaus
    (6.5485, 50.1097),  // Neuheilenbach
    (8.7129, 49.9847),  // Offenthal
    (13.8585, 52.6487), // Prötzel
    (12.0581, 54.1757), // Rostock
    (9.7828, 48.5853),  // Türkheim
    (11.1761, 52.1601), // Ummendorf
];
/// Each radar's precipitation scan reaches this far.
const REACH_KM: f64 = 150.0;
/// The strip at the edge where both sources are mixed (user decision 2026-11-21: about 30 km).
const SEAM_KM: f64 = 30.0;
/// Weights are worked out every this many pixels and interpolated in between.
const CELL: usize = 8;
/// Beyond its last frame, RainViewer's rain is moved on along its motion this far, fading over the second half.
pub const AHEAD_MINUTES: f64 = 60.0;
const STEP: TimeDelta = TimeDelta::minutes(5);

fn distance_km(lon: f64, lat: f64, (rlon, rlat): (f64, f64)) -> f64 {
    let dx = (lon - rlon) * 111.32 * ((lat + rlat) / 2.0).to_radians().cos();
    let dy = (lat - rlat) * 110.57;
    (dx * dx + dy * dy).sqrt()
}

/// How much of a place's rain comes from the DWD: 1 well inside the reach of a radar, 0 outside, a ramp across the seam.
pub fn weight(lon: f64, lat: f64) -> f32 {
    let nearest = RADARS
        .iter()
        .map(|r| distance_km(lon, lat, *r))
        .fold(f64::MAX, f64::min);
    ((REACH_KM + SEAM_KM / 2.0 - nearest) / SEAM_KM).clamp(0.0, 1.0) as f32
}

/// The DWD weight of every pixel of a view.
pub struct Weights {
    view: View,
    columns: usize,
    rows: usize,
    cells: Vec<f32>,
    /// Some pixel shows DWD rain.
    pub any_dwd: bool,
    /// Every pixel shows only DWD rain: RainViewer is not needed.
    pub all_dwd: bool,
}

impl Weights {
    pub fn new(view: &View) -> Self {
        let (columns, rows) = (view.width as usize / CELL + 2, view.height as usize / CELL + 2);
        let cells: Vec<f32> = (0..columns * rows)
            .into_par_iter()
            .map(|i| {
                let (lon, lat) = view.lonlat_of(((i % columns) * CELL) as f64, ((i / columns) * CELL) as f64);
                weight(lon, lat)
            })
            .collect();
        Self {
            view: *view,
            columns,
            rows,
            any_dwd: cells.iter().any(|w| *w > 0.0),
            all_dwd: cells.iter().all(|w| *w >= 1.0),
            cells,
        }
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    fn at(&self, x: usize, y: usize) -> f32 {
        let (fx, fy) = (x as f32 / CELL as f32, y as f32 / CELL as f32);
        let (x0, y0) = ((fx as usize).min(self.columns - 2), (fy as usize).min(self.rows - 2));
        let (kx, ky) = (fx - x0 as f32, fy - y0 as f32);
        let c = |x: usize, y: usize| self.cells[y * self.columns + x];
        let top = c(x0, y0) + (c(x0 + 1, y0) - c(x0, y0)) * kx;
        let bottom = c(x0, y0 + 1) + (c(x0 + 1, y0 + 1) - c(x0, y0 + 1)) * kx;
        top + (bottom - top) * ky
    }
}

/// Where the two sources meet, drawn as a dashed line while the pointer rests on the legend.
const SEAM_COLOR: [u8; 4] = [250, 179, 135, 230];

/// The two pictures of a view, mixed by the weights. A source that is `None` counts as dry there.
pub fn blend(dwd: Option<&[u8]>, rv: Option<&[u8]>, weights: &Weights, seam: bool) -> Vec<u8> {
    let (width, height) = (weights.view.width as usize, weights.view.height as usize);
    let mut out = vec![0u8; width * height * 4];
    out.par_chunks_mut(width * 4).enumerate().for_each(|(y, line)| {
        for (x, pixel) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let w = weights.at(x, y);
            let i = (y * width + x) * 4;
            if seam && (w - 0.5).abs() < 0.035 && ((x + y) / 6) % 2 == 0 {
                *pixel = SEAM_COLOR;
                continue;
            }
            let pick = |p: Option<&[u8]>| {
                p.and_then(|p| p.get(i..i + 4)).map_or([0.0; 4], |c| {
                    [f32::from(c[0]), f32::from(c[1]), f32::from(c[2]), f32::from(c[3])]
                })
            };
            let (d, r) = (pick(dwd), pick(rv));
            let (ad, ar) = (d[3] * w, r[3] * (1.0 - w));
            let alpha = ad + ar;
            if alpha < 0.5 {
                continue;
            }
            for c in 0..3 {
                pixel[c] = ((d[c] * ad + r[c] * ar) / alpha).round() as u8;
            }
            pixel[3] = alpha.round().min(255.0) as u8;
        }
    });
    out
}

/// False when a RainViewer tile lies wholly where the DWD counts, so it need not be fetched.
pub fn tile_needed(z: i32, x: i64, y: i64) -> bool {
    let n = f64::from(1u32 << z.clamp(0, 30));
    (0..=3).any(|j| {
        (0..=3).any(|i| {
            let (lon, lat) = from_unit((x as f64 + f64::from(i) / 3.0) / n, (y as f64 + f64::from(j) / 3.0) / n);
            weight(lon, lat) < 1.0
        })
    })
}

/// The times of the joint timeline, every 5 minutes: those of the DWD when it has frames, else RainViewer's past and up to
/// an hour beyond, which is moved on along its motion.
pub fn times(dwd: &[DateTime<Utc>], rv: &[DateTime<Utc>]) -> Vec<DateTime<Utc>> {
    if !dwd.is_empty() {
        return dwd.to_vec();
    }
    let (Some(first), Some(last)) = (rv.first(), rv.last()) else {
        return Vec::new();
    };
    let end = *last + TimeDelta::minutes(AHEAD_MINUTES as i64);
    std::iter::successors(Some(*first), |t| Some(*t + STEP))
        .take_while(|t| *t <= end)
        .collect()
}

/// The place of a time among RainViewer's frames, as a fractional index; beyond the last it keeps counting.
pub fn rv_position(rv: &[DateTime<Utc>], time: DateTime<Utc>) -> Option<f32> {
    let first = *rv.first()?;
    let step = match rv {
        [a, b, ..] => (*b - *a).num_seconds().max(1),
        _ => 600,
    };
    Some(((time - first).num_seconds() as f64 / step as f64) as f32)
}

/// How strongly RainViewer's rain shows `minutes` after its last frame: fully for half an hour, then fading out.
pub fn ahead_fade(minutes: f64) -> f32 {
    let half = AHEAD_MINUTES / 2.0;
    (1.0 - (minutes - half) / half).clamp(0.0, 1.0) as f32
}

/// The time at a fractional place of a timeline.
pub fn time_at(times: &[DateTime<Utc>], position: f32) -> Option<DateTime<Utc>> {
    let max = times.len().checked_sub(1)?;
    let position = position.clamp(0.0, max as f32);
    let index = position as usize;
    let next = times.get(index + 1).copied().unwrap_or(times[index]);
    let span = (next - times[index]).num_milliseconds() as f32;
    Some(times[index] + TimeDelta::milliseconds((span * (position - index as f32)) as i64))
}

/// The fractional place of a time in a timeline, for keeping the moment when the timeline changes.
pub fn position_of(times: &[DateTime<Utc>], time: DateTime<Utc>) -> Option<f32> {
    let after = times.iter().position(|t| *t > time)?;
    if after == 0 {
        return Some(0.0);
    }
    let (a, b) = (times[after - 1], times[after]);
    let k = (time - a).num_milliseconds() as f32 / (b - a).num_milliseconds().max(1) as f32;
    Some((after - 1) as f32 + k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(lon: f64, lat: f64, zoom: f64) -> View {
        View {
            lon,
            lat,
            zoom,
            width: 400,
            height: 300,
        }
    }

    #[test]
    fn germany_is_dwd_poland_is_rainviewer_and_the_seam_lies_between() {
        assert_eq!(weight(10.0, 51.0), 1.0, "Kassel");
        assert_eq!(weight(21.0, 52.2), 0.0, "Warsaw");
        assert_eq!(weight(2.35, 48.86), 0.0, "Paris");
        // East of Dresden the reach ends near the Polish border's far side.
        let edge = (0..400)
            .map(|i| 13.77 + f64::from(i) * 0.01)
            .find(|lon| weight(*lon, 51.12) < 0.5)
            .unwrap();
        let km = (edge - 13.77) * 111.32 * 51.12f64.to_radians().cos();
        assert!((km - 150.0).abs() < 3.0, "{km}");
    }

    #[test]
    fn the_weights_of_a_view_know_what_is_needed() {
        let inside = Weights::new(&view(10.0, 51.0, 9.0));
        assert!(inside.any_dwd && inside.all_dwd);
        let spain = Weights::new(&view(-3.7, 40.4, 7.0));
        assert!(!spain.any_dwd && !spain.all_dwd);
        let europe = Weights::new(&view(12.0, 50.0, 5.0));
        assert!(europe.any_dwd && !europe.all_dwd);
    }

    #[test]
    fn blending_takes_each_source_where_it_counts() {
        let w = Weights::new(&view(17.0, 51.2, 6.0));
        let size = 400 * 300 * 4;
        let (dwd, rv) = (vec![10u8; size], vec![200u8; size]);
        let out = blend(Some(&dwd), Some(&rv), &w, false);
        let px = |x: usize| &out[(150 * 400 + x) * 4..][..4];
        assert_eq!(px(0)[0], 10, "the west of the view lies in Germany");
        assert_eq!(px(399)[0], 200, "the east lies in Poland");
        let dry = blend(None, None, &w, false);
        assert!(dry.iter().all(|v| *v == 0));
    }

    #[test]
    fn a_tile_inside_the_reach_is_not_fetched() {
        // Level 8 around Kassel, and one over Warsaw.
        let tile = |lon: f64, lat: f64| {
            let (x, y) = crate::map::to_unit(lon, lat);
            ((x * 256.0) as i64, (y * 256.0) as i64)
        };
        let (x, y) = tile(9.5, 51.3);
        assert!(!tile_needed(8, x, y));
        let (x, y) = tile(21.0, 52.2);
        assert!(tile_needed(8, x, y));
    }

    #[test]
    fn without_dwd_frames_the_timeline_runs_an_hour_past_rainviewer() {
        let t0: DateTime<Utc> = "2026-11-21T10:00:00Z".parse().unwrap();
        let rv: Vec<_> = (0..13).map(|i| t0 + TimeDelta::minutes(10 * i)).collect();
        let t = times(&[], &rv);
        assert_eq!(t.first(), Some(&t0));
        assert_eq!(t.last(), Some(&(t0 + TimeDelta::minutes(180))));
        assert_eq!(t[1] - t[0], STEP);
        assert_eq!(rv_position(&rv, t0 + TimeDelta::minutes(25)), Some(2.5));
        assert_eq!(ahead_fade(20.0), 1.0);
        assert!((ahead_fade(45.0) - 0.5).abs() < 1e-6);
        assert_eq!(ahead_fade(70.0), 0.0);
    }

    #[test]
    fn a_moment_keeps_its_place_when_the_timeline_changes() {
        let t0: DateTime<Utc> = "2026-11-21T10:00:00Z".parse().unwrap();
        let five: Vec<_> = (0..10).map(|i| t0 + TimeDelta::minutes(5 * i)).collect();
        let later: Vec<_> = five.iter().skip(2).copied().collect();
        let moment = time_at(&five, 4.5).unwrap();
        assert_eq!(moment, t0 + TimeDelta::seconds(22 * 60 + 30));
        assert_eq!(position_of(&later, moment), Some(2.5));
    }
}
