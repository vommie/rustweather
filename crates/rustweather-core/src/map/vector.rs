//! The app's own map, drawn from Natural Earth borders that ship with it: works without network and without a key.

use std::sync::LazyLock;

use resvg::tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

use super::{View, to_unit};

/// Format of `world.bin`: u32 polygon count, then per polygon a u32 point count and f32 lon, lat pairs (little endian).
const DATA: &[u8] = include_bytes!("../../../../assets/map/world.bin");

struct Polygon {
    points: Vec<(f32, f32)>,
    min: (f32, f32),
    max: (f32, f32),
}

fn read_u32(bytes: &[u8], at: usize) -> Option<usize> {
    let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    usize::try_from(u32::from_le_bytes(raw)).ok()
}

fn read_f32(bytes: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn parse() -> Option<Vec<Polygon>> {
    let count = read_u32(DATA, 0)?;
    let mut at = 4;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let n = read_u32(DATA, at)?;
        at += 4;
        let mut points = Vec::with_capacity(n);
        for _ in 0..n {
            let (lon, lat) = (read_f32(DATA, at)?, read_f32(DATA, at + 4)?);
            at += 8;
            let (x, y) = to_unit(f64::from(lon), f64::from(lat));
            points.push((x as f32, y as f32));
        }
        let min = points
            .iter()
            .fold((f32::MAX, f32::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
        let max = points
            .iter()
            .fold((f32::MIN, f32::MIN), |m, p| (m.0.max(p.0), m.1.max(p.1)));
        out.push(Polygon { points, min, max });
    }
    Some(out)
}

static WORLD: LazyLock<Vec<Polygon>> = LazyLock::new(|| parse().unwrap_or_default());

#[derive(Debug, Clone, Copy)]
pub struct Colors {
    pub water: [u8; 3],
    pub land: [u8; 3],
    pub border: [u8; 3],
}

pub const DARK: Colors = Colors {
    water: [0x11, 0x11, 0x1b],
    land: [0x25, 0x25, 0x38],
    border: [0x58, 0x5b, 0x70],
};
pub const LIGHT: Colors = Colors {
    water: [0xc9, 0xdc, 0xee],
    land: [0xf2, 0xef, 0xe9],
    border: [0xa0, 0xa4, 0xa8],
};

/// Draws the view into straight RGBA.
pub fn draw(view: &View, colors: Colors) -> Vec<u8> {
    let Some(mut pix) = Pixmap::new(view.width.max(1), view.height.max(1)) else {
        return Vec::new();
    };
    let [r, g, b] = colors.water;
    pix.fill(Color::from_rgba8(r, g, b, 255));
    let s = view.scale() as f32;
    let (cx, cy) = to_unit(view.lon, view.lat);
    let (ox, oy) = (
        view.width as f32 / 2.0 - cx as f32 * s,
        view.height as f32 / 2.0 - cy as f32 * s,
    );
    let mut land = PathBuilder::new();
    for polygon in WORLD.iter() {
        let (x0, y0) = (polygon.min.0 * s + ox, polygon.min.1 * s + oy);
        let (x1, y1) = (polygon.max.0 * s + ox, polygon.max.1 * s + oy);
        if x1 < 0.0 || y1 < 0.0 || x0 > view.width as f32 || y0 > view.height as f32 {
            continue;
        }
        let mut points = polygon.points.iter().map(|p| (p.0 * s + ox, p.1 * s + oy));
        if let Some((x, y)) = points.next() {
            land.move_to(x, y);
            for (x, y) in points {
                land.line_to(x, y);
            }
            land.close();
        }
    }
    if let Some(path) = land.finish() {
        let mut paint = Paint::default();
        let [r, g, b] = colors.land;
        paint.set_color_rgba8(r, g, b, 255);
        paint.anti_alias = true;
        pix.fill_path(&path, &paint, FillRule::EvenOdd, Transform::identity(), None);
        let [r, g, b] = colors.border;
        paint.set_color_rgba8(r, g, b, 255);
        pix.stroke_path(
            &path,
            &paint,
            &Stroke {
                width: 0.8,
                ..Stroke::default()
            },
            Transform::identity(),
            None,
        );
    }
    pix.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn germany_is_on_the_map() {
        assert!(WORLD.len() > 100);
        let view = View {
            lon: 10.0,
            lat: 51.0,
            zoom: 5.0,
            width: 64,
            height: 64,
        };
        let rgba = draw(&view, DARK);
        // The middle of the picture lies on land.
        let at = (32 * 64 + 32) * 4;
        assert_eq!(&rgba[at..at + 3], &DARK.land);
    }
}
