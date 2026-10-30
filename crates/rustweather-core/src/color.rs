//! The temperature colors of the tray tile: the app's own scale, independent of any weather service.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.trim().strip_prefix('#')?;
        if hex.len() != 6 {
            return None;
        }
        let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
        Some(Self(byte(0)?, byte(2)?, byte(4)?))
    }
}

const SCALE: [(f32, Rgb); 8] = [
    (-15.0, Rgb(0xb4, 0xbe, 0xfe)),
    (-5.0, Rgb(0x89, 0xb4, 0xfa)),
    (3.0, Rgb(0x74, 0xc7, 0xec)),
    (10.0, Rgb(0x94, 0xe2, 0xd5)),
    (16.0, Rgb(0xa6, 0xe3, 0xa1)),
    (22.0, Rgb(0xf9, 0xe2, 0xaf)),
    (28.0, Rgb(0xfa, 0xb3, 0x87)),
    (34.0, Rgb(0xf3, 0x8b, 0xa8)),
];

/// Deep colors for the tray tile: white digits stay readable on every one of them, on dark and light panels alike.
const TILE_SCALE: [(f32, Rgb); 8] = [
    (-15.0, Rgb(0x5a, 0x4f, 0xcf)),
    (-5.0, Rgb(0x35, 0x6f, 0xd6)),
    (3.0, Rgb(0x1f, 0x7f, 0xbf)),
    (10.0, Rgb(0x13, 0x85, 0x7c)),
    (16.0, Rgb(0x2f, 0x85, 0x3f)),
    (22.0, Rgb(0xa8, 0x6e, 0x0b)),
    (28.0, Rgb(0xcc, 0x55, 0x0c)),
    (34.0, Rgb(0xd0, 0x2b, 0x4a)),
];

/// A soft color for every temperature in Celsius, for charts on the app's background.
pub fn temperature(celsius: f32) -> Rgb {
    interpolate(&SCALE, celsius)
}

/// The deep color of the tray tile and the big temperature in the app.
pub fn tile(celsius: f32) -> Rgb {
    interpolate(&TILE_SCALE, celsius)
}

fn interpolate(scale: &[(f32, Rgb); 8], celsius: f32) -> Rgb {
    let mut low = scale[0];
    for high in *scale {
        if celsius <= high.0 {
            if high.0 <= low.0 {
                return high.1;
            }
            let k = (celsius - low.0) / (high.0 - low.0);
            let mix = |a: u8, b: u8| {
                (f32::from(a) + (f32::from(b) - f32::from(a)) * k)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            return Rgb(mix(low.1.0, high.1.0), mix(low.1.1, high.1.1), mix(low.1.2, high.1.2));
        }
        low = high;
    }
    low.1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_and_middle_of_the_scale() {
        assert_eq!(temperature(-40.0), SCALE[0].1);
        assert_eq!(temperature(50.0), SCALE[7].1);
        assert_eq!(temperature(10.0), SCALE[3].1);
        assert_ne!(temperature(14.0), temperature(16.0));
    }

    fn luminance(c: Rgb) -> f32 {
        let linear = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(c.0) + 0.7152 * linear(c.1) + 0.0722 * linear(c.2)
    }

    #[test]
    fn white_digits_are_readable_on_every_tile() {
        for t in -30..45 {
            let contrast = 1.05 / (luminance(tile(t as f32)) + 0.05);
            assert!(contrast >= 3.5, "{t} °C: contrast {contrast}");
        }
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(Rgb::parse(&Rgb(1, 2, 255).hex()), Some(Rgb(1, 2, 255)));
        assert_eq!(Rgb::parse("nope"), None);
    }
}
