//! Which cities get a temperature label. A city shows from the zoom level at which no bigger city is close, so a label never jumps or goes
//! away while zooming in, and panning or zooming out needs no new value at all.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::temps::CityId;
use crate::map::{View, to_unit};

/// Biggest city first: name, longitude, latitude.
const DATA: &str = include_str!("../../../../assets/map/cities.tsv");
/// Wanted width of the room around a label, in pixels. A label is wider than high, so the room is too.
const SPACING: f64 = 100.0;
const FLATNESS: f64 = 0.5;
const MAX_LEVEL: usize = 13;

struct City {
    name: &'static str,
    lon: f64,
    lat: f64,
    x: f64,
    y: f64,
}

struct Index {
    cities: Vec<City>,
    /// Per zoom level, in the order of importance.
    levels: Vec<Vec<CityId>>,
}

fn read() -> Vec<City> {
    DATA.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let name = fields.next()?;
            let (lon, lat) = (fields.next()?.parse().ok()?, fields.next()?.parse().ok()?);
            let (x, y) = to_unit(lon, lat);
            Some(City { name, lon, lat, x, y })
        })
        .collect()
}

/// The first level at which a city has room: no bigger city is that close. Only bigger ones count, so a small town never keeps a city away.
fn first_levels(cities: &[City]) -> Vec<usize> {
    let spot = |city: &City, level: usize| {
        let room = (1u64 << level) as f64;
        ((city.x * room).floor() as i64, (city.y * room).floor() as i64)
    };
    let mut taken: Vec<HashMap<(i64, i64), Vec<u32>>> = vec![HashMap::new(); MAX_LEVEL + 1];
    let mut first = Vec::with_capacity(cities.len());
    for (id, city) in cities.iter().enumerate() {
        let crowded = |level: usize| {
            let width = 1.0 / (1u64 << level) as f64;
            let (cx, cy) = spot(city, level);
            (cy - 1..=cy + 1).any(|y| {
                (cx - 1..=cx + 1).any(|x| {
                    taken[level].get(&(x, y)).is_some_and(|list| {
                        list.iter().any(|other| {
                            let other = &cities[*other as usize];
                            (other.x - city.x).abs() < width && (other.y - city.y).abs() < width * FLATNESS
                        })
                    })
                })
            })
        };
        // From fine to coarse: the small lists come first, and the first crowded level ends the search.
        let level = (1..=MAX_LEVEL).rev().find(|l| crowded(*l)).map_or(1, |l| l + 1);
        first.push(level);
        for (level, map) in taken.iter_mut().enumerate().skip(1) {
            map.entry(spot(city, level)).or_default().push(id as u32);
        }
    }
    first
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let cities = read();
        let first = first_levels(&cities);
        let levels = (0..=MAX_LEVEL)
            .map(|level| {
                (0..cities.len() as CityId)
                    .filter(|id| first[*id as usize] <= level)
                    .collect()
            })
            .collect();
        Index { cities, levels }
    })
}

fn level(view: &View) -> usize {
    ((view.scale() / SPACING).log2().round() as usize).clamp(1, MAX_LEVEL)
}

/// Where a city lies on a view, if at all.
pub fn on_view(id: CityId, view: &View) -> Option<(&'static str, f64, f64)> {
    let city = index().cities.get(id as usize)?;
    let (cx, _) = to_unit(view.lon, view.lat);
    // A city of the wrapped map can lie one world to the side.
    let shift = (cx - city.x).round();
    let (_, y) = view.pixel_of(city.lon, city.lat);
    let x = (city.x + shift - cx) * view.scale() + f64::from(view.width) / 2.0;
    let inside = x >= -40.0 && y >= -12.0 && x <= f64::from(view.width) + 40.0 && y <= f64::from(view.height) + 12.0;
    inside.then_some((city.name, x, y))
}

/// The cities of a view, biggest first.
pub fn visible(view: &View) -> Vec<CityId> {
    let list = index().levels.get(level(view)).map_or(&[][..], Vec::as_slice);
    let mut out: Vec<CityId> = list.iter().copied().filter(|id| on_view(*id, view).is_some()).collect();
    out.sort_unstable();
    out
}

/// Longitude and latitude of a city.
pub fn place(id: CityId) -> Option<(f64, f64)> {
    let city = index().cities.get(id as usize)?;
    Some((city.lon, city.lat))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(zoom: f64) -> View {
        View {
            lon: 10.0,
            lat: 51.0,
            zoom,
            width: 1100,
            height: 760,
        }
    }

    fn name(id: CityId) -> &'static str {
        index().cities[id as usize].name
    }

    #[test]
    fn the_count_stays_fair_at_every_zoom() {
        for zoom in [3.0, 4.0, 5.3, 6.7, 8.0, 10.0] {
            let count = visible(&view(zoom)).len();
            assert!(count <= 200, "zoom {zoom}: {count} labels");
            assert!(count >= 10, "zoom {zoom}: {count} labels");
        }
    }

    #[test]
    fn big_cities_come_first_and_stay() {
        let wide: Vec<_> = visible(&view(5.0)).into_iter().map(name).collect();
        assert!(wide.contains(&"Berlin"), "{wide:?}");
        assert!(wide.contains(&"Paris"), "{wide:?}");
        assert!(!wide.contains(&"Ingolstadt"), "{wide:?}");
        let close = view(8.0);
        let near: Vec<_> = visible(&View {
            lon: 11.5,
            lat: 48.7,
            ..close
        })
        .into_iter()
        .map(name)
        .collect();
        assert!(near.contains(&"Ingolstadt"), "{near:?}");
        assert!(near.contains(&"Munich"), "{near:?}");
    }

    #[test]
    fn zooming_in_only_adds_cities() {
        for level in 1..MAX_LEVEL {
            let (coarse, fine) = (&index().levels[level], &index().levels[level + 1]);
            assert!(coarse.iter().all(|id| fine.binary_search(id).is_ok()));
        }
    }

    #[test]
    fn labels_of_a_level_keep_their_room() {
        let v = view(6.0);
        let room = (1u64 << level(&v)) as f64;
        let cities: Vec<_> = visible(&v).into_iter().map(|id| &index().cities[id as usize]).collect();
        for (i, a) in cities.iter().enumerate() {
            for b in &cities[i + 1..] {
                let (dx, dy) = ((a.x - b.x).abs() * room, (a.y - b.y).abs() * room);
                assert!(dx >= 1.0 || dy >= FLATNESS, "{} and {}", a.name, b.name);
            }
        }
    }
}
