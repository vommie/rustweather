//! The tray picture, drawn from the app's own SVG pieces: Tabler's digits on a flat tile in the temperature's color.

use resvg::{tiny_skia, usvg};

use crate::color::{self, Rgb};
use crate::model::Trend;

const DIGITS: [&str; 10] = [
    include_str!("../../../assets/tray/number-0.svgfrag"),
    include_str!("../../../assets/tray/number-1.svgfrag"),
    include_str!("../../../assets/tray/number-2.svgfrag"),
    include_str!("../../../assets/tray/number-3.svgfrag"),
    include_str!("../../../assets/tray/number-4.svgfrag"),
    include_str!("../../../assets/tray/number-5.svgfrag"),
    include_str!("../../../assets/tray/number-6.svgfrag"),
    include_str!("../../../assets/tray/number-7.svgfrag"),
    include_str!("../../../assets/tray/number-8.svgfrag"),
    include_str!("../../../assets/tray/number-9.svgfrag"),
];
const MINUS: &str = include_str!("../../../assets/tray/minus.svgfrag");
const ARROW_UP: &str = include_str!("../../../assets/tray/arrow-up.svgfrag");
const ARROW_DOWN: &str = include_str!("../../../assets/tray/arrow-down.svgfrag");

const STALE_TILE: Rgb = Rgb(0x5c, 0x60, 0x70);
const WHITE: Rgb = Rgb(0xff, 0xff, 0xff);
const RAIN_DOT: &str = "#3daee9";
const ALERT: &str = "#fdbc4b";
const OUTLINE: &str = "#1b1e24";

/// The canvas is 44 units, 2 units per pixel of a 22 px panel icon.
const CANVAS: f64 = 44.0;
/// Free space between the digits and the tile's edge, which itself fills the canvas: nearly 4 px at 22 px.
const PADDING: f64 = 7.5;
/// Line width of the digits, in canvas units, whatever their scale.
const STROKE: f64 = 3.6;
/// Space between two glyphs, in the units of Tabler's 24 box.
const GAP: f64 = 3.5;
/// Tabler's digits are 16 units tall in their 24 box.
const GLYPH_HEIGHT: f64 = 16.0;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Look {
    /// None = no data yet.
    pub temperature: Option<f32>,
    pub stale: bool,
    pub rain_soon: bool,
    pub alert: bool,
    pub trend: Trend,
    pub background: Option<Rgb>,
    pub foreground: Option<Rgb>,
}

/// One glyph: its path and where it sits horizontally in Tabler's 24 box, for optical centering.
#[derive(Clone, Copy)]
struct Glyph {
    path: &'static str,
    left: f64,
    right: f64,
}

fn digit(n: usize) -> Glyph {
    // The 1 is narrower than the others; every other digit fills 8..16.
    let (left, right) = if n == 1 { (8.0, 13.0) } else { (8.0, 16.0) };
    Glyph {
        path: DIGITS.get(n).copied().unwrap_or(DIGITS[0]),
        left,
        right,
    }
}

const MINUS_GLYPH: Glyph = Glyph {
    path: MINUS,
    left: 9.0,
    right: 15.0,
};

/// The glyphs of the rounded temperature.
fn glyphs(temperature: f32) -> Vec<Glyph> {
    let rounded = temperature.round() as i32;
    let mut out = Vec::new();
    if rounded < 0 {
        out.push(MINUS_GLYPH);
    }
    for byte in rounded.unsigned_abs().to_string().bytes() {
        out.push(digit(usize::from(byte - b'0')));
    }
    out
}

/// The digits, as large as the padding allows, centered as a group.
fn digits_svg(glyphs: &[Glyph], ink: Rgb) -> String {
    let room = CANVAS - 2.0 * PADDING;
    let width: f64 = glyphs.iter().map(|g| g.right - g.left).sum();
    let gaps = GAP * glyphs.len().saturating_sub(1) as f64;
    let scale = ((room - STROKE) / (width + gaps)).min((room - STROKE) / GLYPH_HEIGHT);
    let mut x = (CANVAS - (width + gaps) * scale) / 2.0;
    let mut out = String::new();
    for glyph in glyphs {
        let tx = x - glyph.left * scale;
        let ty = CANVAS / 2.0 - 12.0 * scale;
        let (path, stroke, color) = (glyph.path, STROKE / scale, ink.hex());
        out.push_str(&format!(
            r#"<g transform="translate({tx} {ty}) scale({scale})" fill="none" stroke="{color}" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round">{path}</g>"#
        ));
        x += (glyph.right - glyph.left + GAP) * scale;
    }
    out
}

/// The picture as SVG. White digits on a deep tile read well on dark and on light panels; signs sit in the corners.
pub fn svg(look: &Look) -> String {
    let (tile, ink) = match (look.temperature, look.stale) {
        (Some(t), false) => (
            look.background.unwrap_or_else(|| color::tile(t)),
            look.foreground.unwrap_or(WHITE),
        ),
        _ => (STALE_TILE, Rgb(0xe6, 0xe9, 0xef)),
    };
    let mut body = format!(r#"<rect width="44" height="44" rx="9" fill="{}"/>"#, tile.hex());
    match look.temperature {
        Some(t) => body.push_str(&digits_svg(&glyphs(t), ink)),
        None => body.push_str(&digits_svg(&[MINUS_GLYPH], ink)),
    }
    if look.alert {
        body.push_str(&format!(
            r#"<path d="M36 1L43.5 15H28.5Z" fill="{ALERT}" stroke="{OUTLINE}" stroke-width="1.5" stroke-linejoin="round"/>"#
        ));
    } else if look.rain_soon {
        body.push_str(&format!(
            r#"<circle cx="37" cy="7" r="6" fill="{RAIN_DOT}" stroke="{OUTLINE}" stroke-width="1.5"/>"#
        ));
    }
    let arrow = match look.trend {
        Trend::Up => Some(ARROW_UP),
        Trend::Down => Some(ARROW_DOWN),
        Trend::Steady => None,
    };
    if let Some(arrow) = arrow {
        body.push_str(&format!(
            r#"<circle cx="37" cy="37" r="7" fill="{OUTLINE}"/><g transform="translate(37 37) scale(0.46) translate(-12 -12)" fill="none" stroke="{}" stroke-width="3.4" stroke-linecap="round" stroke-linejoin="round">{arrow}</g>"#,
            WHITE.hex()
        ));
    }
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 44 44">{body}</svg>"#)
}

/// Renders to straight RGBA, `size` × `size`.
pub fn render(look: &Look, size: u32) -> Vec<u8> {
    let Ok(tree) = usvg::Tree::from_str(&svg(look), &usvg::Options::default()) else {
        return vec![0; (size * size * 4) as usize];
    };
    let Some(mut pixmap) = tiny_skia::Pixmap::new(size, size) else {
        return Vec::new();
    };
    let k = size as f32 / 44.0;
    resvg::render(&tree, tiny_skia::Transform::from_scale(k, k), &mut pixmap.as_mut());
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
    fn every_case_renders() {
        for t in [-12.0, -3.4, 0.0, 9.0, 31.0, 104.0] {
            for trend in [Trend::Up, Trend::Down, Trend::Steady] {
                let look = Look {
                    temperature: Some(t),
                    rain_soon: true,
                    trend,
                    ..Look::default()
                };
                let rgba = render(&look, 44);
                assert_eq!(rgba.len(), 44 * 44 * 4);
                assert!(rgba.chunks(4).any(|p| p[3] == 255));
            }
        }
    }

    #[test]
    fn digits_keep_their_padding() {
        for t in [-88.0, -12.0, 9.0, 31.0, 104.0] {
            let g = glyphs(t);
            let width: f64 = g.iter().map(|g| g.right - g.left).sum::<f64>() + GAP * (g.len() - 1) as f64;
            let scale =
                ((CANVAS - 2.0 * PADDING - STROKE) / width).min((CANVAS - 2.0 * PADDING - STROKE) / GLYPH_HEIGHT);
            assert!(width * scale + STROKE <= CANVAS - 2.0 * PADDING + 1e-9);
            assert!(GLYPH_HEIGHT * scale + STROKE <= CANVAS - 2.0 * PADDING + 1e-9);
        }
    }

    #[test]
    fn no_data_is_a_gray_tile() {
        assert!(svg(&Look::default()).contains(&STALE_TILE.hex()));
    }

    #[test]
    fn minus_gets_a_glyph() {
        assert_eq!(glyphs(-12.0).len(), 3);
        assert_eq!(glyphs(9.4).len(), 1);
        assert_eq!(glyphs(-0.2).len(), 1);
    }
}
