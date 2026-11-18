//! Draws the warning shapes into a layer picture, so they lie on the map like the other layers and stay sharp at every zoom.

use resvg::tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::map::Extent;
use crate::sources::warning_areas::WarningMap;

const FILL_ALPHA: f32 = 0.78;
const EDGE_DARKER: f32 = 0.6;

/// A straight RGBA picture of `width` × `height` that shows `extent` of the map.
pub fn draw(map: &WarningMap, extent: &Extent, width: u32, height: u32) -> Vec<u8> {
    let Some(mut pixmap) = Pixmap::new(width.max(1), height.max(1)) else {
        return vec![0; (width * height * 4) as usize];
    };
    let (sx, sy) = (
        f64::from(width) / (extent.x1 - extent.x0),
        f64::from(height) / (extent.y1 - extent.y0),
    );
    for area in &map.areas {
        let e = area.outline.extent;
        if e.x1 < extent.x0 || e.x0 > extent.x1 || e.y1 < extent.y0 || e.y0 > extent.y1 {
            continue;
        }
        let mut path = PathBuilder::new();
        for ring in area.outline.rings() {
            for (index, (x, y)) in ring.iter().enumerate() {
                let (px, py) = (((x - extent.x0) * sx) as f32, ((y - extent.y0) * sy) as f32);
                if index == 0 {
                    path.move_to(px, py);
                } else {
                    path.line_to(px, py);
                }
            }
            path.close();
        }
        let Some(path) = path.finish() else {
            continue;
        };
        let c = area.color;
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(c.0, c.1, c.2, (FILL_ALPHA * 255.0) as u8));
        pixmap.fill_path(&path, &paint, FillRule::EvenOdd, Transform::identity(), None);
        let darker = |v: u8| (f32::from(v) * EDGE_DARKER) as u8;
        paint.set_color(Color::from_rgba8(darker(c.0), darker(c.1), darker(c.2), 255));
        pixmap.stroke_path(
            &path,
            &paint,
            &Stroke {
                width: 1.0,
                ..Stroke::default()
            },
            Transform::identity(),
            None,
        );
    }
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_area_lands_where_it_lies_and_nothing_else_is_painted() {
        let map = crate::sources::warning_areas::sample_map();
        let area = &map.areas.last().map(|a| a.outline.extent).unwrap();
        let pad = (area.x1 - area.x0) * 0.5;
        let extent = Extent {
            x0: area.x0 - pad,
            y0: area.y0 - pad,
            x1: area.x1 + pad,
            y1: area.y1 + pad,
        };
        let picture = draw(&map, &extent, 100, 100);
        let alpha = |x: usize, y: usize| picture[(y * 100 + x) * 4 + 3];
        assert!(alpha(50, 50) > 150, "the middle of the district");
        assert_eq!(alpha(2, 2), 0, "outside of it");
    }
}
