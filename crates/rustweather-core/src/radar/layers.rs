//! The picture layers over the map: clouds, warnings, lightning. They are tiles from the DWD's map server, cached like the map: what was
//! seen once comes from memory or the disk at once, until the data behind it is new. A layer's id holds the time of its data, so new data
//! has its own tiles, and the old ones stay until the new view is complete.

use chrono::{DateTime, DurationRound as _, TimeDelta, Utc};

use super::clouds;
use crate::map::tiles::{Layer, Post};
use crate::map::{Extent, View};

/// How much of the view's size a picture adds on every side, so short pans need no work at all.
pub const MARGIN: f64 = 0.5;
/// How far the zoom may move before the picture is put together again, for sharpness.
const ZOOM_REACH: f64 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Clouds,
    Warnings,
    Lightning,
}

pub const KINDS: [Kind; 3] = [Kind::Clouds, Kind::Warnings, Kind::Lightning];

impl Kind {
    pub fn index(self) -> usize {
        match self {
            Self::Clouds => 0,
            Self::Warnings => 1,
            Self::Lightning => 2,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Clouds => "clouds",
            Self::Warnings => "warnings",
            Self::Lightning => "lightning",
        }
    }

    fn server_layer(self) -> &'static str {
        match self {
            Self::Clouds => clouds::LAYER,
            Self::Warnings => "dwd:Warnungen_Landkreise",
            Self::Lightning => "dwd:Accumulated_Flash_Geometry",
        }
    }

    /// Clouds have soft edges, so half the density is enough.
    fn density(self) -> f64 {
        if self == Self::Clouds { 0.5 } else { 1.0 }
    }

    /// The finest level worth asking for. Lightning takes the server 4.5 s per tile, whatever its size.
    fn max_level(self) -> i32 {
        match self {
            Self::Clouds => 8,
            Self::Warnings => 10,
            Self::Lightning => 5,
        }
    }
}

/// The time that names the current data of a layer. Clouds have none of their own: `clouds::latest` finds it.
pub fn bucket(kind: Kind, now: DateTime<Utc>) -> DateTime<Utc> {
    let step = if kind == Kind::Lightning { 5 } else { 10 };
    now.duration_trunc(TimeDelta::minutes(step)).unwrap_or(now)
}

pub fn layer(kind: Kind, time: DateTime<Utc>) -> Layer {
    let extra = if kind == Kind::Clouds {
        format!("&time={}", clouds::stamp(time))
    } else {
        "&transparent=true".to_owned()
    };
    Layer {
        post: if kind == Kind::Clouds { Post::Clouds } else { Post::None },
        id: format!("layer-{}/{}", kind.name(), time.format("%Y%m%dT%H%M")),
        url: format!(
            "https://maps.dwd.de/geoserver/dwd/wms?service=WMS&version=1.3.0&request=GetMap&layers={}&styles=&crs=EPSG:3857\
             &bbox={{bbox}}&width=512&height=512&format=image/png{extra}",
            kind.server_layer()
        ),
        tile_size: 512,
        max_zoom: kind.max_level(),
    }
}

/// The picture a layer is put together in: larger than the view, at the layer's density.
pub fn big_view(view: &View, kind: Kind) -> View {
    let density = kind.density();
    let size = |pixels: u32| ((f64::from(pixels) * (1.0 + 2.0 * MARGIN) * density).round() as u32).max(1);
    View {
        width: size(view.width),
        height: size(view.height),
        zoom: view.zoom + density.log2(),
        ..*view
    }
}

/// True while the picture for `extent` and `zoom` can still serve `view`.
pub fn covers(extent: &Extent, zoom: f64, view: &View) -> bool {
    extent.contains(&view.extent(0.0)) && (view.zoom - zoom).abs() < ZOOM_REACH
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        View {
            lon: 10.0,
            lat: 51.0,
            zoom: 6.0,
            width: 800,
            height: 600,
        }
    }

    #[test]
    fn a_picture_serves_until_the_map_leaves_it() {
        let v = view();
        let extent = big_view(&v, Kind::Warnings).extent(0.0);
        assert!(covers(&extent, 6.0, &v.panned(250.0, 100.0)));
        assert!(!covers(&extent, 6.0, &v.panned(700.0, 0.0)));
        assert!(!covers(&extent, 6.0, &View { zoom: 7.0, ..v }));
    }

    #[test]
    fn density_changes_pixels_not_the_stretch_of_map() {
        let v = view();
        let (full, half) = (big_view(&v, Kind::Warnings), big_view(&v, Kind::Clouds));
        assert_eq!((full.width, half.width), (1600, 800));
        assert!((full.extent(0.0).x1 - half.extent(0.0).x1).abs() < 1e-9);
    }

    #[test]
    fn new_data_has_tiles_of_its_own() {
        let a = "2026-10-30T12:03:00Z".parse().unwrap();
        let b = "2026-10-30T12:12:00Z".parse().unwrap();
        assert_ne!(
            layer(Kind::Warnings, bucket(Kind::Warnings, a)).id,
            layer(Kind::Warnings, bucket(Kind::Warnings, b)).id
        );
        assert!(layer(Kind::Clouds, a).url.contains("&time=2026-10-30T12:03:00.000Z"));
    }
}
