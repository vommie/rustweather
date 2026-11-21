//! The radar's worker thread. The UI sends commands, the worker keeps the frames, renders pictures and reports events; the
//! newest command always wins, so dragging the map never queues work. It fetches only while the radar is on screen.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rayon::prelude::*;

use super::clouds;
use super::dwd::{self, Lut, Projector};
use super::layers::{self, KINDS, Kind};
use super::model::{self, Model};
use super::mosaic::{self, Weights};
use super::motion::{self, Field};
use super::palette::{self, Ramp};
use super::rainviewer;
use super::rv_frames::{self, RvFrames};
use super::temps;
use super::thumbs::Thumbs;
use super::wind::{self, Field as WindField, Flow, Loaded as WindLoaded};
use super::{FrameInfo, FrameState};
use super::{flow, warning_layer};
use crate::error::{Error, Result};
use crate::http::Http;
use crate::map::tiles::{self, Layer, Store};
use crate::map::{Extent, View, labels, vector};
use crate::settings::{Basemap, RadarSource, RainPalette};
use crate::sources::warning_areas::{self, WarningMap};

const REFRESH: Duration = Duration::from_secs(5 * 60);
const SLOW_AFTER: Duration = Duration::from_secs(6);
const RETRIES: [u64; 4] = [10, 30, 60, 300];
/// New tiles are shown in steps of this, not one by one.
const LAYER_FRAME: Duration = Duration::from_millis(200);
/// How often a layer looks whether its data has become new.
const LAYER_CHECK: Duration = Duration::from_secs(5);
const CLOUDS_MAX_AGE: Duration = Duration::from_secs(30 * 60);
const LAYER_RETRY: Duration = Duration::from_secs(60);
/// The frames stay in memory this long after the last radar view closed: opening the radar next uses them at once.
const KEEP_WARM: Duration = Duration::from_secs(120);
/// A view counts as settled after this, so a drag does not ask for every picture on its way.
const SETTLE: Duration = Duration::from_millis(400);
const WIND_FRAME: Duration = Duration::from_millis(33);
const WIND_DELAY: Duration = Duration::from_millis(600);
const WIND_MAX_AGE: Duration = Duration::from_secs(15 * 60);
const WIND_RETRY: Duration = Duration::from_secs(60);
const WIND_DWD_RETRY: Duration = Duration::from_secs(10 * 60);
const METEO_PAUSE: Duration = Duration::from_secs(60 * 60);
/// The warnings change a few times an hour; one answer holds all of Germany.
const WARNINGS_REFRESH: Duration = Duration::from_secs(10 * 60);
const WARNINGS_RETRY: Duration = Duration::from_secs(60);
/// Warnings from the disk older than this are not shown: they may be over.
const WARNINGS_MAX_AGE: Duration = Duration::from_secs(60 * 60);
/// A layer tile that failed is asked for again after this, once the store lets it.
/// From this zoom one cell of the DWD grid (1 km) covers more than two pixels, and is drawn smooth.
const SMOOTH_FROM_ZOOM: f64 = 7.5;
/// RainViewer tiles are asked for this far around the view (user decision 2026-11-21: half a screen on each side).
const RV_MARGIN: f64 = 0.5;
const TILE_RETRY: Duration = Duration::from_secs(32);

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
    pub temps_layer: bool,
    pub heat_layer: bool,
    pub wind_layer: bool,
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
    /// The chosen map works: whatever was said about its key is over.
    MapReady,
    Fallback {
        text: String,
    },
    /// Something on the map is missing, and the user should know why.
    Failed {
        text: String,
    },
    /// What `Failed` said is over.
    Solved,
}

pub enum Event {
    /// The map, and the view it shows: layers take their place from it, so they move in step.
    Base(Picture, View),
    /// A picture layer and the stretch of the map it shows; `None` takes it away.
    Layer {
        kind: Kind,
        picture: Option<(Picture, Extent)>,
        time: Option<DateTime<Utc>>,
    },
    Rain(Picture),
    Labels(Option<Picture>),
    Wind(Option<Picture>),
    /// City, pixel position and temperature of each label.
    Temps(Vec<(&'static str, f32, f32, f32)>),
    /// The temperature colors and the stretch of the map they show; `None` takes them away.
    Heat(Option<(Picture, Extent)>),
    /// A picture for the map picker; `index` is the place in `Basemap::ALL`.
    MapThumb {
        index: usize,
        picture: Picture,
    },
    Cities(Vec<(String, f32, f32)>),
    /// All warning shapes, so a click finds its warning at once; `None` when the layer is off.
    Warnings(Option<Arc<WarningMap>>),
    Timeline {
        frames: Vec<FrameInfo>,
        live: usize,
        source: String,
        /// Where the shown moment lies in the new timeline, when it changed.
        position: Option<f32>,
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
    /// The screen is locked: nothing is fetched or animated until it is unlocked.
    Away(bool),
    Reload,
    /// The pointer rests on the legend, or left it.
    Seam(bool),
    Probe {
        lon: f64,
        lat: f64,
    },
    PreloadGermany,
    /// The map picker opens around a place, or closes.
    MapThumbs(Option<(f64, f64)>),
    ThumbTile(Result<()>),
    Close,
    DwdLoaded(Result<Vec<dwd::Frame>>),
    RvLoaded(Result<Vec<rainviewer::Frame>>),
    LayerTile {
        kind: Kind,
    },
    LayerTileFailed {
        kind: Kind,
    },
    WarningsLoaded(Result<WarningMap>),
    CloudsTime(Result<DateTime<Utc>>),
    TempsLoaded(Result<Model>),
    KeyChecked(bool),
    WindLoaded(WindLoaded),
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
        slots: Default::default(),
        clouds_time: remembered_clouds_time(&setup.cache_dir.join("clouds-time")),
        clouds_file: setup.cache_dir.join("clouds-time"),
        clouds_asked: None,
        temp_grid: None,
        temp_file: setup.cache_dir.join("temperature-grid.bin"),
        temps_busy: false,
        temps_retry: None,
        heat_shown: None,
        thumbs: None,
        moved_at: None,
        rv_due: None,
        idle_since: None,
        key_banner: false,
        away: false,
        wind: None,
        flow: None,
        wind_at: None,
        wind_file: setup.cache_dir.join("wind-grid.bin"),
        wind_dwd_retry: None,
        meteo_pause: None,
        problem_shown: false,
        wind_busy: false,
        wind_due: None,
        wind_frame: Instant::now(),
        slow_said: false,
        rain_dirty: true,
        base_dirty: true,
        rv_frames: RvFrames::default(),
        warning_map: None,
        warning_file: setup.cache_dir.join("warnings.json"),
        warnings_busy: false,
        warnings_due: None,
        mosaic_weights: RefCell::new(None),
        seam: false,
        timeline: Vec::new(),
        dwd_busy: false,
        rv_busy: false,
        shown_source: String::new(),
    };
    std::thread::Builder::new()
        .name("radar".into())
        .spawn(move || worker.run())
        .map_err(|e| Error::io(std::path::Path::new("radar thread"), e))?;
    Ok(handle)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Dwd,
    RainViewer,
    /// Both at once, joined by the reach of the DWD's radars (`mosaic.rs`).
    Mosaic,
}

mod mixed;

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
    slots: [Slot; 3],
    clouds_time: Option<(DateTime<Utc>, Instant)>,
    clouds_file: PathBuf,
    clouds_asked: Option<Instant>,
    temp_grid: Option<Model>,
    temp_file: PathBuf,
    temps_busy: bool,
    temps_retry: Option<Instant>,
    /// The stretch of the map and the zoom of the heatmap on screen.
    heat_shown: Option<(Extent, f64)>,
    thumbs: Option<Thumbs>,
    moved_at: Option<Instant>,
    rv_due: Option<Instant>,
    idle_since: Option<Instant>,
    /// A notice about the map's key is up.
    key_banner: bool,
    away: bool,
    wind: Option<WindField>,
    flow: Option<Flow>,
    wind_at: Option<Instant>,
    wind_file: PathBuf,
    /// The DWD failed: not before this again.
    wind_dwd_retry: Option<Instant>,
    /// Open-Meteo's limit is reached: not before this again.
    meteo_pause: Option<Instant>,
    /// A banner about temperatures or wind is on screen.
    problem_shown: bool,
    wind_busy: bool,
    wind_due: Option<Instant>,
    wind_frame: Instant,
    slow_said: bool,
    rain_dirty: bool,
    base_dirty: bool,
    rv_frames: RvFrames,
    warning_map: Option<Arc<WarningMap>>,
    warning_file: PathBuf,
    warnings_busy: bool,
    warnings_due: Option<Instant>,
    mosaic_weights: RefCell<Option<Arc<Weights>>>,
    /// The pointer rests on the legend: the edge between the sources is drawn.
    seam: bool,
    /// The times of the timeline on screen, so a new one keeps the moment that is shown.
    timeline: Vec<DateTime<Utc>>,
    dwd_busy: bool,
    rv_busy: bool,
    /// The source named in the legend, to notice when panning changes it.
    shown_source: String,
}

/// True when the key gets another picture than no key at all.
pub async fn carto_key_works(http: &Http, key: &str) -> Result<bool> {
    let base = "https://basemaps.cartocdn.com/rastertiles/dark_all/0/0/0.png";
    let (with_key, without) = (http.bytes(&format!("{base}?key={key}")).await?, http.bytes(base).await?);
    Ok(with_key != without)
}

/// The time of the last satellite picture from the disk. It counts as old, so it gets checked as soon as the network answers.
fn remembered_clouds_time(file: &std::path::Path) -> Option<(DateTime<Utc>, Instant)> {
    let text = std::fs::read_to_string(file).ok()?;
    let time = DateTime::parse_from_rfc3339(text.trim()).ok()?.with_timezone(&Utc);
    Some((time, Instant::now().checked_sub(CLOUDS_MAX_AGE)?))
}

/// One picture layer on the map.
#[derive(Default)]
struct Slot {
    /// The data in use.
    layer: Option<Layer>,
    /// The stretch of the map and the zoom of the picture on screen.
    shown: Option<(Extent, f64)>,
    /// Tiles have arrived, or the view left the picture.
    dirty: bool,
    drawn: Option<Instant>,
    checked: Option<Instant>,
    /// A tile failed: the picture is put together again from this time on, which asks for it again.
    retry: Option<Instant>,
}

impl Worker {
    fn emit(&self, event: Event) {
        (self.events)(event);
    }

    fn kind(&self) -> Provider {
        match self.config.source {
            RadarSource::Dwd => Provider::Dwd,
            RadarSource::RainViewer => Provider::RainViewer,
            RadarSource::Auto => Provider::Mosaic,
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
                let started = Instant::now();
                self.render_base();
                tracing::trace!(ms = started.elapsed().as_millis(), "[Radar] Map drawn");
            }
            if self.rain_dirty {
                self.rain_dirty = false;
                let started = Instant::now();
                self.render_rain();
                tracing::trace!(
                    ms = started.elapsed().as_millis(),
                    position = self.position,
                    "[Radar] Rain drawn"
                );
            }
        }
    }

    fn next_timeout(&self) -> Duration {
        if !self.active && self.idle_since.is_some() {
            return Duration::from_secs(5);
        }
        if !self.active || self.away {
            return Duration::from_secs(3600);
        }
        if self.wind_on() {
            return WIND_FRAME;
        }
        if self.slots.iter().any(|s| s.dirty) || self.thumbs.as_ref().is_some_and(Thumbs::busy) {
            return Duration::from_millis(50);
        }
        Duration::from_secs(1)
    }

    fn apply(&mut self, command: Command) -> bool {
        match &command {
            Command::DwdLoaded(_) => self.dwd_busy = false,
            Command::RvLoaded(_) => self.rv_busy = false,
            _ => {}
        }
        match command {
            Command::Close => return false,
            Command::View(view) => {
                if view != self.view {
                    let before = self.kind();
                    self.view = view;
                    self.lut = None;
                    self.base_dirty = true;
                    self.rain_dirty = true;
                    self.flow = None;
                    self.wind_due = Some(Instant::now() + WIND_DELAY);
                    self.moved_at = Some(Instant::now());
                    self.rv_due = Some(Instant::now() + SETTLE);
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
                let before = KINDS.map(|kind| self.layer_on(kind));
                self.config = *config;
                self.layer_failed = false;
                self.base_dirty = true;
                self.rain_dirty = true;
                self.wind_due = Some(Instant::now());
                self.check_key();
                for kind in KINDS {
                    if before[kind.index()] != self.layer_on(kind) {
                        self.slots[kind.index()] = Slot::default();
                        if kind == Kind::Warnings && !self.layer_on(kind) {
                            self.emit(Event::Warnings(None));
                        }
                        if !self.layer_on(kind) {
                            self.emit(Event::Layer {
                                kind,
                                picture: None,
                                time: None,
                            });
                        }
                    }
                }
                if !self.config.temps_layer {
                    self.emit(Event::Temps(Vec::new()));
                }
                if !self.config.heat_layer {
                    self.heat_shown = None;
                    self.emit(Event::Heat(None));
                }
                if !self.config.wind_layer {
                    self.flow = None;
                    self.emit(Event::Wind(None));
                }
                if source_changed {
                    self.fetch();
                    self.publish_timeline();
                }
            }
            Command::Active(active) => {
                self.active = active;
                self.idle_since = (!active).then(Instant::now);
                if active {
                    self.load_cached();
                    self.publish_timeline();
                    self.base_dirty = true;
                    self.rain_dirty = true;
                    self.wind_due = Some(Instant::now());
                    self.check_key();
                    if self.last_fetch.is_none_or(|t| t.elapsed() >= REFRESH) {
                        self.fetch();
                    }
                }
            }
            Command::Away(away) => {
                if self.away && !away {
                    self.base_dirty = true;
                    self.rain_dirty = true;
                }
                self.away = away;
            }
            Command::Reload => {
                self.store.forget_failures();
                self.layer_failed = false;
                self.retry_at = None;
                self.temps_retry = None;
                self.wind_dwd_retry = None;
                self.wind_due = Some(Instant::now());
                self.fetch();
                self.base_dirty = true;
            }
            Command::Probe { lon, lat } => self.probe(lon, lat),
            Command::Seam(on) => {
                if on != self.seam {
                    self.seam = on;
                    self.rain_dirty = true;
                }
            }
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
                    self.rv_frames.clear();
                    self.loaded();
                    self.preload_rv();
                }
                Err(e) => self.failed(&e),
            },
            Command::LayerTile { kind } => self.slots[kind.index()].dirty = true,
            Command::LayerTileFailed { kind } => {
                let slot = &mut self.slots[kind.index()];
                slot.retry = slot.retry.or(Some(Instant::now() + TILE_RETRY));
            }
            Command::WarningsLoaded(result) => self.warnings_loaded(result),
            Command::TempsLoaded(result) => {
                self.temps_busy = false;
                match result {
                    Ok(grid) => {
                        self.temp_grid = Some(grid);
                        self.heat_shown = None;
                        self.show_temps();
                        self.problem_over();
                    }
                    Err(error) => {
                        tracing::warn!(%error, "[Radar] Temperatures failed");
                        self.temps_retry = Some(Instant::now() + LAYER_RETRY);
                        self.problem_shown = true;
                        self.emit(Event::Notice(Notice::Failed {
                            text: format!("Temperatures could not be loaded: {error}"),
                        }));
                    }
                }
            }
            Command::CloudsTime(result) => match result {
                Ok(time) => {
                    self.clouds_time = Some((time, Instant::now()));
                    // The next start knows at once which tiles on the disk count, also without a network.
                    let _ = crate::store::write_atomic(&self.clouds_file, time.to_rfc3339().as_bytes());
                }
                Err(e) => {
                    tracing::warn!(error = %e, "[Radar] No cloud picture");
                    self.emit(Event::Notice(Notice::Fallback {
                        text: "No cloud picture right now".into(),
                    }));
                }
            },
            Command::WindLoaded(loaded) => self.wind_loaded(loaded),
            Command::KeyChecked(valid) => {
                if !valid && !self.layer_failed {
                    self.layer_failed = true;
                    self.key_banner = true;
                    self.base_dirty = true;
                    self.emit(Event::Notice(Notice::BadKey {
                        service: "CARTO".into(),
                    }));
                }
            }
            Command::MapThumbs(place) => {
                self.thumbs = place.map(|(lon, lat)| Thumbs::new(lon, lat, self.config.dark));
            }
            Command::ThumbTile(result) => match result {
                Ok(()) => {
                    if let Some(thumbs) = &mut self.thumbs {
                        thumbs.tile_arrived();
                    }
                }
                Err(e) => tracing::debug!(error = %e, "[Radar] A tile of a map preview failed"),
            },
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
                        self.key_banner = true;
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
            Provider::Dwd => self.grid.iter().map(|f| f.time).filter(|t| *t <= Utc::now()).max(),
            Provider::RainViewer => self.rv.last().map(|f| f.time),
            Provider::Mosaic => {
                let dwd = self.grid.iter().map(|f| f.time).filter(|t| *t <= Utc::now()).max();
                dwd.into_iter().chain(self.rv.last().map(|f| f.time)).max()
            }
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
            Provider::Dwd => "DWD",
            Provider::RainViewer => "RainViewer",
            Provider::Mosaic => return self.mosaic_label(),
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
        let outside = self.kind() == Provider::Mosaic && mosaic::weight(lon, lat) < 0.5;
        let text = if self.kind() == Provider::RainViewer || outside {
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
                        self.fetch_tile(&layer, z, x, y, true);
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
        if !self.active || self.away || self.key_checked.as_deref() == Some(key.as_str()) {
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

    fn layer_on(&self, kind: Kind) -> bool {
        match kind {
            Kind::Clouds => self.config.clouds_layer,
            Kind::Warnings => self.config.alerts_layer,
            Kind::Lightning => self.config.lightning_layer,
        }
    }

    /// Finds the newest satellite picture now and then.
    fn check_clouds_time(&mut self) {
        let fresh = self.clouds_time.is_some_and(|(_, at)| at.elapsed() < CLOUDS_MAX_AGE);
        if fresh || self.clouds_asked.is_some_and(|t| t.elapsed() < LAYER_RETRY) {
            return;
        }
        self.clouds_asked = Some(Instant::now());
        let (http, tx) = (self.http.clone(), self.tx.clone());
        self.runtime.spawn(async move {
            let _ = tx.send(Command::CloudsTime(clouds::latest(&http).await));
        });
    }

    fn request_layer(&self, kind: Kind, layer: &Layer, z: i32, x: i64, y: i64) {
        let tx = self.tx.clone();
        self.store
            .request(&self.runtime, layer, z, x, y, move |result| match result {
                Ok(()) => {
                    let _ = tx.send(Command::LayerTile { kind });
                }
                Err(e) => {
                    tracing::debug!(error = %e, ?kind, "[Radar] A layer tile failed");
                    let _ = tx.send(Command::LayerTileFailed { kind });
                }
            });
    }

    /// New data takes over once all of the view is there: until then the old picture stays, so nothing flashes.
    fn follow_data(&mut self, kind: Kind) {
        let slot = &self.slots[kind.index()];
        if slot.checked.is_some_and(|t| t.elapsed() < LAYER_CHECK) {
            return;
        }
        let wanted = match kind {
            Kind::Clouds => self.clouds_time.map(|(time, _)| layers::layer(kind, time)),
            _ => Some(layers::layer(kind, layers::bucket(kind, Utc::now()))),
        };
        let Some(wanted) = wanted else {
            return;
        };
        if slot.layer.is_none() {
            let slot = &mut self.slots[kind.index()];
            slot.layer = Some(wanted);
            slot.dirty = true;
            return;
        }
        self.slots[kind.index()].checked = Some(Instant::now());
        if self.slots[kind.index()].layer.as_ref() == Some(&wanted) {
            return;
        }
        let big = layers::big_view(&self.view, kind);
        let z = wanted.level(big.zoom);
        let mut complete = true;
        for (x, y) in tiles::visible(&big, &wanted, z) {
            let x = x.rem_euclid(1 << z.clamp(0, 30));
            if self.store.cached(&wanted, z, x, y).is_none() {
                complete = false;
                self.request_layer(kind, &wanted, z, x, y);
            }
        }
        if complete {
            let slot = &mut self.slots[kind.index()];
            slot.layer = Some(wanted);
            slot.dirty = true;
        }
    }

    /// Puts a layer's picture together from its tiles. What is not there yet is fetched, a coarser tile stands in meanwhile.
    fn render_layer(&mut self, kind: Kind) {
        let started = Instant::now();
        let _done = Defer(move || tracing::trace!(ms = started.elapsed().as_millis(), ?kind, "[Radar] Layer drawn"));
        if kind == Kind::Warnings {
            self.render_warnings();
            return;
        }
        let Some(layer) = self.slots[kind.index()].layer.clone() else {
            return;
        };
        let view = self.view;
        let big = layers::big_view(&view, kind);
        let mut rgba = vec![0u8; (big.width * big.height * 4) as usize];
        for (z, x, y) in tiles::compose(&big, &layer, &self.store, &mut rgba, 1.0) {
            self.request_layer(kind, &layer, z, x, y);
        }
        if kind == Kind::Clouds {
            clouds::soften(&mut rgba, &big);
        }
        let extent = big.extent(0.0);
        let slot = &mut self.slots[kind.index()];
        slot.dirty = false;
        slot.drawn = Some(Instant::now());
        slot.shown = Some((extent, view.zoom));
        let picture = Picture {
            width: big.width,
            height: big.height,
            rgba: Arc::new(rgba),
        };
        let time = if kind == Kind::Clouds {
            self.clouds_time.map(|(t, _)| t)
        } else {
            None
        };
        self.emit(Event::Layer {
            kind,
            picture: Some((picture, extent)),
            time,
        });
    }

    fn update_layers(&mut self) {
        let view = self.view;
        if self.config.clouds_layer {
            self.check_clouds_time();
        }
        for kind in KINDS {
            if !self.layer_on(kind) {
                continue;
            }
            if kind == Kind::Warnings {
                self.fetch_warnings();
            } else {
                self.follow_data(kind);
            }
            let slot = &mut self.slots[kind.index()];
            if slot.retry.is_some_and(|t| t <= Instant::now()) {
                slot.retry = None;
                slot.dirty = true;
            }
            if !slot
                .shown
                .is_some_and(|(extent, zoom)| layers::covers(&extent, zoom, &view))
            {
                slot.dirty = true;
            }
            if slot.dirty && slot.drawn.is_none_or(|t| t.elapsed() >= LAYER_FRAME) {
                self.render_layer(kind);
            }
        }
    }

    fn wind_on(&self) -> bool {
        self.active && !self.away && self.config.wind_layer
    }

    /// Asks for a new grid when the view left the old one, or when it is too old.
    fn fetch_wind(&mut self) {
        if !self.wind_on() || self.wind_busy || self.wind_due.is_none_or(|t| t > Instant::now()) {
            return;
        }
        self.wind_due = None;
        let (now, hour) = (Instant::now(), model::hour(Utc::now()));
        let fresh = self.wind_at.is_some_and(|t| t.elapsed() < WIND_MAX_AGE);
        if self.wind.as_ref().is_some_and(|f| f.enough(&self.view, hour, fresh)) {
            self.wind_at = Some(now);
            return;
        }
        let dwd = self.wind_dwd_retry.is_none_or(|t| t <= now);
        let meteo = self.meteo_pause.is_none_or(|t| t <= now) && !self.wind.as_ref().is_some_and(|f| f.usable(hour));
        if !dwd && !meteo {
            self.wind_at = Some(now);
            return;
        }
        self.wind_busy = true;
        let (http, tx, view, file) = (self.http.clone(), self.tx.clone(), self.view, self.wind_file.clone());
        self.runtime.spawn(async move {
            let loaded = wind::load(&http, &file, &view, hour, dwd, meteo).await;
            let _ = tx.send(Command::WindLoaded(loaded));
        });
    }

    fn problem_over(&mut self) {
        if std::mem::take(&mut self.problem_shown) {
            self.emit(Event::Notice(Notice::Solved));
        }
    }

    fn wind_loaded(&mut self, loaded: WindLoaded) {
        self.wind_busy = false;
        self.wind_at = Some(Instant::now());
        if loaded.dwd_failed {
            self.wind_dwd_retry = Some(Instant::now() + WIND_DWD_RETRY);
        }
        if loaded.meteo_limited {
            self.meteo_pause = Some(Instant::now() + METEO_PAUSE);
        }
        if loaded.field.is_some() {
            self.wind = loaded.field;
            self.problem_over();
        } else if self.wind.is_none() {
            self.wind_due = Some(Instant::now() + WIND_RETRY);
            if let Some(error) = loaded.problem {
                self.problem_shown = true;
                self.emit(Event::Notice(Notice::Failed {
                    text: format!("Wind could not be loaded: {error}"),
                }));
            }
        }
    }

    fn animate_wind(&mut self) {
        if !self.wind_on() || self.wind_frame.elapsed() < WIND_FRAME {
            return;
        }
        self.wind_frame = Instant::now();
        let Some(field) = &self.wind else {
            return;
        };
        let view = self.view;
        let flow = self.flow.get_or_insert_with(|| Flow::new(&view));
        flow.step(&view, field, self.config.dark);
        let picture = Picture {
            width: flow.width,
            height: flow.height,
            rgba: Arc::new(flow.pixels().to_vec()),
        };
        self.emit(Event::Wind(Some(picture)));
    }

    /// In the tray the app stays small: the grids can be read back from the disk in a moment.
    fn drop_when_cold(&mut self) {
        if self.idle_since.is_some_and(|t| t.elapsed() >= KEEP_WARM) {
            self.idle_since = None;
            self.grid = Vec::new();
            self.motions = Vec::new();
            self.rv = Vec::new();
            self.lut = None;
            self.flow = None;
            self.wind = None;
            self.wind_at = None;
            self.temp_grid = None;
            self.rv_frames.clear();
            self.warning_map = None;
        }
    }

    /// After the map stopped: the radar frames of the new place, instead of those still queued for the old one.
    fn preload_after_move(&mut self) {
        if self.rv_due.is_none_or(|t| t > Instant::now()) {
            return;
        }
        self.rv_due = None;
        if self.kind() == Provider::Mosaic {
            self.fetch_missing();
            if self.source_name() != self.shown_source {
                self.publish_timeline();
            }
        }
        if self.kind() != Provider::Dwd && !self.rv.is_empty() {
            self.store.new_radar_round();
            self.preload_rv();
            self.report_rv_progress();
        }
    }

    fn show_temps(&self) {
        if self.config.temps_layer
            && let Some(grid) = &self.temp_grid
        {
            self.emit(Event::Temps(grid.labels(&self.view)));
        }
    }

    /// One grid an hour serves every city and the heatmap, wherever the map goes.
    fn fetch_temps(&mut self) {
        if !self.config.temps_layer && !self.config.heat_layer {
            return;
        }
        self.draw_heat();
        let hour = model::hour(Utc::now());
        let current = self.temp_grid.as_ref().is_some_and(|g| g.hour == hour);
        if current || self.temps_busy || self.temps_retry.is_some_and(|t| t > Instant::now()) {
            return;
        }
        self.temps_busy = true;
        let (http, tx, file) = (self.http.clone(), self.tx.clone(), self.temp_file.clone());
        self.runtime.spawn(async move {
            let _ = tx.send(Command::TempsLoaded(temps::load(&http, &file, hour).await));
        });
    }

    fn draw_thumbs(&mut self) {
        let Some(thumbs) = self.thumbs.as_mut().filter(|t| t.due()) else {
            return;
        };
        let (pictures, wanted) = thumbs.render(&self.store, self.config.carto_key.as_deref());
        for (index, picture) in pictures {
            self.emit(Event::MapThumb { index, picture });
        }
        for (layer, z, x, y) in wanted {
            let tx = self.tx.clone();
            self.store.request(&self.runtime, &layer, z, x, y, move |result| {
                let _ = tx.send(Command::ThumbTile(result));
            });
        }
    }

    fn draw_heat(&mut self) {
        if !self.config.heat_layer
            || self
                .heat_shown
                .is_some_and(|(extent, zoom)| layers::covers(&extent, zoom, &self.view))
        {
            return;
        }
        let Some(grid) = &self.temp_grid else {
            return;
        };
        let heat = grid.heat(&self.view);
        self.heat_shown = Some((heat.extent, self.view.zoom));
        self.emit(Event::Heat(Some((
            Picture {
                width: heat.width,
                height: heat.height,
                rgba: Arc::new(heat.rgba),
            },
            heat.extent,
        ))));
    }

    fn tick(&mut self) {
        self.drop_when_cold();
        if !self.active || self.away {
            return;
        }
        self.preload_after_move();
        self.fetch_temps();
        self.draw_thumbs();
        if self.wind_at.is_some_and(|t| t.elapsed() >= WIND_MAX_AGE) && self.wind_due.is_none() {
            self.wind_due = Some(Instant::now());
        }
        self.update_layers();
        self.fetch_wind();
        self.animate_wind();
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
        if self.kind() != Provider::RainViewer
            && self.grid.is_empty()
            && let Some(frames) = dwd::load_cached(&self.grid_cache)
        {
            self.motions = vec![None; frames.len().saturating_sub(1)];
            self.grid = frames;
        }
    }

    fn fetch(&mut self) {
        if !self.active || self.away || self.fetching.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
            return;
        }
        self.fetching = Some(Instant::now());
        self.emit(Event::Notice(Notice::Loading { done: 0, total: 1 }));
        // Only what the view shows: over Spain the DWD is not asked, deep in Germany RainViewer is not.
        if self.needs_dwd() {
            self.spawn_dwd();
        }
        if self.needs_rv() {
            self.spawn_rv();
        }
    }

    fn publish_timeline(&mut self) {
        let now = Utc::now();
        let (times, states) = self.timeline_now();
        let live = times.iter().rposition(|t| *t <= now).unwrap_or(0);
        // A new timeline keeps the moment on screen, also when it starts earlier or ends later than the old one.
        let moment = (!self.position.is_nan())
            .then(|| mosaic::time_at(&self.timeline, self.position))
            .flatten();
        let moved = moment
            .and_then(|t| mosaic::position_of(&times, t))
            .filter(|p| (*p - self.position).abs() > 0.001 && !self.timeline.is_empty());
        if let Some(p) = moved {
            self.position = p;
        }
        if self.position.is_nan() || self.position as usize >= times.len() {
            self.position = live as f32;
        }
        self.timeline = times.clone();
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
        self.shown_source = source.clone();
        self.emit(Event::Timeline {
            frames,
            live,
            source,
            position: moved,
        });
    }

    /// The times of the timeline for the chosen source, and whether each can be drawn.
    fn timeline_now(&self) -> (Vec<DateTime<Utc>>, Vec<FrameState>) {
        match self.kind() {
            Provider::Dwd => self.grid.iter().map(|f| (f.time, FrameState::Ready)).unzip(),
            Provider::RainViewer => self.rv.iter().map(|f| (f.time, self.rv_state(f))).unzip(),
            Provider::Mosaic => {
                let times = self.mosaic_times();
                let states = self.mosaic_states(&times);
                (times, states)
            }
        }
    }

    fn rv_state(&self, frame: &rainviewer::Frame) -> FrameState {
        let z = frame.layer.level(self.view.zoom);
        if self.rv_missing(&self.view, &frame.layer, z).is_empty() {
            FrameState::Ready
        } else {
            FrameState::Missing
        }
    }

    /// Newest first: the live picture comes before the history.
    /// With half a screen around the view, so a pan shows sharp rain at once.
    fn preload_rv(&self) {
        let grow = |pixels: u32| (f64::from(pixels) * (1.0 + 2.0 * RV_MARGIN)).round() as u32;
        let around = View {
            width: grow(self.view.width),
            height: grow(self.view.height),
            ..self.view
        };
        for frame in self.rv.iter().rev() {
            let z = frame.layer.level(self.view.zoom);
            for (x, y) in self.rv_missing(&around, &frame.layer, z) {
                self.request(&frame.layer, z, x, y, false);
            }
        }
    }

    fn report_rv_progress(&self) {
        let (_, states) = self.timeline_now();
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

    /// Only for what is on screen. Pictures still get drawn from the cache in the background, they just fetch nothing.
    fn request(&self, layer: &Layer, z: i32, x: i64, y: i64, base: bool) {
        if self.active && !self.away && (base || self.rv_tile_wanted(z, x, y)) {
            self.fetch_tile(layer, z, x, y, base);
        }
    }

    fn fetch_tile(&self, layer: &Layer, z: i32, x: i64, y: i64, base: bool) {
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
        self.config
            .basemap
            .layers(self.config.carto_key.as_deref(), self.config.labels_above_rain)
    }

    fn render_base(&mut self) {
        let view = self.view;
        let colors = if self.config.dark { vector::DARK } else { vector::LIGHT };
        let mut rgba = vector::draw(&view, colors);
        let (base, labels_layer) = self.map_layers();
        let no_key = self.config.carto_key.is_none() && self.config.basemap.needs_key();
        if no_key && !self.key_banner {
            self.key_banner = true;
            self.emit(Event::Notice(Notice::MissingKey {
                service: "CARTO".into(),
            }));
        } else if !no_key && !self.layer_failed && self.key_banner {
            self.key_banner = false;
            self.emit(Event::Notice(Notice::MapReady));
        }
        let own = base.is_none();
        if let Some(layer) = &base {
            for (z, x, y) in tiles::compose(&view, layer, &self.store, &mut rgba, 1.0) {
                self.request(layer, z, x, y, true);
            }
        }
        self.emit(Event::Base(
            Picture {
                width: view.width,
                height: view.height,
                rgba: Arc::new(rgba),
            },
            view,
        ));
        self.show_temps();
        // The temperature labels carry the names of the cities themselves.
        let names = own && !self.config.temps_layer;
        self.emit(Event::Cities(if names { labels::visible(&view) } else { Vec::new() }));
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
            Provider::Dwd => self.rain_dwd_at(if self.position.is_nan() { 0.0 } else { self.position }),
            Provider::RainViewer => self.rain_rv_at(if self.position.is_nan() {
                f32::MAX
            } else {
                self.position
            }),
            Provider::Mosaic => self.rain_mosaic(),
        };
        self.emit(Event::Rain(Picture {
            width: view.width,
            height: view.height,
            rgba: Arc::new(rgba),
        }));
    }

    fn rain_dwd_at(&mut self, position: f32) -> Vec<u8> {
        let view = self.view;
        let size = (view.width * view.height * 4) as usize;
        let Some(max) = self.grid.len().checked_sub(1) else {
            return vec![0; size];
        };
        let position = position.clamp(0.0, max as f32);
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
        let pick: fn(&[u16], f32, f32) -> f32 = if view.zoom >= SMOOTH_FROM_ZOOM {
            dwd::sample_smooth
        } else {
            dwd::sample
        };
        let mut out = vec![0u8; size];
        out.par_chunks_mut(lut.width.max(1) * 4)
            .zip(lut.cells.par_chunks(lut.width.max(1)))
            .for_each(|(row, cells)| {
                for (px, cell) in row.as_chunks_mut::<4>().0.iter_mut().zip(cells) {
                    let [x, y] = *cell;
                    let value = match (&warp, &b) {
                        (Some(field), Some(b)) => {
                            let (vx, vy) = field.at(x, y);
                            let from = pick(&a, x - vx * k, y - vy * k);
                            let to = pick(b, x + vx * (1.0 - k), y + vy * (1.0 - k));
                            from + (to - from) * k
                        }
                        _ => pick(&a, x, y),
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

    /// RainViewer at a fractional frame; past the last frame its rain is moved on along its motion for up to an hour.
    fn rain_rv_at(&mut self, position: f32) -> Vec<u8> {
        let view = self.view;
        let size = (view.width * view.height * 4) as usize;
        let Some(max) = self.rv.len().checked_sub(1) else {
            return vec![0; size];
        };
        if self.kind() == Provider::Mosaic && position > max as f32 + 0.01 && max > 0 {
            return self.rv_ahead(max, position - max as f32);
        }
        let position = position.clamp(0.0, max as f32);
        let index = position as usize;
        let k = position - index as f32;
        // A frame that is not complete yet must not show an empty map: the nearest complete one stands in.
        let ready = |i: usize| self.rv.get(i).is_some_and(|f| self.rv_state(f) == FrameState::Ready);
        let shown = (0..=max)
            .flat_map(|d| [index.checked_sub(d), Some(index + d)])
            .flatten()
            .find(|i| ready(*i))
            .unwrap_or(index);
        let next = (shown == index && ready(index + 1)).then_some(index + 1);
        if shown != index
            && let Some(frame) = self.rv.get(index).cloned()
        {
            let z = frame.layer.level(view.zoom);
            for (x, y) in tiles::visible(&view, &frame.layer, z) {
                self.request(&frame.layer, z, x.rem_euclid(1 << z), y, false);
            }
        }
        self.rv_frames.keep(&view, shown);
        let opacity = self.config.opacity;
        let Some(a) = self.rv_picture(shown) else {
            return vec![0; size];
        };
        // Moving both frames toward each other lets a shower travel; fading them only made it stand in two places.
        if self.config.smooth
            && k > 0.01
            && let Some(b) = next.and_then(|n| self.rv_picture(n))
        {
            let (width, height) = (view.width as usize, view.height as usize);
            let motion = self.rv_frames.flow(shown, &a, &b, width, height);
            return flow::blend(&a, &b, width, height, &motion, k, opacity);
        }
        rv_frames::faded(&a, opacity)
    }

    /// A whole frame of the view; complete ones are kept until the view changes.
    fn rv_picture(&mut self, index: usize) -> Option<Arc<Vec<u8>>> {
        if let Some(picture) = self.rv_frames.picture(index) {
            return Some(picture);
        }
        let frame = self.rv.get(index)?.clone();
        let view = self.view;
        let mut rgba = vec![0u8; (view.width * view.height * 4) as usize];
        let missing: Vec<_> = tiles::compose(&view, &frame.layer, &self.store, &mut rgba, 1.0)
            .into_iter()
            .filter(|(z, x, y)| self.rv_tile_wanted(*z, *x, *y))
            .collect();
        let complete = missing.is_empty();
        for (z, x, y) in missing {
            self.request(&frame.layer, z, x, y, false);
        }
        let rgba = Arc::new(rgba);
        if complete {
            self.rv_frames.put(index, Arc::clone(&rgba));
        }
        Some(rgba)
    }

    /// The last frame moved on by `steps` frame steps, fading after half an hour.
    fn rv_ahead(&mut self, max: usize, steps: f32) -> Vec<u8> {
        let view = self.view;
        let (width, height) = (view.width as usize, view.height as usize);
        let minutes = f64::from(steps) * self.rv_step_minutes();
        let fade = mosaic::ahead_fade(minutes) * self.config.opacity;
        self.rv_frames.keep(&view, max);
        let (Some(a), Some(b)) = (self.rv_picture(max - 1), self.rv_picture(max)) else {
            return vec![0; width * height * 4];
        };
        if fade <= 0.0 {
            return vec![0; width * height * 4];
        }
        let motion = self.rv_frames.flow(max - 1, &a, &b, width, height);
        flow::advect(&b, width, height, &motion, steps, fade)
    }

    fn rv_step_minutes(&self) -> f64 {
        match self.rv.as_slice() {
            [a, b, ..] => (b.time - a.time).num_seconds() as f64 / 60.0,
            _ => 10.0,
        }
    }

    /// The shapes from the disk first, then from the DWD every few minutes while the layer is on screen.
    fn fetch_warnings(&mut self) {
        let now = Instant::now();
        if self.warning_map.is_none()
            && let Some((map, at)) = warning_areas::load(&self.warning_file, WARNINGS_MAX_AGE)
        {
            let age = (Utc::now() - at).to_std().unwrap_or_default();
            self.warnings_due = Some(now + WARNINGS_REFRESH.saturating_sub(age));
            self.set_warnings(map);
        }
        if self.warnings_busy || self.warnings_due.is_some_and(|t| t > now) {
            return;
        }
        self.warnings_busy = true;
        let (http, tx, file) = (self.http.clone(), self.tx.clone(), self.warning_file.clone());
        self.runtime.spawn(async move {
            let _ = tx.send(Command::WarningsLoaded(warning_areas::fetch(&http, &file).await));
        });
    }

    fn warnings_loaded(&mut self, result: Result<WarningMap>) {
        self.warnings_busy = false;
        match result {
            Ok(map) => {
                self.warnings_due = Some(Instant::now() + WARNINGS_REFRESH);
                self.set_warnings(map);
                self.problem_over();
            }
            Err(error) => {
                tracing::warn!(%error, "[Radar] Warnings failed");
                self.warnings_due = Some(Instant::now() + WARNINGS_RETRY);
                if self.warning_map.is_none() {
                    self.problem_shown = true;
                    self.emit(Event::Notice(Notice::Failed {
                        text: format!("Warnings could not be loaded: {error}"),
                    }));
                }
            }
        }
    }

    fn set_warnings(&mut self, map: WarningMap) {
        let map = Arc::new(map);
        self.warning_map = Some(Arc::clone(&map));
        self.slots[Kind::Warnings.index()].dirty = true;
        self.emit(Event::Warnings(Some(map)));
    }

    /// The shapes drawn for the view, sharp at every zoom.
    fn render_warnings(&mut self) {
        let Some(map) = self.warning_map.clone() else {
            // Drawn when the shapes come; until then nothing is to do.
            self.slots[Kind::Warnings.index()].dirty = false;
            return;
        };
        let view = self.view;
        let big = layers::big_view(&view, Kind::Warnings);
        let extent = big.extent(0.0);
        let rgba = warning_layer::draw(&map, &extent, big.width, big.height);
        let slot = &mut self.slots[Kind::Warnings.index()];
        slot.dirty = false;
        slot.drawn = Some(Instant::now());
        slot.shown = Some((extent, view.zoom));
        self.emit(Event::Layer {
            kind: Kind::Warnings,
            picture: Some((
                Picture {
                    width: big.width,
                    height: big.height,
                    rgba: Arc::new(rgba),
                },
                extent,
            )),
            time: None,
        });
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

/// Runs a closure when dropped, for timing a function with several ways out.
struct Defer<F: FnMut()>(F);

impl<F: FnMut()> Drop for Defer<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}
