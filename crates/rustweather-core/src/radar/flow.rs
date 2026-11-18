//! How rain moves between two pictures of the same view, for RainViewer, whose frames come without motion. Fading one frame into the next makes
//! every shower stand in two places at once and look as if it went back and forth; warping both along the motion lets it travel.

use rayon::prelude::*;

/// The pictures are compared at a quarter of their size.
const FACTOR: usize = 4;
/// Blocks of this many coarse pixels each get one vector.
const BLOCK: usize = 12;
const REACH: i32 = 6;
/// A block needs this much rain, as a sum of coarse alpha, to say where it went.
const SIGNAL: f32 = 2000.0;

/// Motion in pixels per frame step, one vector per block.
#[derive(Debug, Clone, Default)]
pub struct Flow {
    columns: usize,
    rows: usize,
    vectors: Vec<(f32, f32)>,
}

fn coarse(rgba: &[u8], width: usize, height: usize) -> (Vec<f32>, usize, usize) {
    let (cw, ch) = (width / FACTOR, height / FACTOR);
    let cells = (0..ch)
        .into_par_iter()
        .flat_map_iter(|y| {
            (0..cw).map(move |x| {
                let mut sum = 0u32;
                for dy in 0..FACTOR {
                    let row = (y * FACTOR + dy) * width;
                    for dx in 0..FACTOR {
                        sum += u32::from(rgba.get((row + x * FACTOR + dx) * 4 + 3).copied().unwrap_or(0));
                    }
                }
                sum as f32
            })
        })
        .collect();
    (cells, cw, ch)
}

/// The minimum of a parabola through three values, as an offset from the middle one.
fn refine(left: f32, middle: f32, right: f32) -> f32 {
    let bend = left - 2.0 * middle + right;
    if bend <= f32::EPSILON {
        0.0
    } else {
        ((left - right) / (2.0 * bend)).clamp(-0.5, 0.5)
    }
}

fn block_vector(a: &[f32], b: &[f32], cw: usize, ch: usize, bx: usize, by: usize) -> Option<(f32, f32)> {
    let cell = |g: &[f32], x: i32, y: i32| -> f32 {
        if x < 0 || y < 0 || x >= cw as i32 || y >= ch as i32 {
            return 0.0;
        }
        g[y as usize * cw + x as usize]
    };
    let (x0, y0) = ((bx * BLOCK) as i32, (by * BLOCK) as i32);
    let mut signal = 0.0;
    for y in 0..BLOCK as i32 {
        for x in 0..BLOCK as i32 {
            signal += cell(a, x0 + x, y0 + y);
        }
    }
    if signal < SIGNAL {
        return None;
    }
    let cost = |dx: i32, dy: i32| -> f32 {
        let mut sum = 0.0;
        for y in 0..BLOCK as i32 {
            for x in 0..BLOCK as i32 {
                sum += (cell(a, x0 + x, y0 + y) - cell(b, x0 + x + dx, y0 + y + dy)).abs();
            }
        }
        sum
    };
    let side = (2 * REACH + 1) as usize;
    let costs: Vec<f32> = (-REACH..=REACH)
        .flat_map(|dy| (-REACH..=REACH).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| cost(dx, dy))
        .collect();
    let best = costs
        .iter()
        .enumerate()
        .min_by(|p, q| p.1.total_cmp(q.1))
        .map(|(i, _)| i)?;
    let (bxi, byi) = (best % side, best / side);
    let at = |x: usize, y: usize| costs[y * side + x];
    let fx = if bxi > 0 && bxi + 1 < side {
        refine(at(bxi - 1, byi), at(bxi, byi), at(bxi + 1, byi))
    } else {
        0.0
    };
    let fy = if byi > 0 && byi + 1 < side {
        refine(at(bxi, byi - 1), at(bxi, byi), at(bxi, byi + 1))
    } else {
        0.0
    };
    Some((
        (bxi as f32 - REACH as f32 + fx) * FACTOR as f32,
        (byi as f32 - REACH as f32 + fy) * FACTOR as f32,
    ))
}

/// Where the rain of `a` went in `b`, two straight RGBA pictures of `width` × `height`.
pub fn estimate(a: &[u8], b: &[u8], width: usize, height: usize) -> Flow {
    let ((ca, cw, ch), (cb, _, _)) = (coarse(a, width, height), coarse(b, width, height));
    let (columns, rows) = ((cw / BLOCK).max(1), (ch / BLOCK).max(1));
    let found: Vec<Option<(f32, f32)>> = (0..columns * rows)
        .into_par_iter()
        .map(|i| block_vector(&ca, &cb, cw, ch, i % columns, i / columns))
        .collect();
    let known: Vec<(f32, f32)> = found.iter().flatten().copied().collect();
    let median = |pick: fn(&(f32, f32)) -> f32| {
        let mut values: Vec<f32> = known.iter().map(pick).collect();
        values.sort_by(f32::total_cmp);
        values.get(values.len() / 2).copied().unwrap_or(0.0)
    };
    let fallback = (median(|v| v.0), median(|v| v.1));
    let raw: Vec<(f32, f32)> = found.iter().map(|v| v.unwrap_or(fallback)).collect();
    // A light smoothing keeps neighbors from tearing the rain apart.
    let vectors = (0..columns * rows)
        .map(|i| {
            let (x, y) = ((i % columns) as i32, (i / columns) as i32);
            let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < columns && (ny as usize) < rows {
                        let v = raw[ny as usize * columns + nx as usize];
                        sx += v.0;
                        sy += v.1;
                        n += 1.0;
                    }
                }
            }
            (sx / n, sy / n)
        })
        .collect();
    Flow { columns, rows, vectors }
}

impl Flow {
    pub fn at(&self, x: f32, y: f32) -> (f32, f32) {
        if self.vectors.is_empty() {
            return (0.0, 0.0);
        }
        let size = (FACTOR * BLOCK) as f32;
        let fx = (x / size - 0.5).clamp(0.0, (self.columns - 1) as f32);
        let fy = (y / size - 0.5).clamp(0.0, (self.rows - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.columns - 1), (y0 + 1).min(self.rows - 1));
        let (kx, ky) = (fx - x0 as f32, fy - y0 as f32);
        let v = |x: usize, y: usize| self.vectors[y * self.columns + x];
        let mix = |a: f32, b: f32, k: f32| a + (b - a) * k;
        let (a, b, c, d) = (v(x0, y0), v(x1, y0), v(x0, y1), v(x1, y1));
        (
            mix(mix(a.0, b.0, kx), mix(c.0, d.0, kx), ky),
            mix(mix(a.1, b.1, kx), mix(c.1, d.1, kx), ky),
        )
    }
}

/// A pixel between pixels, with its color weighted by its alpha.
fn sample(rgba: &[u8], width: usize, height: usize, x: f32, y: f32) -> [f32; 4] {
    if x < 0.0 || y < 0.0 || x > (width - 1) as f32 || y > (height - 1) as f32 {
        return [0.0; 4];
    }
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let (kx, ky) = (x - x0 as f32, y - y0 as f32);
    let mut out = [0.0; 4];
    for (px, py, w) in [
        (x0, y0, (1.0 - kx) * (1.0 - ky)),
        (x1, y0, kx * (1.0 - ky)),
        (x0, y1, (1.0 - kx) * ky),
        (x1, y1, kx * ky),
    ] {
        let i = (py * width + px) * 4;
        let alpha = f32::from(rgba[i + 3]) / 255.0 * w;
        for c in 0..3 {
            out[c] += f32::from(rgba[i + c]) * alpha;
        }
        out[3] += alpha;
    }
    out
}

/// The picture between `a` and `b` at `k` (0 to 1): both moved along `flow` toward each other, then mixed.
pub fn blend(a: &[u8], b: &[u8], width: usize, height: usize, flow: &Flow, k: f32, opacity: f32) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 4];
    out.par_chunks_mut(width * 4).enumerate().for_each(|(y, line)| {
        for (x, pixel) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (fx, fy) = (x as f32, y as f32);
            let (vx, vy) = flow.at(fx, fy);
            let from = sample(a, width, height, fx - vx * k, fy - vy * k);
            let to = sample(b, width, height, fx + vx * (1.0 - k), fy + vy * (1.0 - k));
            let alpha = from[3] * (1.0 - k) + to[3] * k;
            if alpha <= 0.002 {
                continue;
            }
            let mut color = [0u8; 4];
            for c in 0..3 {
                color[c] = ((from[c] * (1.0 - k) + to[c] * k) / alpha).clamp(0.0, 255.0) as u8;
            }
            color[3] = (alpha * opacity * 255.0).clamp(0.0, 255.0) as u8;
            *pixel = color;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 240;
    const H: usize = 160;

    /// A round shower of strong rain around a point.
    fn shower(cx: f32, cy: f32) -> Vec<u8> {
        let mut rgba = vec![0u8; W * H * 4];
        for y in 0..H {
            for x in 0..W {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                if d < 24.0 {
                    let i = (y * W + x) * 4;
                    rgba[i..i + 4].copy_from_slice(&[40, 120, 250, 255]);
                }
            }
        }
        rgba
    }

    fn centre(rgba: &[u8]) -> (f32, f32) {
        let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
        for (i, p) in rgba.as_chunks::<4>().0.iter().enumerate() {
            let a = f32::from(p[3]);
            sx += (i % W) as f32 * a;
            sy += (i / W) as f32 * a;
            n += a;
        }
        (sx / n, sy / n)
    }

    #[test]
    fn the_flow_finds_where_the_shower_went() {
        let flow = estimate(&shower(100.0, 80.0), &shower(114.0, 74.0), W, H);
        let (vx, vy) = flow.at(100.0, 80.0);
        assert!((vx - 14.0).abs() < 3.0 && (vy + 6.0).abs() < 3.0, "{vx} {vy}");
    }

    #[test]
    fn the_shower_travels_and_does_not_go_back() {
        let (a, b) = (shower(100.0, 80.0), shower(116.0, 80.0));
        let flow = estimate(&a, &b, W, H);
        let xs: Vec<f32> = [0.0, 0.25, 0.5, 0.75, 1.0]
            .iter()
            .map(|k| centre(&blend(&a, &b, W, H, &flow, *k, 1.0)).0)
            .collect();
        assert!(xs.windows(2).all(|p| p[1] >= p[0] - 0.3), "{xs:?}");
        assert!((xs[2] - 108.0).abs() < 2.5, "halfway: {xs:?}");
        assert!((xs[0] - 100.0).abs() < 1.0 && (xs[4] - 116.0).abs() < 1.0, "{xs:?}");
    }

    #[test]
    fn no_rain_no_motion_and_no_picture() {
        let empty = vec![0u8; W * H * 4];
        let flow = estimate(&empty, &empty, W, H);
        assert_eq!(flow.at(50.0, 50.0), (0.0, 0.0));
        assert!(blend(&empty, &empty, W, H, &flow, 0.5, 1.0).iter().all(|v| *v == 0));
    }

    #[test]
    fn opacity_reaches_the_picture() {
        let a = shower(100.0, 80.0);
        let flow = estimate(&a, &a, W, H);
        let out = blend(&a, &a, W, H, &flow, 0.0, 0.5);
        let i = (80 * W + 100) * 4;
        assert!((i32::from(out[i + 3]) - 127).abs() <= 1);
    }
}
