//! The radar's worker thread. The UI sends commands, the worker keeps the frames, renders pictures and reports events; the
//! newest command always wins, so dragging the map never queues work. It fetches only while the radar is on screen.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rayon::prelude::*;

use super::dwd::{self, Lut, Projector};
use super::motion::{self, Field};
use super::palette::{self, Ramp};
use super::rainviewer;
use super::{FrameInfo, FrameState};
use crate::error::{Error, Result};
use crate::http::Http;
use crate::map::tiles::{self, Layer, Store};
use crate::map::{View, in_germany, labels, vector};
use crate::settings::{Basemap, RadarSource, RainPalette};

const REFRESH: Duration = Duration::from_secs(5 * 60);
const SLOW_AFTER: Duration = Duration::from_secs(6);
const RETRIES: [u64; 4] = [10, 30, 60, 300];
const OVERLAY_DELAY: Duration = Duration::from_millis(350);

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub source: RadarSource,
    pub basemap: Basemap,
    pub palette: RainPalette,
    pub opacity: f32,
    pub smooth: bool,
    pub labels_above_rain: bool,
    pub dark: bool,
    pub carto_key: Option<String>,
    pub alerts_layer: bool,
    pub clouds_layer: bool,
    pub lightning_layer: bool,
}

#[derive(Clone)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// Everything is fine and up to date.
    Ready {
        source: String,
        time: Option<DateTime<Utc>>,
    },
    Loading {
        done: usize,
        total: usize,
    },
    Slow,
    Offline {
        data_from: Option<DateTime<Utc>>,
        retry_in: u64,
    },
    BackOnline,
    RateLimited {
        host: String,
        retry_in: u64,
    },
    BadKey {
        service: String,
    },
    MissingKey {
        service: String,
    },
    Fallback {
        text: String,
    },
}

pub enum Event {
    Base(Picture),
    Overlay(Option<Picture>),
    Rain(Picture),
    Labels(Option<Picture>),
    Cities(Vec<(String, f32, f32)>),
    Timeline {
        frames: Vec<FrameInfo>,
        live: usize,
        source: String,
    },
    Frame {
        index: usize,
        state: FrameState,
    },
    Notice(Notice),
    Probe {
        title: String,
        text: String,
    },
}

pub enum Command {
    View(View),
    Position(f32),
    Config(Box<Config>),
    Active(bool),
    Reload,
    Probe {
        lon: f64,
        lat: f64,
    },
    PreloadGermany,
    Close,
    DwdLoaded(Result<Vec<dwd::Frame>>),
    RvLoaded(Result<Vec<rainviewer::Frame>>),
    OverlayLoaded {
        view: View,
        clouds: bool,
        result: Result<Vec<u8>>,
    },
    KeyChecked(bool),
    TileDone {
        base: bool,
        result: Result<()>,
    },
}

#[derive(Clone)]
pub struct Handle {
    tx: Sender<Command>,
}

impl Handle {
    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }
}

pub struct Setup {
    pub http: Http,
    pub cache_dir: PathBuf,
    pub runtime: tokio::runtime::Handle,
    pub view: View,
    pub config: Config,
}

pub fn start(setup: Setup, events: impl Fn(Event) + Send + 'static) -> Result<Handle> {
    let (tx, rx) = mpsc::channel();
    let handle = Handle { tx: tx.clone() };
    let projector = Projector::new()?;
    let store = Arc::new(Store::new(setup.http.clone(), &setup.cache_dir));
    let worker = Worker {
        rx,
        tx,
        events: Box::new(events),
        grid_cache: setup.cache_dir.join("radar-dwd.json"),
        http: setup.http,
        runtime: setup.runtime,
        store,
        projector,
        view: setup.view,
        ramp: palette::ramp(setup.config.palette),
        config: setup.config,
        position: f32::NAN,
        active: false,
        grid: Vec::new(),
        motions: Vec::new(),
        rv: Vec::new(),
        lut: None,
        last_fetch: None,
        fetching: None,
        was_offline: false,
        failures: 0,
        retry_at: None,
        layer_failed: false,
        key_checked: None,
        overlay_due: None,
        slow_said: false,
        rain_dirty: true,
        base_dirty: true,
    };
    std::thread::Builder::new()
        .name("radar".into())
        .spawn(move || worker.run())
        .map_err(|e| Error::io(std::path::Path::new("radar thread"), e))?;
    Ok(handle)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Dwd,
    RainViewer,
}

struct Worker {
    rx: Receiver<Command>,
    tx: Sender<Command>,
    events: Box<dyn Fn(Event) + Send>,
    grid_cache: PathBuf,
    http: Http,
    runtime: tokio::runtime::Handle,
    store: Arc<Store>,
    projector: Projector,
    view: View,
    ramp: Ramp,
    config: Config,
    position: f32,
    active: bool,
    grid: Vec<dwd::Frame>,
    motions: Vec<Option<Field>>,
    rv: Vec<rainviewer::Frame>,
    lut: Option<Lut>,
    last_fetch: Option<Instant>,
    fetching: Option<Instant>,
    was_offline: bool,
    failures: usize,
    retry_at: Option<Instant>,
    layer_failed: bool,
    key_checked: Option<String>,
    overlay_due: Option<Instant>,
    slow_said: bool,
    rain_dirty: bool,
    base_dirty: bool,
}

/// The provider's URL for a map layer; the key stays out of `id`, which names cache folders.
fn carto(style: &str, key: &str) -> Layer {
    Layer {
        id: format!("carto-{style}"),
        url: format!("https://basemaps.cartocdn.com/rastertiles/{style}/{{z}}/{{x}}/{{y}}.png?key={key}"),
        tile_size: 256,
        max_zoom: 19,
    }
}

/// True when the key gets another picture than no key at all.
pub async fn carto_key_works(http: &Http, key: &str) -> Result<bool> {
    let base = "https://basemaps.cartocdn.com/rastertiles/dark_all/0/0/0.png";
    let (with_key, without) = (http.bytes(&format!("{base}?key={key}")).await?, http.bytes(base).await?);
    Ok(with_key != without)
}

/// Warnings, lightning and clouds from the DWD's map server, one picture for the whole view.
fn overlay_url(view: &View, config: &Config, clouds: bool) -> Option<String> {
    let mut layers = Vec::new();
    if clouds {
        layers.push("dwd:Satellite_meteosat_1km_euat_rgb_day_hrv_and_night_ir108_3h");
    }
    if config.alerts_layer {
        layers.push("dwd:Warnungen_Landkreise");
    }
    if config.lightning_layer {
        layers.push("dwd:Accumulated_Flash_Geometry");
    }
    if layers.is_empty() {
        return None;
    }
    let half = 6_378_137.0 * std::f64::consts::PI;
    let scale = view.scale();
    let (cx, cy) = crate::map::to_unit(view.lon, view.lat);
    let (ux0, ux1) = (
        cx - f64::from(view.width) / 2.0 / scale,
        cx + f64::from(view.width) / 2.0 / scale,
    );
    let (uy0, uy1) = (
        cy - f64::from(view.height) / 2.0 / scale,
        cy + f64::from(view.height) / 2.0 / scale,
    );
    let (x0, x1) = ((ux0 * 2.0 - 1.0) * half, (ux1 * 2.0 - 1.0) * half);
    let (y0, y1) = ((1.0 - uy1 * 2.0) * half, (1.0 - uy0 * 2.0) * half);
    Some(format!(
        "https://maps.dwd.de/geoserver/dwd/wms?service=WMS&version=1.3.0&request=GetMap&layers={}&styles=&crs=EPSG:3857\
         &bbox={x0},{y0},{x1},{y1}&width={}&height={}&format=image/png&transparent=true",
        layers.join(","),
        view.width,
        view.height
    ))
}

impl Worker {
    fn emit(&self, event: Event) {
        (self.events)(event);
    }

    fn kind(&self) -> Kind {
        match self.config.source {
            RadarSource::Dwd => Kind::Dwd,
            RadarSource::RainViewer => Kind::RainViewer,
            RadarSource::Auto if in_germany(self.view.lon, self.view.lat) => Kind::Dwd,
            RadarSource::Auto => Kind::RainViewer,
        }
    }

    fn run(mut self) {
        loop {
            let wait = self.next_timeout();
            match self.rx.recv_timeout(wait) {
                Ok(first) => {
                    if !self.apply(first) {
                        return;
                    }
                    while let Ok(next) = self.rx.try_recv() {
                        if !self.apply(next) {
                            return;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.tick();
            if self.base_dirty {
                self.base_dirty = false;
                self.render_base();
            }
            if self.rain_dirty {
                self.rain_dirty = false;
                self.render_rain();
            }
        }
    }

    fn next_timeout(&self) -> Duration {
        if !self.active {
            return Duration::from_secs(3600);
        }
        match self.overlay_due {
            Some(t) => t.saturating_duration_since(Instant::now()).min(Duration::from_secs(1)),
            None => Duration::from_secs(1),
        }
    }

    fn apply(&mut self, command: Command) -> bool {
        match command {
            Command::Close => return false,
            Command::View(view) => {
                if view != self.view {
                    let before = self.kind();
                    self.view = view;
                    self.lut = None;
                    self.base_dirty = true;
                    self.rain_dirty = true;
                    self.overlay_due = Some(Instant::now() + OVERLAY_DELAY);
                    if self.kind() != before {
                        self.load_cached();
                        self.fetch();
                        self.publish_timeline();
                    }
                }
            }
            Command::Position(position) => {
                if position != self.position {
                    self.position = position;
                    self.rain_dirty = true;
                }
            }
            Command::Config(config) => {
                self.ramp = palette::ramp(config.palette);
                let source_changed = config.source != self.config.source;
                self.config = *config;
                self.layer_failed = false;
                self.base_dirty = true;
                self.rain_dirty = true;
                self.overlay_due = Some(Instant::now());
                self.check_key();
                if source_changed {
                    self.fetch();
                    self.publish_timeline();
                }
            }
            Command::Active(active) => {
                self.active = active;
                if !active {
                    // In the tray the app stays small: the grids can be read back from the disk in a moment.
                    self.grid = Vec::new();
                    self.motions = Vec::new();
                    self.rv = Vec::new();
                    self.lut = None;
                }
                if active {
                    self.load_cached();
                    self.publish_timeline();
                    self.base_dirty = true;
                    self.rain_dirty = true;
                    self.overlay_due = Some(Instant::now());
                    self.check_key();
                    if self.last_fetch.is_none_or(|t| t.elapsed() >= REFRESH) {
                        self.fetch();
                    }
                }
            }
            Command::Reload => {
                self.store.forget_failures();
                self.layer_failed = false;
                self.retry_at = None;
                self.fetch();
                self.base_dirty = true;
            }
            Command::Probe { lon, lat } => self.probe(lon, lat),
            Command::PreloadGermany => self.preload_germany(),
            Command::DwdLoaded(result) => match result {
                Ok(frames) => {
                    self.motions = vec![None; frames.len().saturating_sub(1)];
                    self.grid = frames;
                    self.loaded();
                }
                Err(e) => self.failed(&e),
            },
            Command::RvLoaded(result) => match result {
                Ok(frames) => {
                    self.rv = frames;
                    self.loaded();
                    self.preload_rv();
                }
                Err(e) => self.failed(&e),
            },
            Command::OverlayLoaded { view, clouds, result } => match result {
                Ok(bytes) if view == self.view => {
                    if let Some(image) = image::load_from_memory(&bytes)
                        .ok()
                        .map(image::DynamicImage::into_rgba8)
                    {
                        let (width, height) = image.dimensions();
                        self.emit(Event::Overlay(Some(Picture {
                            width,
                            height,
                            rgba: Arc::new(image.into_raw()),
                        })));
                    } else if clouds {
                        // The map server answers with an error text when its satellite data is out; the others stay.
                        self.emit(Event::Notice(Notice::Fallback {
                            text: "The DWD has no cloud picture right now".into(),
                        }));
                        self.fetch_overlay(false);
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "[Radar] A map layer failed"),
            },
            Command::KeyChecked(valid) => {
                if !valid && !self.layer_failed {
                    self.layer_failed = true;
                    self.base_dirty = true;
                    self.emit(Event::Notice(Notice::BadKey {
                        service: "CARTO".into(),
                    }));
                }
            }
            Command::TileDone { base, result } => match result {
                Ok(()) => {
                    if base {
                        self.base_dirty = true;
                    } else {
                        self.rain_dirty = true;
                        self.report_rv_progress();
                    }
                }
                Err(e @ Error::BadKey { .. }) if base => {
                    if !self.layer_failed {
                        self.layer_failed = true;
                        if let Error::BadKey { service } = e {
                            self.emit(Event::Notice(Notice::BadKey { service }));
                        }
                        self.base_dirty = true;
                    }
                }
                Err(e) => tracing::debug!(error = %e, "[Radar] A tile failed"),
            },
        }
        true
    }

    fn loaded(&mut self) {
        self.fetching = None;
        self.failures = 0;
        self.retry_at = None;
        self.slow_said = false;
        if self.was_offline {
            self.was_offline = false;
            self.emit(Event::Notice(Notice::BackOnline));
        }
        self.last_fetch = Some(Instant::now());
        self.publish_timeline();
        self.rain_dirty = true;
        self.report_ready();
    }

    fn data_time(&self) -> Option<DateTime<Utc>> {
        match self.kind() {
            Kind::Dwd => self.grid.iter().map(|f| f.time).filter(|t| *t <= Utc::now()).max(),
            Kind::RainViewer => self.rv.last().map(|f| f.time),
        }
    }

    fn report_ready(&self) {
        let source = self.source_name();
        self.emit(Event::Notice(Notice::Ready {
            source,
            time: self.data_time(),
        }));
    }

    fn source_name(&self) -> String {
        match self.kind() {
            Kind::Dwd => "DWD",
            Kind::RainViewer => "RainViewer",
        }
        .to_owned()
    }

    fn failed(&mut self, error: &Error) {
        self.fetching = None;
        let retry_in = RETRIES[self.failures.min(RETRIES.len() - 1)];
        self.failures += 1;
        let notice = match error {
            Error::RateLimited { host, retry_after_secs } => {
                let retry_in = retry_after_secs.unwrap_or(retry_in);
                self.retry_at = Some(Instant::now() + Duration::from_secs(retry_in));
                Notice::RateLimited {
                    host: host.clone(),
                    retry_in,
                }
            }
            _ => {
                self.was_offline = true;
                self.retry_at = Some(Instant::now() + Duration::from_secs(retry_in));
                Notice::Offline {
                    data_from: self.data_time(),
                    retry_in,
                }
            }
        };
        tracing::warn!(error = %error, "[Radar] Fetch failed");
        self.emit(Event::Notice(notice));
    }

    /// What the radar says about one point of the map.
    fn probe(&self, lon: f64, lat: f64) {
        let title = format!("{lat:.2}°, {lon:.2}°");
        let text = if self.kind() == Kind::RainViewer {
            "Exact values are available over Germany.".to_owned()
        } else {
            let index = (self.position.max(0.0) as usize).min(self.grid.len().saturating_sub(1));
            let rate_at = |frame: &dwd::Frame| {
                self.projector
                    .project(lon, lat)
                    .map_or(0.0, |(x, y)| dwd::rate(dwd::sample(&frame.values, x, y)))
            };
            match self.grid.get(index).map(rate_at) {
                None => "No radar data yet.".to_owned(),
                Some(rate) if rate >= 0.1 => {
                    let ends = self.grid.iter().skip(index).find(|f| rate_at(f) < 0.1).map(|f| f.time);
                    let end = ends.map_or(String::new(), |t| {
                        format!(", ends around {}", t.with_timezone(&chrono::Local).format("%H:%M"))
                    });
                    format!("{rate:.1} mm/h, {}{end}", crate::model::rain_word(rate))
                }
                Some(_) => {
                    let next = self.grid.iter().skip(index).find(|f| rate_at(f) >= 0.1).map(|f| f.time);
                    next.map_or("Dry for the next hours.".to_owned(), |t| {
                        format!("Dry, rain around {}", t.with_timezone(&chrono::Local).format("%H:%M"))
                    })
                }
            }
        };
        self.emit(Event::Probe { title, text });
    }

    /// Fetches the current map for all of Germany, down to street-free detail, for use without a network.
    fn preload_germany(&self) {
        let (base, labels_layer) = self.map_layers();
        for layer in [base, labels_layer].into_iter().flatten() {
            for z in 5..=9 {
                let view = View {
                    lon: 10.45,
                    lat: 51.2,
                    zoom: f64::from(z),
                    width: 1600 >> (9 - z),
                    height: 1800 >> (9 - z),
                };
                for (x, y) in tiles::visible(&view, &layer, z) {
                    if self.store.cached(&layer, z, x, y).is_none() {
                        self.request(&layer, z, x, y, true);
                    }
                }
            }
        }
    }

    /// A wrong key gets the same watermarked tiles as no key, with HTTP 200, so it has to be tested on purpose.
    fn check_key(&mut self) {
        let Some(key) = self.config.carto_key.clone() else {
            return;
        };
        if !self.active || self.key_checked.as_deref() == Some(key.as_str()) {
            return;
        }
        self.key_checked = Some(key.clone());
        let (http, tx) = (self.http.clone(), self.tx.clone());
        self.runtime.spawn(async move {
            if let Ok(valid) = carto_key_works(&http, &key).await {
                let _ = tx.send(Command::KeyChecked(valid));
            }
        });
    }

    fn fetch_overlay(&self, clouds: bool) {
        let view = self.view;
        let clouds = clouds && self.config.clouds_layer;
        let Some(url) = overlay_url(&view, &self.config, clouds) else {
            self.emit(Event::Overlay(None));
            return;
        };
        let (http, tx) = (self.http.clone(), self.tx.clone());
        self.runtime.spawn(async move {
            let _ = tx.send(Command::OverlayLoaded {
                view,
                clouds,
                result: http.bytes(&url).await,
            });
        });
    }

    fn tick(&mut self) {
        if !self.active {
            return;
        }
        if self.overlay_due.is_some_and(|t| t <= Instant::now()) {
            self.overlay_due = None;
            self.fetch_overlay(true);
        }
        if let Some(started) = self.fetching
            && started.elapsed() > SLOW_AFTER
            && !self.slow_said
        {
            self.slow_said = true;
            self.emit(Event::Notice(Notice::Slow));
        }
        let due = self.retry_at.is_some_and(|t| t <= Instant::now())
            || (self.retry_at.is_none() && self.last_fetch.is_some_and(|t| t.elapsed() >= REFRESH));
        if due {
            self.retry_at = None;
            self.fetch();
        }
    }

    /// The last radar from the disk, so the window has rain before the network answers.
    fn load_cached(&mut self) {
        if self.kind() == Kind::Dwd
            && self.grid.is_empty()
            && let Some(frames) = dwd::load_cached(&self.grid_cache)
        {
            self.motions = vec![None; frames.len().saturating_sub(1)];
            self.grid = frames;
        }
    }

    fn fetch(&mut self) {
        if !self.active || self.fetching.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
            return;
        }
        self.fetching = Some(Instant::now());
        self.emit(Event::Notice(Notice::Loading { done: 0, total: 1 }));
        let (http, tx) = (self.http.clone(), self.tx.clone());
        match self.kind() {
            Kind::Dwd => {
                let cache = self.grid_cache.clone();
                self.runtime.spawn(async move {
                    let _ = tx.send(Command::DwdLoaded(dwd::fetch(&http, &cache).await));
                });
            }
            Kind::RainViewer => {
                self.runtime.spawn(async move {
                    let _ = tx.send(Command::RvLoaded(rainviewer::frames(&http).await));
                });
            }
        }
    }

    fn publish_timeline(&mut self) {
        let now = Utc::now();
        let (times, states): (Vec<_>, Vec<_>) = match self.kind() {
            Kind::Dwd => self.grid.iter().map(|f| (f.time, FrameState::Ready)).unzip(),
            Kind::RainViewer => self.rv.iter().map(|f| (f.time, self.rv_state(f))).unzip(),
        };
        let live = times.iter().rposition(|t| *t <= now).unwrap_or(0);
        if self.position.is_nan() || self.position as usize >= times.len() {
            self.position = live as f32;
        }
        let frames = times
            .into_iter()
            .zip(states)
            .map(|(time, state)| FrameInfo {
                time,
                forecast: time > now,
                state,
            })
            .collect();
        let source = self.source_name();
        self.emit(Event::Timeline { frames, live, source });
    }

    fn rv_state(&self, frame: &rainviewer::Frame) -> FrameState {
        let z = frame.layer.level(self.view.zoom);
        let all = tiles::visible(&self.view, &frame.layer, z)
            .into_iter()
            .all(|(x, y)| self.store.cached(&frame.layer, z, x.rem_euclid(1 << z), y).is_some());
        if all { FrameState::Ready } else { FrameState::Missing }
    }

    /// Newest first: the live picture comes before the history.
    fn preload_rv(&self) {
        for frame in self.rv.iter().rev() {
            let z = frame.layer.level(self.view.zoom);
            for (x, y) in tiles::visible(&self.view, &frame.layer, z) {
                let x = x.rem_euclid(1 << z);
                if self.store.cached(&frame.layer, z, x, y).is_none() {
                    self.request(&frame.layer, z, x, y, false);
                }
            }
        }
    }

    fn report_rv_progress(&self) {
        let states: Vec<FrameState> = self.rv.iter().map(|f| self.rv_state(f)).collect();
        for (index, state) in states.iter().enumerate() {
            self.emit(Event::Frame { index, state: *state });
        }
        let done = states.iter().filter(|s| **s == FrameState::Ready).count();
        if done == states.len() {
            self.report_ready();
        } else {
            self.emit(Event::Notice(Notice::Loading {
                done,
                total: states.len(),
            }));
        }
    }

    fn request(&self, layer: &Layer, z: i32, x: i64, y: i64, base: bool) {
        let tx = self.tx.clone();
        self.store.request(&self.runtime, layer, z, x, y, move |result| {
            let _ = tx.send(Command::TileDone { base, result });
        });
    }

    /// Base layer without labels, and a labels layer to lay over the rain, if the map style has them.
    fn map_layers(&self) -> (Option<Layer>, Option<Layer>) {
        if self.layer_failed {
            return (None, None);
        }
        let split = self.config.labels_above_rain;
        match (self.config.basemap, self.config.carto_key.as_deref()) {
            (Basemap::Own, _) => (None, None),
            (Basemap::Osm, _) => (
                Some(Layer {
                    id: "osm".into(),
                    url: "https://tile.openstreetmap.org/{z}/{x}/{y}.png".into(),
                    tile_size: 256,
                    max_zoom: 19,
                }),
                None,
            ),
            (Basemap::CartoDark, Some(key)) if split => {
                (Some(carto("dark_nolabels", key)), Some(carto("dark_only_labels", key)))
            }
            (Basemap::CartoDark, Some(key)) => (Some(carto("dark_all", key)), None),
            (Basemap::CartoVoyager, Some(key)) if split => (
                Some(carto("voyager_nolabels", key)),
                Some(carto("voyager_only_labels", key)),
            ),
            (Basemap::CartoVoyager, Some(key)) => (Some(carto("voyager", key)), None),
            (Basemap::CartoDark | Basemap::CartoVoyager, None) => (None, None),
        }
    }

    fn render_base(&mut self) {
        let view = self.view;
        let colors = if self.config.dark { vector::DARK } else { vector::LIGHT };
        let mut rgba = vector::draw(&view, colors);
        let (base, labels_layer) = self.map_layers();
        let no_key = self.config.carto_key.is_none()
            && matches!(self.config.basemap, Basemap::CartoDark | Basemap::CartoVoyager);
        if no_key {
            self.emit(Event::Notice(Notice::MissingKey {
                service: "CARTO".into(),
            }));
        }
        let own = base.is_none();
        if let Some(layer) = &base {
            for (z, x, y) in tiles::compose(&view, layer, &self.store, &mut rgba, 1.0) {
                self.request(layer, z, x, y, true);
            }
        }
        self.emit(Event::Base(Picture {
            width: view.width,
            height: view.height,
            rgba: Arc::new(rgba),
        }));
        self.emit(Event::Cities(if own { labels::visible(&view) } else { Vec::new() }));
        let labels_picture = labels_layer.map(|layer| {
            let mut rgba = vec![0u8; (view.width * view.height * 4) as usize];
            for (z, x, y) in tiles::compose(&view, &layer, &self.store, &mut rgba, 1.0) {
                self.request(&layer, z, x, y, true);
            }
            Picture {
                width: view.width,
                height: view.height,
                rgba: Arc::new(rgba),
            }
        });
        self.emit(Event::Labels(labels_picture));
    }

    fn motion(&mut self, index: usize) -> Option<Field> {
        if self.motions.get(index)?.is_none() {
            let field = motion::estimate(&self.grid.get(index)?.values, &self.grid.get(index + 1)?.values);
            *self.motions.get_mut(index)? = Some(field);
        }
        self.motions.get(index)?.clone()
    }

    fn render_rain(&mut self) {
        if !self.active {
            return;
        }
        let view = self.view;
        let rgba = match self.kind() {
            Kind::Dwd => self.rain_dwd(),
            Kind::RainViewer => self.rain_rv(),
        };
        self.emit(Event::Rain(Picture {
            width: view.width,
            height: view.height,
            rgba: Arc::new(rgba),
        }));
    }

    fn rain_dwd(&mut self) -> Vec<u8> {
        let view = self.view;
        let size = (view.width * view.height * 4) as usize;
        let Some(max) = self.grid.len().checked_sub(1) else {
            return vec![0; size];
        };
        let position = if self.position.is_nan() {
            0.0
        } else {
            self.position.clamp(0.0, max as f32)
        };
        let index = position as usize;
        let k = position - index as f32;
        let warp = if self.config.smooth && k > 0.01 && index < max {
            self.motion(index)
        } else {
            None
        };
        if self.lut.is_none() {
            self.lut = Some(Lut::new(&view, &self.projector));
        }
        let (Some(lut), Some(a)) = (self.lut.as_ref(), self.grid.get(index)) else {
            return vec![0; size];
        };
        let a = Arc::clone(&a.values);
        let b = self.grid.get(index + 1).map(|f| Arc::clone(&f.values));
        let index_table = rate_table();
        let (ramp, opacity) = (&self.ramp, self.config.opacity);
        let mut out = vec![0u8; size];
        out.par_chunks_mut(lut.width.max(1) * 4)
            .zip(lut.cells.par_chunks(lut.width.max(1)))
            .for_each(|(row, cells)| {
                for (px, cell) in row.as_chunks_mut::<4>().0.iter_mut().zip(cells) {
                    let [x, y] = *cell;
                    let value = match (&warp, &b) {
                        (Some(field), Some(b)) => {
                            let (vx, vy) = field.at(x, y);
                            let from = dwd::sample(&a, x - vx * k, y - vy * k);
                            let to = dwd::sample(b, x + vx * (1.0 - k), y + vy * (1.0 - k));
                            from + (to - from) * k
                        }
                        _ => dwd::sample(&a, x, y),
                    };
                    if let Some(i) = index_table.get((value * 4.0) as usize).copied().flatten() {
                        let c = ramp[usize::from(i)];
                        px[0] = c[0];
                        px[1] = c[1];
                        px[2] = c[2];
                        px[3] = (f32::from(c[3]) * opacity) as u8;
                    }
                }
            });
        out
    }

    fn rain_rv(&mut self) -> Vec<u8> {
        let view = self.view;
        let size = (view.width * view.height * 4) as usize;
        let mut out = vec![0u8; size];
        let Some(max) = self.rv.len().checked_sub(1) else {
            return out;
        };
        let position = if self.position.is_nan() {
            max as f32
        } else {
            self.position.clamp(0.0, max as f32)
        };
        let index = position as usize;
        let k = position - index as f32;
        let (a, b) = (self.rv.get(index).cloned(), self.rv.get(index + 1).cloned());
        let opacity = self.config.opacity;
        let mut missing = Vec::new();
        if let Some(frame) = &a {
            let weight = if self.config.smooth && b.is_some() {
                1.0 - k
            } else {
                1.0
            };
            for tile in tiles::compose(&view, &frame.layer, &self.store, &mut out, opacity * weight) {
                missing.push((frame.layer.clone(), tile));
            }
        }
        if let (true, Some(frame)) = (self.config.smooth && k > 0.01, &b) {
            tiles::compose(&view, &frame.layer, &self.store, &mut out, opacity * k);
        }
        for (layer, (z, x, y)) in missing {
            self.request(&layer, z, x, y, false);
        }
        out
    }
}

/// Ramp index by grid value × 4, up to about 120 mm/h.
fn rate_table() -> &'static [Option<u8>] {
    static TABLE: std::sync::OnceLock<Vec<Option<u8>>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        (0u16..4096)
            .map(|i| palette::index_of(dwd::rate(f32::from(i) / 4.0)))
            .collect()
    })
}
