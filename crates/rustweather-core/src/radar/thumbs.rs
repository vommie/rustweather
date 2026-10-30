//! Small pictures of every map for the map picker, without any weather on them. They all show the same stretch around the place at one
//! zoom, so their tiles are fetched once and come from the cache afterwards. Nothing is fetched until the picker opens.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::engine::Picture;
use crate::map::tiles::{self, Layer, Store};
use crate::map::{View, vector};
use crate::settings::Basemap;

const ZOOM: f64 = 5.0;
/// Twice the size on screen, so they stay sharp on a fine display.
const WIDTH: u32 = 216;
const HEIGHT: u32 = 128;
const EVERY: Duration = Duration::from_millis(200);

/// A tile that is still missing: its layer, level, column and row.
pub type Wanted = (Layer, i32, i64, i64);

pub struct Thumbs {
    view: View,
    /// The drawn map below the tiles; the same for every picture and not cheap.
    backdrop: Vec<u8>,
    /// Positions in `Basemap::ALL` whose picture is not done yet.
    open: Vec<usize>,
    dirty: bool,
    drawn: Option<Instant>,
}

impl Thumbs {
    pub fn new(lon: f64, lat: f64, dark: bool) -> Self {
        let view = View {
            lon,
            lat,
            zoom: ZOOM,
            width: WIDTH,
            height: HEIGHT,
        };
        Self {
            view,
            backdrop: vector::draw(&view, if dark { vector::DARK } else { vector::LIGHT }),
            open: (0..Basemap::ALL.len()).collect(),
            dirty: true,
            drawn: None,
        }
    }

    pub fn tile_arrived(&mut self) {
        self.dirty = true;
    }

    pub fn due(&self) -> bool {
        self.dirty && !self.open.is_empty() && self.drawn.is_none_or(|t| t.elapsed() >= EVERY)
    }

    /// True while pictures wait for tiles: the engine then looks more often.
    pub fn busy(&self) -> bool {
        self.dirty && !self.open.is_empty()
    }

    /// The pictures that are done, and the tiles still needed. A picture shows only once it is complete: half a map would mislead.
    pub fn render(&mut self, store: &Store, carto_key: Option<&str>) -> (Vec<(usize, Picture)>, Vec<Wanted>) {
        self.dirty = false;
        self.drawn = Some(Instant::now());
        let (mut pictures, mut wanted) = (Vec::new(), Vec::new());
        let (view, backdrop) = (self.view, &self.backdrop);
        self.open.retain(|&index| {
            let mut rgba = backdrop.clone();
            if let Some(layer) = Basemap::ALL[index].layers(carto_key, false).0 {
                let z = layer.level(view.zoom);
                let missing: Vec<(i64, i64)> = tiles::visible(&view, &layer, z)
                    .into_iter()
                    .map(|(x, y)| (x.rem_euclid(1 << z), y))
                    .filter(|&(x, y)| store.cached(&layer, z, x, y).is_none())
                    .collect();
                if !missing.is_empty() {
                    wanted.extend(missing.into_iter().map(|(x, y)| (layer.clone(), z, x, y)));
                    return true;
                }
                tiles::compose(&view, &layer, store, &mut rgba, 1.0);
            }
            pictures.push((
                index,
                Picture {
                    width: view.width,
                    height: view.height,
                    rgba: Arc::new(rgba),
                },
            ));
            false
        });
        (pictures, wanted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_waits_for_its_tiles_and_the_own_map_for_nothing() {
        let dir = std::env::temp_dir().join(format!("rustweather-thumbs-{}", std::process::id()));
        let store = Store::new(crate::http::Http::new().unwrap(), &dir);
        let mut thumbs = Thumbs::new(10.0, 51.0, true);
        assert!(thumbs.due());
        let (pictures, wanted) = thumbs.render(&store, Some("k"));
        assert_eq!(pictures.iter().map(|p| p.0).collect::<Vec<_>>(), [0]);
        assert!(!wanted.is_empty());
        assert!(!thumbs.due() && !thumbs.busy());
        thumbs.tile_arrived();
        assert!(thumbs.busy());
        thumbs.drawn = None;
        assert!(thumbs.due());
    }

    #[test]
    fn carto_without_a_key_is_the_own_map_and_needs_no_tiles() {
        let dir = std::env::temp_dir().join(format!("rustweather-thumbs-nokey-{}", std::process::id()));
        let store = Store::new(crate::http::Http::new().unwrap(), &dir);
        let mut thumbs = Thumbs::new(10.0, 51.0, true);
        let (pictures, wanted) = thumbs.render(&store, None);
        assert_eq!(pictures.len(), 3);
        assert!(wanted.iter().all(|(layer, ..)| !layer.id.starts_with("carto")));
    }
}
