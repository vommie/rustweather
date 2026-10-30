//! Rain colors. Index 0 is "no rain"; the scale is logarithmic from 0.1 to 100 mm/h, the way rain is perceived.

use crate::settings::RainPalette;

pub type Ramp = Vec<[u8; 4]>;

const CLASSIC: &[(f32, [u8; 4])] = &[
    (0.0, [116, 199, 236, 150]),
    (0.2, [59, 130, 246, 215]),
    (0.42, [34, 197, 94, 235]),
    (0.62, [250, 204, 21, 245]),
    (0.8, [249, 115, 22, 250]),
    (0.92, [239, 68, 68, 255]),
    (1.0, [217, 70, 239, 255]),
];
const CALM: &[(f32, [u8; 4])] = &[
    (0.0, [180, 190, 254, 150]),
    (0.25, [137, 180, 250, 210]),
    (0.45, [116, 199, 236, 230]),
    (0.6, [148, 226, 213, 240]),
    (0.75, [249, 226, 175, 250]),
    (0.88, [250, 179, 135, 255]),
    (1.0, [243, 139, 168, 255]),
];
const BLUE: &[(f32, [u8; 4])] = &[
    (0.0, [69, 89, 138, 140]),
    (0.35, [90, 120, 184, 210]),
    (0.65, [137, 180, 250, 240]),
    (1.0, [255, 255, 255, 255]),
];

/// 256 colors.
pub fn ramp(palette: RainPalette) -> Ramp {
    let stops = match palette {
        RainPalette::Classic => CLASSIC,
        RainPalette::Calm => CALM,
        RainPalette::Blue => BLUE,
    };
    (0..256u16)
        .map(|i| {
            let k = f32::from(i) / 255.0;
            let upper = stops.iter().position(|s| s.0 >= k).unwrap_or(stops.len() - 1).max(1);
            let (a, b) = (stops[upper - 1], stops[upper]);
            let f = ((k - a.0) / (b.0 - a.0)).clamp(0.0, 1.0);
            let mix = |n: usize| (f32::from(a.1[n]) + (f32::from(b.1[n]) - f32::from(a.1[n])) * f) as u8;
            [mix(0), mix(1), mix(2), mix(3)]
        })
        .collect()
}

/// Ramp index of a rain rate; `None` when it is below what counts as rain.
pub fn index_of(mmh: f32) -> Option<u8> {
    (mmh >= 0.1).then(|| ((mmh / 0.1).log10() / 3.0 * 255.0).clamp(0.0, 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_ends() {
        assert_eq!(index_of(0.05), None);
        assert_eq!(index_of(0.1), Some(0));
        assert_eq!(index_of(100.0), Some(255));
        assert_eq!(ramp(RainPalette::Classic).len(), 256);
    }
}
