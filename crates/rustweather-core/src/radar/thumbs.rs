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
    /// Positions in `Basemap::ALL` whose picture is not complete yet.
    open: Vec<usize>,
    dirty: bool,
    drawn: Option<Instant>,
}

impl Thumbs {
    pub fn new(lon: f64, lat: f64) -> Self {
        Self {
            view: View {
                lon,
                lat,
                zoom: ZOOM,
                width: WIDTH,
                height: HEIGHT,
            },
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

    /// The pictures that changed, and the tiles still needed.
    pub fn render(
        &mut self,
        store: &Store,
        carto_key: Option<&str>,
        dark: bool,
    ) -> (Vec<(usize, Picture)>, Vec<Wanted>) {
        self.dirty = false;
        self.drawn = Some(Instant::now());
        let colors = if dark { vector::DARK } else { vector::LIGHT };
        let (mut pictures, mut wanted) = (Vec::new(), Vec::new());
        let view = self.view;
        self.open.retain(|&index| {
            let mut rgba = vector::draw(&view, colors);
            let mut missing = Vec::new();
            if let Some(layer) = Basemap::ALL[index].layers(carto_key, false).0 {
                missing = tiles::compose(&view, &layer, store, &mut rgba, 1.0);
                wanted.extend(missing.iter().map(|&(z, x, y)| (layer.clone(), z, x, y)));
            }
            pictures.push((
                index,
                Picture {
                    width: view.width,
                    height: view.height,
                    rgba: Arc::new(rgba),
                },
            ));
            !missing.is_empty()
        });
        (pictures, wanted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_own_map_needs_no_tiles_and_is_done_at_once() {
        let dir = std::env::temp_dir().join(format!("rustweather-thumbs-{}", std::process::id()));
        let store = Store::new(crate::http::Http::new().unwrap(), &dir);
        let mut thumbs = Thumbs::new(10.0, 51.0);
        assert!(thumbs.due());
        let (pictures, wanted) = thumbs.render(&store, None, true);
        assert_eq!(pictures.len(), Basemap::ALL.len());
        assert!(!wanted.is_empty());
        assert!(!thumbs.open.contains(&0));
        assert!(thumbs.open.contains(&1));
        assert!(!thumbs.due());
        thumbs.tile_arrived();
        thumbs.drawn = None;
        assert!(thumbs.due());
    }
}
