//! Map and radar tiles: a RAM cache over a disk cache, fetched by a few parallel workers, composed into a view.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use quick_cache::sync::Cache;
use rayon::prelude::*;
use tokio::sync::Semaphore;

use super::View;
use crate::error::Result;
use crate::http::Http;
use crate::store;

pub struct Tile {
    pub size: u32,
    pub rgba: Vec<u8>,
}

/// One tile layer. `url` has `{z}`, `{x}` and `{y}`; `id` names its cache folder and must not contain a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    pub id: String,
    pub url: String,
    pub tile_size: u32,
    pub max_zoom: i32,
}

impl Layer {
    fn url_of(&self, z: i32, x: i64, y: i64) -> String {
        self.url
            .replace("{z}", &z.to_string())
            .replace("{x}", &x.to_string())
            .replace("{y}", &y.to_string())
    }

    /// The tile zoom level that fits a view best.
    pub fn level(&self, zoom: f64) -> i32 {
        let shift = f64::from(self.tile_size / 256).log2();
        ((zoom - shift).round() as i32).clamp(0, self.max_zoom)
    }
}

fn key(layer: &str, z: i32, x: i64, y: i64) -> String {
    format!("{layer}/{z}/{x}/{y}")
}

/// A sliding window of requests per minute, so a service's limit is never hit.
struct Budget {
    per_minute: usize,
    sent: Mutex<Vec<Instant>>,
}

impl Budget {
    async fn take(&self) {
        loop {
            let wait = {
                let mut sent = self.sent.lock();
                let now = Instant::now();
                sent.retain(|t| now - *t < Duration::from_secs(60));
                if sent.len() < self.per_minute {
                    sent.push(now);
                    None
                } else {
                    sent.first().map(|t| Duration::from_secs(60).saturating_sub(now - *t))
                }
            };
            match wait {
                None => return,
                Some(d) => tokio::time::sleep(d + Duration::from_millis(50)).await,
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Fetch {
    pub pending: usize,
    pub failed: usize,
}

pub struct Store {
    http: Http,
    dir: PathBuf,
    ram: Cache<String, Arc<Tile>>,
    busy: Mutex<HashSet<String>>,
    /// Tiles that failed, with the time from which they may be tried again.
    retry_at: Mutex<HashMap<String, Instant>>,
    gate: Arc<Semaphore>,
    radar_budget: Budget,
}

fn decode(bytes: &[u8]) -> Option<Tile> {
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    Some(Tile {
        size: image.width(),
        rgba: image.into_raw(),
    })
}

impl Store {
    pub fn new(http: Http, cache_dir: &Path) -> Self {
        Self {
            http,
            dir: cache_dir.join("tiles"),
            ram: Cache::new(1024),
            busy: Mutex::new(HashSet::new()),
            retry_at: Mutex::new(HashMap::new()),
            gate: Arc::new(Semaphore::new(8)),
            radar_budget: Budget {
                per_minute: 90,
                sent: Mutex::new(Vec::new()),
            },
        }
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.png"))
    }

    /// From RAM or disk; never touches the network.
    pub fn cached(&self, layer: &Layer, z: i32, x: i64, y: i64) -> Option<Arc<Tile>> {
        let k = key(&layer.id, z, x, y);
        if let Some(tile) = self.ram.get(&k) {
            return Some(tile);
        }
        let tile = Arc::new(decode(&std::fs::read(self.path(&k)).ok()?)?);
        self.ram.insert(k, Arc::clone(&tile));
        Some(tile)
    }

    /// Fetches in the background and calls `done(ok)` afterwards.
    pub fn request(
        self: &Arc<Self>,
        runtime: &tokio::runtime::Handle,
        layer: &Layer,
        z: i32,
        x: i64,
        y: i64,
        done: impl FnOnce(Result<()>) + Send + 'static,
    ) -> bool {
        let k = key(&layer.id, z, x, y);
        if self.retry_at.lock().get(&k).is_some_and(|t| *t > Instant::now()) || !self.busy.lock().insert(k.clone()) {
            return false;
        }
        let url = layer.url_of(z, x, y);
        let limited = layer.id.starts_with("radar-");
        let store = Arc::clone(self);
        runtime.spawn(async move {
            if limited {
                store.radar_budget.take().await;
            }
            let Ok(_permit) = store.gate.acquire().await else {
                return;
            };
            let result = store.fetch(&k, &url).await;
            store.busy.lock().remove(&k);
            if result.is_err() {
                store
                    .retry_at
                    .lock()
                    .insert(k, Instant::now() + Duration::from_secs(30));
            }
            done(result);
        });
        true
    }

    async fn fetch(&self, key: &str, url: &str) -> Result<()> {
        let bytes = self.http.bytes(url).await?;
        let tile = decode(&bytes).ok_or_else(|| crate::error::Error::format("Map tiles", "not a picture"))?;
        let path = self.path(key);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        store::write_atomic(&path, &bytes)?;
        self.ram.insert(key.to_owned(), Arc::new(tile));
        Ok(())
    }

    pub fn forget_failures(&self) {
        self.retry_at.lock().clear();
    }
}

/// The tiles a view needs at level `z`.
pub fn visible(view: &View, layer: &Layer, z: i32) -> Vec<(i64, i64)> {
    let per_unit = f64::from(1u32 << z.clamp(0, 30));
    let (x0, y0) = view_unit(view, 0.0, 0.0);
    let (x1, y1) = view_unit(view, f64::from(view.width), f64::from(view.height));
    let max = per_unit as i64 - 1;
    let _ = layer;
    let (tx0, tx1) = ((x0 * per_unit).floor() as i64, (x1 * per_unit).floor() as i64);
    let (ty0, ty1) = (
        ((y0 * per_unit).floor() as i64).max(0),
        ((y1 * per_unit).floor() as i64).min(max),
    );
    // Tiles from the middle outward, so what matters most arrives first.
    let (mx, my) = ((tx0 + tx1) / 2, (ty0 + ty1) / 2);
    let mut out: Vec<(i64, i64)> = (ty0..=ty1).flat_map(|y| (tx0..=tx1).map(move |x| (x, y))).collect();
    out.sort_by_key(|&(x, y)| (x - mx).abs() + (y - my).abs());
    out
}

fn view_unit(view: &View, px: f64, py: f64) -> (f64, f64) {
    let (cx, cy) = super::to_unit(view.lon, view.lat);
    let s = view.scale();
    (
        cx + (px - f64::from(view.width) / 2.0) / s,
        cy + (py - f64::from(view.height) / 2.0) / s,
    )
}

/// A tile plus where in it the wanted tile sits: a coarser parent stands in until the right one is there.
#[derive(Clone)]
struct Source {
    tile: Arc<Tile>,
    /// Offset and scale in tile fractions.
    ox: f64,
    oy: f64,
    span: f64,
}

fn best_tile(store: &Store, layer: &Layer, z: i32, x: i64, y: i64) -> Option<Source> {
    let wrap = |v: i64, z: i32| v.rem_euclid(1i64 << z.clamp(0, 30));
    let wx = wrap(x, z);
    for up in 0..=4.min(z) {
        let (zz, shift) = (z - up, 1i64 << up);
        let (px, py) = (wx.div_euclid(shift), y.div_euclid(shift));
        if let Some(tile) = store.cached(layer, zz, px, py) {
            let span = 1.0 / shift as f64;
            return Some(Source {
                tile,
                ox: wx.rem_euclid(shift) as f64 * span,
                oy: y.rem_euclid(shift) as f64 * span,
                span,
            });
        }
    }
    None
}

#[inline]
fn sample(tile: &Tile, u: f64, v: f64) -> [f32; 4] {
    let size = tile.size as usize;
    let last = size.saturating_sub(1) as f64;
    let (fx, fy) = (
        (u * size as f64 - 0.5).clamp(0.0, last),
        (v * size as f64 - 0.5).clamp(0.0, last),
    );
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(size - 1), (y0 + 1).min(size - 1));
    let (kx, ky) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
    let px = |x: usize, y: usize| -> [f32; 4] {
        let at = (y * size + x) * 4;
        match tile.rgba.get(at..at + 4) {
            Some(p) => [f32::from(p[0]), f32::from(p[1]), f32::from(p[2]), f32::from(p[3])],
            None => [0.0; 4],
        }
    };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let mut out = [0.0; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * kx;
        let bottom = c[i] + (d[i] - c[i]) * kx;
        out[i] = top + (bottom - top) * ky;
    }
    out
}

/// Draws a layer over `dst` (straight RGBA, the view's size). Returns the tiles that are missing.
pub fn compose(view: &View, layer: &Layer, store: &Store, dst: &mut [u8], opacity: f32) -> Vec<(i32, i64, i64)> {
    let z = layer.level(view.zoom);
    let per_unit = f64::from(1u32 << z.clamp(0, 30));
    let mut sources: HashMap<(i64, i64), Source> = HashMap::new();
    let mut missing = Vec::new();
    for (x, y) in visible(view, layer, z) {
        if store.cached(layer, z, x.rem_euclid(per_unit as i64), y).is_none() {
            missing.push((z, x.rem_euclid(per_unit as i64), y));
        }
        if let Some(source) = best_tile(store, layer, z, x, y) {
            sources.insert((x, y), source);
        }
    }
    let width = view.width as usize;
    let (x0, y0) = view_unit(view, 0.0, 0.0);
    let step = per_unit / view.scale();
    dst.par_chunks_mut(width * 4).enumerate().for_each(|(py, row)| {
        let ty = (y0 * per_unit) + (py as f64 + 0.5) * step;
        let (tile_y, fy) = (ty.floor() as i64, ty - ty.floor());
        for (px, out) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let tx = (x0 * per_unit) + (px as f64 + 0.5) * step;
            let (tile_x, fx) = (tx.floor() as i64, tx - tx.floor());
            let Some(s) = sources.get(&(tile_x, tile_y)) else {
                continue;
            };
            let c = sample(&s.tile, s.ox + fx * s.span, s.oy + fy * s.span);
            let a = c[3] / 255.0 * opacity;
            if a <= 0.0 {
                continue;
            }
            let under = f32::from(out[3]) / 255.0;
            let total = a + under * (1.0 - a);
            for i in 0..3 {
                let mixed = (c[i] * a + f32::from(out[i]) * under * (1.0 - a)) / total;
                out[i] = mixed.clamp(0.0, 255.0) as u8;
            }
            out[3] = (total * 255.0).clamp(0.0, 255.0) as u8;
        }
    });
    missing
}

/// Cache use in bytes of the folder `dir`, for Settings > Cache.
pub fn folder_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => folder_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

fn walk(dir: &Path, out: &mut Vec<(std::time::SystemTime, u64, PathBuf)>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            walk(&entry.path(), out);
        } else if let Ok(modified) = meta.modified() {
            out.push((modified, meta.len(), entry.path()));
        }
    }
}

/// Removes old radar files, then the oldest of everything while the cache is over its limit.
pub fn prune(cache_dir: &Path, max_bytes: u64, radar_age: Duration) {
    let mut files = Vec::new();
    walk(cache_dir, &mut files);
    files.sort_by_key(|f| f.0);
    let now = std::time::SystemTime::now();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    for (modified, size, path) in files {
        let is_radar = path.to_string_lossy().contains("radar");
        let expired = is_radar && now.duration_since(modified).is_ok_and(|age| age > radar_age);
        if (expired || total > max_bytes) && std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}
