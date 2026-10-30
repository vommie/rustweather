//! The radar's worker thread. The UI sends commands, the worker keeps the frames, renders pictures and reports events; the
//! newest command always wins, so dragging the map never queues work. It fetches only while the radar is on screen.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rayon::prelude::*;

use super::clouds;
use super::dwd::{self, Lut, Projector};
use super::layers::{self, KINDS, Kind};
use super::motion::{self, Field};
use super::palette::{self, Ramp};
use super::rainviewer;
use super::temps;
use super::thumbs::Thumbs;
use super::wind::{self, Field as WindField, Flow};
use super::{FrameInfo, FrameState};
use crate::error::{Error, Result};
use crate::http::Http;
use crate::map::tiles::{self, Layer, Store};
use crate::map::{Extent, View, in_germany, labels, vector};
use crate::settings::{Basemap, RadarSource, RainPalette};

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
/// The share of a new heatmap that must be known before it replaces the one on screen.
const HEAT_COVERAGE: f32 = 0.7;
const WIND_FRAME: Duration = Duration::from_millis(33);
const WIND_DELAY: Duration = Duration::from_millis(600);
const WIND_MAX_AGE: Duration = Duration::from_secs(15 * 60);
const WIND_RETRY: Duration = Duration::from_secs(60);

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
    /// Pixel position and temperature of each label.
    Temps(Vec<(f32, f32, f32)>),
    /// The temperature colors and the stretch of the map they show; `None` takes them away.
    Heat(Option<(Picture, Extent)>),
    /// A picture for the map picker; `index` is the place in `Basemap::ALL`.
    MapThumb {
        index: usize,
        picture: Picture,
    },
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
    /// The screen is locked: nothing is fetched or animated until it is unlocked.
    Away(bool),
    Reload,
    Probe {
        lon: f64,
        lat: f64,
    },
    PreloadGermany,
    /// The map picker opens around a place, or closes.
    MapThumbs(Option<(f64, f64)>),
    Close,
    DwdLoaded(Result<Vec<dwd::Frame>>),
    RvLoaded(Result<Vec<rainviewer::Frame>>),
    LayerTile {
        kind: Kind,
    },
    CloudsTime(Result<DateTime<Utc>>),
    TempsLoaded(Result<Vec<(temps::Cell, f32)>>),
    KeyChecked(bool),
    WindLoaded {
        result: Result<WindField>,
    },
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
        temps: temps::Store::default(),
        temps_busy: false,
        temps_retry: None,
        heat_drawn: None,
        heat_shown: false,
        thumbs: None,
        moved_at: None,
        rv_due: None,
        idle_since: None,
        key_banner: false,
        away: false,
        wind: None,
        flow: None,
        wind_at: None,
        wind_busy: false,
        wind_due: None,
        wind_frame: Instant::now(),
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
enum Provider {
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
    slots: [Slot; 3],
    clouds_time: Option<(DateTime<Utc>, Instant)>,
    clouds_file: PathBuf,
    clouds_asked: Option<Instant>,
    temps: temps::Store,
    temps_busy: bool,
    temps_retry: Option<Instant>,
    /// The view the heatmap was last drawn for.
    heat_drawn: Option<View>,
    heat_shown: bool,
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
    wind_busy: bool,
    wind_due: Option<Instant>,
    wind_frame: Instant,
    slow_said: bool,
    rain_dirty: bool,
    base_dirty: bool,
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
}

impl Worker {
    fn emit(&self, event: Event) {
        (self.events)(event);
    }

    fn kind(&self) -> Provider {
        match self.config.source {
            RadarSource::Dwd => Provider::Dwd,
            RadarSource::RainViewer => Provider::RainViewer,
            RadarSource::Auto if in_germany(self.view.lon, self.view.lat) => Provider::Dwd,
            RadarSource::Auto => Provider::RainViewer,
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
        if !self.active && self.idle_since.is_some() {
            return Duration::from_secs(5);
        }
        if !self.active || self.away {
            return Duration::from_secs(3600);
        }
        if self.wind_on() {
            return WIND_FRAME;
        }
        if self.slots.iter().any(|s| s.dirty) {
            return Duration::from_millis(50);
        }
        Duration::from_secs(1)
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
                self.heat_drawn = None;
                if !self.config.heat_layer {
                    self.heat_shown = false;
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
            Command::Away(away) => self.away = away,
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
            Command::LayerTile { kind } => self.slots[kind.index()].dirty = true,
            Command::TempsLoaded(result) => {
                self.temps_busy = false;
                match result {
                    Ok(found) => {
                        self.temps.insert(found);
                        if self.config.temps_layer {
                            self.emit(Event::Temps(self.temps.labels(&self.view)));
                        }
                        self.heat_drawn = None;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "[Radar] No temperatures");
                        self.temps_retry = Some(Instant::now() + LAYER_RETRY);
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
            Command::WindLoaded { result } => {
                self.wind_busy = false;
                self.wind_at = Some(Instant::now());
                match result {
                    Ok(field) => self.wind = Some(field),
                    Err(e) => {
                        tracing::warn!(error = %e, "[Radar] Wind failed");
                        self.emit(Event::Notice(Notice::Fallback {
                            text: "No wind data right now".into(),
                        }));
                        self.wind_due = Some(Instant::now() + WIND_RETRY);
                    }
                }
            }
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
            Command::MapThumbs(place) => self.thumbs = place.map(|(lon, lat)| Thumbs::new(lon, lat)),
            Command::TileDone { base, result } => match result {
                Ok(()) => {
                    if base {
                        self.base_dirty = true;
                        if let Some(thumbs) = &mut self.thumbs {
                            thumbs.tile_arrived();
                        }
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
        let text = if self.kind() == Provider::RainViewer {
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
                Err(e) => tracing::debug!(error = %e, ?kind, "[Radar] A layer tile failed"),
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
        let Some(layer) = self.slots[kind.index()].layer.clone() else {
            return;
        };
        let view = self.view;
        let big = layers::big_view(&view, kind);
        let mut rgba = vec![0u8; (big.width * big.height * 4) as usize];
        for (z, x, y) in tiles::compose(&big, &layer, &self.store, &mut rgba, 1.0) {
            self.request_layer(kind, &layer, z, x, y);
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
            self.follow_data(kind);
            let slot = &mut self.slots[kind.index()];
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
        let fresh = self.wind_at.is_some_and(|t| t.elapsed() < WIND_MAX_AGE);
        self.wind_due = None;
        if fresh && self.wind.as_ref().is_some_and(|f| f.covers(&self.view)) {
            return;
        }
        self.wind_busy = true;
        let (http, tx, view) = (self.http.clone(), self.tx.clone(), self.view);
        self.runtime.spawn(async move {
            let _ = tx.send(Command::WindLoaded {
                result: wind::fetch(&http, &view).await,
            });
        });
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
        }
    }

    /// After the map stopped: the radar frames of the new place, instead of those still queued for the old one.
    fn preload_after_move(&mut self) {
        if self.rv_due.is_none_or(|t| t > Instant::now()) {
            return;
        }
        self.rv_due = None;
        if self.kind() == Provider::RainViewer && !self.rv.is_empty() {
            self.store.new_radar_round();
            self.preload_rv();
            self.report_rv_progress();
        }
    }

    fn fetch_temps(&mut self) {
        let wanted = self.config.temps_layer || self.config.heat_layer;
        if !wanted || self.temps_busy || self.moved_at.is_some_and(|t| t.elapsed() < SETTLE) {
            return;
        }
        self.draw_heat();
        if self.temps_retry.is_some_and(|t| t > Instant::now()) {
            return;
        }
        let cells = if self.config.heat_layer {
            temps::around(&self.view)
        } else {
            temps::visible(&self.view)
        };
        let missing = self.temps.missing(&cells);
        if missing.is_empty() {
            return;
        }
        self.temps_busy = true;
        let (http, tx) = (self.http.clone(), self.tx.clone());
        self.runtime.spawn(async move {
            let _ = tx.send(Command::TempsLoaded(temps::fetch(&http, &missing).await));
        });
    }

    fn draw_thumbs(&mut self) {
        let Some(thumbs) = self.thumbs.as_mut().filter(|t| t.due()) else {
            return;
        };
        let (pictures, wanted) = thumbs.render(&self.store, self.config.carto_key.as_deref(), self.config.dark);
        for (index, picture) in pictures {
            self.emit(Event::MapThumb { index, picture });
        }
        for (layer, z, x, y) in wanted {
            self.request(&layer, z, x, y, true);
        }
    }

    /// After a zoom the cells are new: the old picture stays on its place on the map until most of the new one is known.
    fn draw_heat(&mut self) {
        if !self.config.heat_layer || self.heat_drawn == Some(self.view) {
            return;
        }
        self.heat_drawn = Some(self.view);
        let Some(heat) = self.temps.heat(&self.view) else {
            return;
        };
        if self.heat_shown && heat.coverage < HEAT_COVERAGE {
            return;
        }
        self.heat_shown = true;
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
        if self.kind() == Provider::Dwd
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
            Provider::Dwd => {
                let cache = self.grid_cache.clone();
                self.runtime.spawn(async move {
                    let _ = tx.send(Command::DwdLoaded(dwd::fetch(&http, &cache).await));
                });
            }
            Provider::RainViewer => {
                self.runtime.spawn(async move {
                    let _ = tx.send(Command::RvLoaded(rainviewer::frames(&http).await));
                });
            }
        }
    }

    fn publish_timeline(&mut self) {
        let now = Utc::now();
        let (times, states): (Vec<_>, Vec<_>) = match self.kind() {
            Provider::Dwd => self.grid.iter().map(|f| (f.time, FrameState::Ready)).unzip(),
            Provider::RainViewer => self.rv.iter().map(|f| (f.time, self.rv_state(f))).unzip(),
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
        if self.config.temps_layer {
            self.emit(Event::Temps(self.temps.labels(&view)));
        }
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
            Provider::Dwd => self.rain_dwd(),
            Provider::RainViewer => self.rain_rv(),
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
        // A frame that is not complete yet must not show an empty map: the nearest complete one stands in.
        let ready = |i: usize| self.rv.get(i).is_some_and(|f| self.rv_state(f) == FrameState::Ready);
        let shown = (0..=max)
            .flat_map(|d| [index.checked_sub(d), Some(index + d)])
            .flatten()
            .find(|i| ready(*i))
            .unwrap_or(index);
        let a = self.rv.get(shown).cloned();
        let b = self
            .rv
            .get(shown + 1)
            .filter(|_| shown == index && ready(index + 1))
            .cloned();
        if shown != index
            && let Some(frame) = self.rv.get(index).cloned()
        {
            let z = frame.layer.level(view.zoom);
            for (x, y) in tiles::visible(&view, &frame.layer, z) {
                self.request(&frame.layer, z, x.rem_euclid(1 << z), y, false);
            }
        }
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
