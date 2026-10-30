//! How the rain moves between two grid frames: block matching on a coarse grid. The player warps both neighbors along these
//! vectors, so the rain travels instead of fading from one picture to the next.

use rayon::prelude::*;

use super::dwd::{HEIGHT, WIDTH};

const FACTOR: usize = 8;
const BLOCK: usize = 6;
const REACH: i32 = 4;
const CW: usize = WIDTH / FACTOR;
const CH: usize = HEIGHT / FACTOR;
const BW: usize = CW / BLOCK;
const BH: usize = CH / BLOCK;

/// Motion in grid cells per frame step, one vector per block of 48 km.
#[derive(Clone)]
pub struct Field {
    vectors: Vec<(f32, f32)>,
}

impl Default for Field {
    fn default() -> Self {
        Self {
            vectors: vec![(0.0, 0.0); BW * BH],
        }
    }
}

fn coarse(values: &[u16]) -> Vec<f32> {
    (0..CH)
        .into_par_iter()
        .flat_map_iter(|y| {
            (0..CW).map(move |x| {
                let mut sum = 0u32;
                for dy in 0..FACTOR {
                    for dx in 0..FACTOR {
                        sum += u32::from(
                            values
                                .get((y * FACTOR + dy) * WIDTH + x * FACTOR + dx)
                                .copied()
                                .unwrap_or(0),
                        );
                    }
                }
                sum as f32
            })
        })
        .collect()
}

fn block_vector(a: &[f32], b: &[f32], bx: usize, by: usize) -> Option<(f32, f32)> {
    let cell = |g: &[f32], x: i32, y: i32| -> f32 {
        if x < 0 || y < 0 || x >= CW as i32 || y >= CH as i32 {
            return 0.0;
        }
        g.get(y as usize * CW + x as usize).copied().unwrap_or(0.0)
    };
    let (x0, y0) = ((bx * BLOCK) as i32, (by * BLOCK) as i32);
    let mut signal = 0.0;
    for y in 0..BLOCK as i32 {
        for x in 0..BLOCK as i32 {
            signal += cell(a, x0 + x, y0 + y);
        }
    }
    if signal < 40.0 {
        return None;
    }
    let mut best = (f32::MAX, (0, 0));
    for dy in -REACH..=REACH {
        for dx in -REACH..=REACH {
            let mut cost = 0.0;
            for y in 0..BLOCK as i32 {
                for x in 0..BLOCK as i32 {
                    cost += (cell(a, x0 + x, y0 + y) - cell(b, x0 + x + dx, y0 + y + dy)).abs();
                }
            }
            // A slight preference for little motion, so noise does not win.
            cost += (dx.abs() + dy.abs()) as f32 * signal * 0.002;
            if cost < best.0 {
                best = (cost, (dx, dy));
            }
        }
    }
    Some(((best.1.0 * FACTOR as i32) as f32, (best.1.1 * FACTOR as i32) as f32))
}

fn median(mut values: Vec<f32>) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f32::total_cmp);
    values[values.len() / 2]
}

pub fn estimate(a: &[u16], b: &[u16]) -> Field {
    let (ca, cb) = (coarse(a), coarse(b));
    let found: Vec<Option<(f32, f32)>> = (0..BW * BH)
        .into_par_iter()
        .map(|i| block_vector(&ca, &cb, i % BW, i / BW))
        .collect();
    let known: Vec<(f32, f32)> = found.iter().flatten().copied().collect();
    let common = (
        median(known.iter().map(|v| v.0).collect()),
        median(known.iter().map(|v| v.1).collect()),
    );
    let mut vectors: Vec<(f32, f32)> = found.iter().map(|v| v.unwrap_or(common)).collect();
    // One round of smoothing against outliers between neighbors.
    let source = vectors.clone();
    for y in 0..BH {
        for x in 0..BW {
            let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
            for ny in y.saturating_sub(1)..(y + 2).min(BH) {
                for nx in x.saturating_sub(1)..(x + 2).min(BW) {
                    let v = source[ny * BW + nx];
                    sx += v.0;
                    sy += v.1;
                    n += 1.0;
                }
            }
            vectors[y * BW + x] = (sx / n, sy / n);
        }
    }
    Field { vectors }
}

impl Field {
    /// The vector at a grid position, interpolated between blocks.
    #[inline]
    pub fn at(&self, x: f32, y: f32) -> (f32, f32) {
        let size = (FACTOR * BLOCK) as f32;
        let (fx, fy) = (
            (x / size - 0.5).clamp(0.0, (BW - 1) as f32),
            (y / size - 0.5).clamp(0.0, (BH - 1) as f32),
        );
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(BW - 1), (y0 + 1).min(BH - 1));
        let (kx, ky) = (fx - x0 as f32, fy - y0 as f32);
        let get = |x: usize, y: usize| self.vectors.get(y * BW + x).copied().unwrap_or((0.0, 0.0));
        let (a, b, c, d) = (get(x0, y0), get(x1, y0), get(x0, y1), get(x1, y1));
        let mix = |a: f32, b: f32, k: f32| a + (b - a) * k;
        (
            mix(mix(a.0, b.0, kx), mix(c.0, d.0, kx), ky),
            mix(mix(a.1, b.1, kx), mix(c.1, d.1, kx), ky),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(cx: usize, cy: usize) -> Vec<u16> {
        let mut grid = vec![0u16; WIDTH * HEIGHT];
        for y in cy..cy + 80 {
            for x in cx..cx + 80 {
                grid[y * WIDTH + x] = 50;
            }
        }
        grid
    }

    #[test]
    fn finds_the_motion_of_a_rain_cell() {
        let field = estimate(&blob(400, 500), &blob(416, 492));
        let (dx, dy) = field.at(440.0, 540.0);
        assert!((dx - 16.0).abs() <= 8.0, "dx {dx}");
        assert!((dy + 8.0).abs() <= 8.0, "dy {dy}");
    }
}
