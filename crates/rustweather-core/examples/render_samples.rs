//! Writes sample pictures to a folder: tray icons and the own map. `cargo run --example render_samples -- <folder>`

#![allow(clippy::expect_used)]

use rustweather_core::map::{View, vector};
use rustweather_core::model::Trend;
use rustweather_core::tray_icon::{self, Look};

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let looks = [
        Look {
            temperature: Some(9.0),
            ..Look::default()
        },
        Look {
            temperature: Some(-12.0),
            trend: Trend::Down,
            ..Look::default()
        },
        Look {
            temperature: Some(31.0),
            rain_soon: true,
            ..Look::default()
        },
        Look {
            temperature: Some(18.0),
            alert: true,
            trend: Trend::Up,
            ..Look::default()
        },
        Look {
            temperature: Some(9.0),
            stale: true,
            ..Look::default()
        },
        Look::default(),
        Look {
            temperature: Some(-3.0),
            rain_soon: true,
            trend: Trend::Up,
            ..Look::default()
        },
        Look {
            temperature: Some(24.0),
            ..Look::default()
        },
        Look {
            temperature: Some(0.0),
            ..Look::default()
        },
        Look {
            temperature: Some(11.0),
            ..Look::default()
        },
    ];
    // Big, then real size on a dark and on a light panel.
    let mut sheet = image::RgbaImage::from_pixel(10 * 100, 190, image::Rgba([0x2a, 0x2e, 0x32, 255]));
    for y in 150..190 {
        for x in 0..1000 {
            sheet.put_pixel(x, y, image::Rgba([0xef, 0xf0, 0xf1, 255]));
        }
    }
    for (i, look) in looks.iter().enumerate() {
        let x = i64::from(i as u32 * 100);
        let big = image::RgbaImage::from_raw(88, 88, tray_icon::render(look, 88)).expect("size");
        image::imageops::overlay(&mut sheet, &big, x + 6, 6);
        let small = image::RgbaImage::from_raw(22, 22, tray_icon::render(look, 22)).expect("size");
        image::imageops::overlay(&mut sheet, &small, x + 20, 108);
        image::imageops::overlay(&mut sheet, &small, x + 20, 160);
        let mid = image::RgbaImage::from_raw(32, 32, tray_icon::render(look, 32)).expect("size");
        image::imageops::overlay(&mut sheet, &mid, x + 54, 104);
    }
    sheet.save(format!("{dir}/tray.png")).expect("save");
    let view = View {
        lon: 10.45,
        lat: 51.2,
        zoom: 5.6,
        width: 900,
        height: 620,
    };
    let rgba = vector::draw(&view, vector::DARK);
    image::RgbaImage::from_raw(900, 620, rgba)
        .expect("size")
        .save(format!("{dir}/own-map.png"))
        .expect("save");
}
