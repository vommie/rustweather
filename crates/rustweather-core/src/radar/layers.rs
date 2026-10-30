//! The picture layers over the map: clouds, warnings, lightning. Each one is a picture of a stretch of the map, larger than the
//! window, and the UI draws it at its place. So it moves with the map at once, and the network is only asked when the map leaves the
//! picture. Each has a request of its own: lightning takes seconds, warnings a tenth of that.

use std::time::Duration;

use chrono::{DateTime, Utc};

use super::clouds;
use crate::error::{Error, Result};
use crate::http::Http;
use crate::map::{Extent, View};

/// How much of the view's size a picture adds on every side.
pub const MARGIN: f64 = 0.5;
/// Lightning takes the server seconds to draw.
const PICTURE_TIMEOUT: Duration = Duration::from_secs(60);
/// How far the zoom may move before a sharper picture is worth a request.
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

    fn layer(self) -> &'static str {
        match self {
            Self::Clouds => "",
            Self::Warnings => "dwd:Warnungen_Landkreise",
            Self::Lightning => "dwd:Accumulated_Flash_Geometry",
        }
    }

    /// Clouds have soft edges and cost the server seconds, so they come at half the density.
    fn density(self) -> f64 {
        if self == Self::Clouds { 0.5 } else { 1.0 }
    }
}

/// True while the picture for `extent` and `zoom` can still serve `view`.
pub fn covers(extent: &Extent, zoom: f64, view: &View) -> bool {
    extent.contains(&view.extent(0.0)) && (view.zoom - zoom).abs() < ZOOM_REACH
}

pub struct Loaded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub time: Option<DateTime<Utc>>,
}

pub async fn fetch(http: &Http, kind: Kind, extent: &Extent, view: &View) -> Result<Loaded> {
    let size = |pixels: u32, part: f64, whole: f64| {
        (f64::from(pixels) * part / whole * kind.density()).round().max(1.0) as u32
    };
    let whole = view.extent(0.0);
    let width = size(view.width, extent.x1 - extent.x0, whole.x1 - whole.x0);
    let height = size(view.height, extent.y1 - extent.y0, whole.y1 - whole.y0);
    if kind == Kind::Clouds {
        let c = clouds::fetch(http, extent, width, height).await?;
        return Ok(Loaded {
            width: c.width,
            height: c.height,
            rgba: c.rgba,
            time: Some(c.time),
        });
    }
    let url = format!(
        "https://maps.dwd.de/geoserver/dwd/wms?service=WMS&version=1.3.0&request=GetMap&layers={}&styles=&crs=EPSG:3857\
         &bbox={}&width={width}&height={height}&format=image/png&transparent=true",
        kind.layer(),
        extent.bbox_3857()
    );
    let bytes = http.bytes_within(&url, PICTURE_TIMEOUT).await?;
    let image = image::load_from_memory(&bytes)
        .map_err(|e| Error::format("DWD", e))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Ok(Loaded {
        width,
        height,
        rgba: image.into_raw(),
        time: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_serves_until_the_map_leaves_it() {
        let view = View {
            lon: 10.0,
            lat: 51.0,
            zoom: 6.0,
            width: 800,
            height: 600,
        };
        let extent = view.extent(MARGIN);
        assert!(covers(&extent, 6.0, &view.panned(250.0, 100.0)));
        assert!(!covers(&extent, 6.0, &view.panned(700.0, 0.0)));
        assert!(!covers(&extent, 6.0, &View { zoom: 7.0, ..view }));
    }
}
