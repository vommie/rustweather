//! The map: Web Mercator views, tile layers, and the app's own vector map as the fallback under everything.

pub mod labels;
pub mod tiles;
pub mod vector;

use std::f64::consts::PI;

/// What the map window shows. `zoom` is continuous, in the sense of tile zoom levels for 256 px tiles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub lon: f64,
    pub lat: f64,
    pub zoom: f64,
    pub width: u32,
    pub height: u32,
}

pub const MIN_ZOOM: f64 = 3.0;
pub const MAX_ZOOM: f64 = 12.0;

/// Longitude, latitude to the unit square.
pub fn to_unit(lon: f64, lat: f64) -> (f64, f64) {
    ((lon + 180.0) / 360.0, (1.0 - lat.to_radians().tan().asinh() / PI) / 2.0)
}

pub fn from_unit(x: f64, y: f64) -> (f64, f64) {
    (x * 360.0 - 180.0, (PI * (1.0 - 2.0 * y)).sinh().atan().to_degrees())
}

impl View {
    /// Pixels per unit square.
    pub fn scale(&self) -> f64 {
        256.0 * self.zoom.exp2()
    }

    pub fn pixel_of(&self, lon: f64, lat: f64) -> (f64, f64) {
        let (x, y) = to_unit(lon, lat);
        let (cx, cy) = to_unit(self.lon, self.lat);
        let s = self.scale();
        (
            (x - cx) * s + f64::from(self.width) / 2.0,
            (y - cy) * s + f64::from(self.height) / 2.0,
        )
    }

    pub fn lonlat_of(&self, px: f64, py: f64) -> (f64, f64) {
        let (cx, cy) = to_unit(self.lon, self.lat);
        let s = self.scale();
        from_unit(
            cx + (px - f64::from(self.width) / 2.0) / s,
            cy + (py - f64::from(self.height) / 2.0) / s,
        )
    }

    /// What the view shows, plus `margin` of its own size on every side.
    pub fn extent(&self, margin: f64) -> Extent {
        let (cx, cy) = to_unit(self.lon, self.lat);
        let s = self.scale();
        let (w, h) = (f64::from(self.width) / s, f64::from(self.height) / s);
        let (dx, dy) = (w * (0.5 + margin), h * (0.5 + margin));
        Extent {
            x0: cx - dx,
            y0: cy - dy,
            x1: cx + dx,
            y1: cy + dy,
        }
    }

    /// Where an extent lies in this view, in pixels: left, top, width, height. A picture drawn there stays on its place on the map.
    pub fn rect_of(&self, extent: &Extent) -> (f64, f64, f64, f64) {
        let (cx, cy) = to_unit(self.lon, self.lat);
        let s = self.scale();
        (
            (extent.x0 - cx) * s + f64::from(self.width) / 2.0,
            (extent.y0 - cy) * s + f64::from(self.height) / 2.0,
            (extent.x1 - extent.x0) * s,
            (extent.y1 - extent.y0) * s,
        )
    }

    /// Moves the map by a drag of `dx`, `dy` pixels.
    pub fn panned(self, dx: f64, dy: f64) -> Self {
        let (cx, cy) = to_unit(self.lon, self.lat);
        let s = self.scale();
        let (lon, lat) = from_unit(cx - dx / s, (cy - dy / s).clamp(0.02, 0.98));
        Self { lon, lat, ..self }
    }

    /// Zooms by `steps` levels while the point under the pointer stays where it is.
    pub fn zoomed(self, steps: f64, px: f64, py: f64) -> Self {
        let (lon, lat) = self.lonlat_of(px, py);
        let zoom = (self.zoom + steps).clamp(MIN_ZOOM, MAX_ZOOM);
        let moved = Self { zoom, ..self };
        let (x, y) = moved.pixel_of(lon, lat);
        moved.panned(px - x, py - y)
    }
}

/// A stretch of the map in the unit square, for pictures that belong to a place on the map and not to the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Extent {
    /// True when `other` lies inside.
    pub fn contains(&self, other: &Self) -> bool {
        other.x0 >= self.x0 && other.y0 >= self.y0 && other.x1 <= self.x1 && other.y1 <= self.y1
    }

    /// `minx,miny,maxx,maxy` in Web Mercator meters.
    pub fn bbox_3857(&self) -> String {
        let half = 6_378_137.0 * PI;
        let (x0, x1) = ((self.x0 * 2.0 - 1.0) * half, (self.x1 * 2.0 - 1.0) * half);
        let (y0, y1) = ((1.0 - self.y1 * 2.0) * half, (1.0 - self.y0 * 2.0) * half);
        format!("{x0},{y0},{x1},{y1}")
    }
}

/// Germany and its radar range, where the DWD grid is the better source.
pub fn in_germany(lon: f64, lat: f64) -> bool {
    (46.0..=55.8).contains(&lat) && (3.5..=17.0).contains(&lon)
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
    fn unit_roundtrip() {
        let (lon, lat) = from_unit(to_unit(13.4, 52.5).0, to_unit(13.4, 52.5).1);
        assert!((lon - 13.4).abs() < 1e-9 && (lat - 52.5).abs() < 1e-9);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let v = view();
        let before = v.lonlat_of(200.0, 100.0);
        let after = v.zoomed(1.5, 200.0, 100.0).lonlat_of(200.0, 100.0);
        assert!((before.0 - after.0).abs() < 1e-6 && (before.1 - after.1).abs() < 1e-6);
    }

    #[test]
    fn a_picture_stays_on_its_place_when_the_map_moves() {
        let v = view();
        let extent = v.extent(0.5);
        let (x, y, w, h) = v.rect_of(&extent);
        assert!((x + 400.0).abs() < 1e-6 && (y + 300.0).abs() < 1e-6);
        assert!((w - 1600.0).abs() < 1e-6 && (h - 1200.0).abs() < 1e-6);
        let (px, py, ..) = v.panned(30.0, -20.0).rect_of(&extent);
        assert!((px - x - 30.0).abs() < 1e-6 && (py - y + 20.0).abs() < 1e-6);
        let (_, _, zw, _) = v.zoomed(1.0, 400.0, 300.0).rect_of(&extent);
        assert!((zw - 3200.0).abs() < 1e-6);
    }

    #[test]
    fn a_margin_covers_short_pans() {
        let v = view();
        let wide = v.extent(0.5);
        assert!(wide.contains(&v.panned(200.0, 100.0).extent(0.0)));
        assert!(!wide.contains(&v.panned(600.0, 0.0).extent(0.0)));
    }

    #[test]
    fn pan_moves_by_pixels() {
        let v = view();
        let p = v.pixel_of(13.0, 52.0);
        let q = v.panned(30.0, -20.0).pixel_of(13.0, 52.0);
        assert!((q.0 - p.0 - 30.0).abs() < 1e-6 && (q.1 - p.1 + 20.0).abs() < 1e-6);
    }
}
