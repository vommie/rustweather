//! The running app: owns the windows and the state, and connects tray, weather loop and radar. Everything here lives on the
//! UI thread; other threads hand results over with `invoke_from_event_loop`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rustweather_core::color::Rgb;
use rustweather_core::http::Http;
use rustweather_core::map::View;
use rustweather_core::paths::Paths;
use rustweather_core::radar::engine::{self, Command, Config, Handle, Setup};
use rustweather_core::secrets::{self, Secrets};
use rustweather_core::settings::{Basemap, Settings};
use rustweather_core::ui_state::UiState;
use rustweather_core::{APP_ID, legacy, store};
use slint::winit_030::winit::window::WindowAttributes;
use slint::{ComponentHandle as _, ModelRc, VecModel};
use tokio::sync::watch;

use crate::alerts::Notifier;
use crate::tray::{self, TrayData, TrayEvent};
use crate::weather::{self, Control, Snapshot};
use crate::{
    Cli, FlyoutWindow, MainWindow, MiniWindow, SettingsWindow, Theme, instance, logging, models, presence, radar_ui,
    settings_ui,
};

pub struct App {
    pub main: MainWindow,
    pub settings_window: SettingsWindow,
    pub flyout: FlyoutWindow,
    pub mini: MiniWindow,
    pub paths: Paths,
    pub settings: RefCell<Settings>,
    pub secrets: RefCell<Secrets>,
    pub ui: RefCell<UiState>,
    pub runtime: tokio::runtime::Runtime,
    pub http: Http,
    pub radar: Handle,
    pub view: Cell<View>,
    pub snapshot: RefCell<Snapshot>,
    pub notifier: RefCell<Notifier>,
    pub settings_tx: watch::Sender<Settings>,
    pub control: RefCell<Option<Control>>,
    pub tray_tx: mpsc::Sender<TrayData>,
    pub cli_bg: Option<Rgb>,
    pub cli_fg: Option<Rgb>,
    pub position: Cell<f32>,
    pub frame_count: Cell<usize>,
    pub live: Cell<usize>,
    pub times: RefCell<Vec<chrono::DateTime<chrono::Utc>>>,
    pub playing: Cell<bool>,
    pub play_timer: slint::Timer,
    pub play_last: Cell<Instant>,
    pub toast_timer: slint::Timer,
    pub search_results: RefCell<Vec<rustweather_core::settings::Place>>,
    pub flyout_closed_at: Cell<Instant>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// Runs `f` with the app, on the UI thread.
pub fn with_app(f: impl FnOnce(&Rc<App>) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        let app = APP.with(|a| a.borrow().clone());
        if let Some(app) = app {
            f(&app);
        }
    });
}

impl App {
    pub fn save_settings(&self) {
        let settings = self.settings.borrow().clone();
        if let Err(e) = settings.save(&self.paths.settings_file()) {
            tracing::error!(error = %e, "[App] Could not save the settings");
            self.toast(&format!("Could not save the settings: {e}"), 3);
        }
        let _ = self.settings_tx.send(settings);
    }

    pub fn save_ui(&self) {
        let mut ui = self.ui.borrow_mut();
        let view = self.view.get();
        ui.view_lon = view.lon;
        ui.view_lat = view.lat;
        ui.view_zoom = view.zoom;
        ui.page = self.main.get_page();
        if let Err(e) = store::save(&self.paths.ui_state_file(), &*ui) {
            tracing::warn!(error = %e, "[App] Could not save the window state");
        }
    }

    pub fn toast(&self, text: &str, level: i32) {
        self.main.set_toast(text.into());
        self.main.set_toast_level(level);
        self.settings_window.set_toast(text.into());
        self.settings_window.set_toast_level(level);
        let (main, settings) = (self.main.as_weak(), self.settings_window.as_weak());
        self.toast_timer
            .start(slint::TimerMode::SingleShot, Duration::from_secs(5), move || {
                if let Some(w) = main.upgrade() {
                    w.set_toast("".into());
                }
                if let Some(w) = settings.upgrade() {
                    w.set_toast("".into());
                }
            });
    }

    pub fn radar_config(&self) -> Config {
        let settings = self.settings.borrow();
        let ui = self.ui.borrow();
        Config {
            source: settings.radar.source,
            basemap: settings.radar.basemap,
            palette: settings.radar.palette,
            opacity: settings.radar.opacity as f32 / 100.0,
            smooth: settings.radar.smooth,
            labels_above_rain: settings.radar.labels_above_rain,
            dark: self.main.global::<Theme>().get_dark(),
            carto_key: self.secrets.borrow().get(secrets::CARTO).map(str::to_owned),
            alerts_layer: ui.layer_alerts,
            clouds_layer: ui.layer_clouds,
            lightning_layer: ui.layer_lightning,
            wind_layer: ui.layer_wind,
        }
    }

    pub fn push_radar_config(&self) {
        self.radar.send(Command::Config(Box::new(self.radar_config())));
    }

    /// The radar loads only while it is on screen.
    pub fn update_radar_activity(&self) {
        let main_shows = self.main.window().is_visible() && self.main.get_page() == 2;
        let active = main_shows || self.mini.window().is_visible() || self.flyout.window().is_visible();
        self.radar.send(Command::Active(active));
        if !active {
            radar_ui::stop(self);
        }
    }

    /// Shows the weather everywhere.
    pub fn show_weather(&self) {
        let snapshot = self.snapshot.borrow();
        let settings = self.settings.borrow();
        let now = models::now(&snapshot, &settings);
        self.main.set_now(now.clone());
        self.flyout.set_now(now);
        self.main
            .set_hours(ModelRc::new(VecModel::from(models::hours(&snapshot, 24, 1))));
        self.flyout
            .set_hours(ModelRc::new(VecModel::from(models::hours(&snapshot, 6, 1))));
        self.main
            .set_days(ModelRc::new(VecModel::from(models::days(&snapshot, &settings))));
        self.main
            .set_alerts(ModelRc::new(VecModel::from(models::alerts(&snapshot))));
        let bars = models::bars(&snapshot);
        self.main.set_bars(ModelRc::new(VecModel::from(bars.clone())));
        self.flyout.set_bars(ModelRc::new(VecModel::from(bars)));
        let line = rustweather_core::tooltip::rain_line(snapshot.outlook).unwrap_or_default();
        self.main
            .set_eta_title(line.split(',').next().unwrap_or_default().into());
        self.main.set_eta_text(line.split_once(", ").map_or("", |p| p.1).into());
        let cfg_bg = Rgb::parse(&settings.tray.background).or(self.cli_bg);
        let cfg_fg = Rgb::parse(&settings.tray.foreground).or(self.cli_fg);
        let _ = self.tray_tx.send(models::tray(&snapshot, &settings, cfg_bg, cfg_fg));
        if let Some(error) = &snapshot.error {
            self.main.set_status(error.as_str().into());
            self.main.set_status_level(2);
        } else if self.main.get_page() != 2 {
            let at = snapshot
                .fetched_at
                .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string());
            self.main
                .set_status(at.map_or(String::new(), |t| format!("Updated {t}")).into());
            self.main.set_status_level(1);
        }
        let mut notifier = self.notifier.borrow_mut();
        notifier.update(
            &settings.alerts,
            &snapshot.alerts,
            snapshot.forecast.as_ref(),
            snapshot.outlook,
        );
    }

    pub fn open_main(&self, page: Option<i32>) {
        if let Some(page) = page {
            self.main.set_page(page);
        }
        if let Err(e) = self.main.show() {
            tracing::error!(error = %e, "[App] Could not show the window");
        }
        self.update_radar_activity();
    }

    pub fn open_settings(&self, page: Option<i32>) {
        if let Some(page) = page {
            self.settings_window.set_page(page);
        }
        settings_ui::refresh(self);
        if let Err(e) = self.settings_window.show() {
            tracing::error!(error = %e, "[App] Could not show the settings");
        }
    }

    pub fn toggle_flyout(&self, x: i32, y: i32) {
        if self.flyout.window().is_visible() {
            let _ = self.flyout.hide();
        } else if self.flyout_closed_at.get().elapsed() > Duration::from_millis(300) {
            // A click on the icon first takes the focus away, which already closed it.
            radar_ui::place_flyout(self, x, y);
            if let Err(e) = self.flyout.show() {
                tracing::error!(error = %e, "[Flyout] Could not show");
            }
            // The window manager places a new window itself, so the position is set once more when it is mapped.
            let weak = self.flyout.as_weak();
            let position = self.flyout.window().position();
            slint::Timer::single_shot(Duration::from_millis(60), move || {
                if let Some(flyout) = weak.upgrade() {
                    flyout.window().set_position(position);
                }
            });
        }
        self.update_radar_activity();
    }

    pub fn select_place(&self, index: usize) {
        {
            let mut settings = self.settings.borrow_mut();
            if index >= settings.places.len() {
                return;
            }
            settings.active_place = index;
        }
        self.save_settings();
        self.show_weather();
        if let Some(control) = self.control.borrow().as_ref() {
            control.now.notify_one();
        }
    }

    fn on_tray(&self, event: TrayEvent) {
        tracing::debug!(?event, "[Tray] Event");
        match event {
            TrayEvent::Clicked(x, y) => self.toggle_flyout(x, y),
            TrayEvent::MiddleClicked | TrayEvent::OpenRadar => self.open_main(Some(2)),
            TrayEvent::Scrolled(delta) => {
                let (count, active) = {
                    let s = self.settings.borrow();
                    (s.places.len(), s.active_place)
                };
                if count > 1 {
                    let next = if delta > 0 { active + 1 } else { active + count - 1 };
                    self.select_place(next % count);
                }
            }
            TrayEvent::OpenWindow => self.open_main(None),
            TrayEvent::FetchNow => {
                if let Some(control) = self.control.borrow().as_ref() {
                    control.now.notify_one();
                }
            }
            TrayEvent::SelectPlace(index) => self.select_place(index),
            TrayEvent::TogglePause => {
                {
                    let mut s = self.settings.borrow_mut();
                    s.alerts.paused = !s.alerts.paused;
                }
                self.save_settings();
                self.show_weather();
            }
            TrayEvent::OpenSettings => self.open_settings(None),
            TrayEvent::OpenLog => self.open_log(),
            TrayEvent::Quit => self.quit(),
            TrayEvent::Failed(text) => {
                tracing::error!(error = %text, "[Tray] The tray icon failed");
                self.open_main(None);
                self.toast("The tray is not available. RustWeather stays in this window.", 3);
            }
        }
    }

    pub fn open_log(&self) {
        if let Err(e) = opener::open(self.paths.logs()) {
            self.toast(&format!("Could not open the logs: {e}"), 3);
        }
    }

    pub fn quit(&self) {
        self.save_ui();
        self.radar.send(Command::Close);
        let _ = slint::quit_event_loop();
    }
}

/// The window class: the desktop file and the task manager match on it, so the taskbar shows the right icon.
fn with_window_class(attributes: WindowAttributes) -> WindowAttributes {
    let attributes =
        slint::winit_030::winit::platform::x11::WindowAttributesExtX11::with_name(attributes, APP_ID, "rustweather");
    slint::winit_030::winit::platform::wayland::WindowAttributesExtWayland::with_name(attributes, APP_ID, "rustweather")
}

fn select_backend() -> Result<(), slint::PlatformError> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(with_window_class)
        .select()
}

fn view_from(ui: &UiState) -> View {
    View {
        lon: ui.view_lon,
        lat: ui.view_lat,
        zoom: ui.view_zoom,
        width: 900,
        height: 640,
    }
}

pub fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    if !cli.allow_multiple && instance::signal_running() {
        return Ok(());
    }
    let paths = cli
        .home
        .as_deref()
        .map_or_else(Paths::standard, |h| Ok(Paths::under(h)))?;
    paths.create_all()?;
    let mut settings = Settings::load(&paths)?;
    let _log = logging::init(
        &paths.logs(),
        cli.log_level.as_deref().unwrap_or(&settings.general.log_level),
    );
    let mut ui: UiState = store::load(&paths.ui_state_file()).unwrap_or_default();
    if !ui.legacy_import_done {
        ui.legacy_import_done = true;
        if let Some(path) = legacy::candidates().into_iter().find(|p| p.exists())
            && legacy::import(&path, &mut settings)
        {
            tracing::info!(path = %path.display(), "[App] Imported the old settings");
            settings.save(&paths.settings_file())?;
        }
    }
    if let Some(minutes) = cli.interval {
        settings.general.refresh_minutes = minutes.max(1);
    }
    if let Some(url) = &cli.url {
        settings.general.station_url.clone_from(url);
    }
    let secrets_store = Secrets::load(&paths.secrets_file())?;
    select_backend()?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(3)
        .enable_all()
        .build()?;
    let http = Http::new()?;
    let main = MainWindow::new()?;
    let settings_window = SettingsWindow::new()?;
    let flyout = FlyoutWindow::new()?;
    let mini = MiniWindow::new()?;
    let mode = match settings.general.theme {
        rustweather_core::settings::ThemeMode::System => 0,
        rustweather_core::settings::ThemeMode::Dark => 1,
        rustweather_core::settings::ThemeMode::Light => 2,
    };
    main.global::<Theme>().set_mode(mode);
    settings_window.global::<Theme>().set_mode(mode);
    flyout.global::<Theme>().set_mode(mode);
    mini.global::<Theme>().set_mode(mode);

    let view = view_from(&ui);
    let (settings_tx, settings_rx) = watch::channel(settings.clone());
    let (tray_tx, tray_rx) = mpsc::channel();
    let (tray_events, tray_inbox) = mpsc::channel();
    tray::start(tray_events, tray_rx);

    let config = Config {
        source: settings.radar.source,
        basemap: settings.radar.basemap,
        palette: settings.radar.palette,
        opacity: settings.radar.opacity as f32 / 100.0,
        smooth: settings.radar.smooth,
        labels_above_rain: settings.radar.labels_above_rain,
        dark: main.global::<Theme>().get_dark(),
        carto_key: secrets_store.get(secrets::CARTO).map(str::to_owned),
        alerts_layer: ui.layer_alerts,
        clouds_layer: ui.layer_clouds,
        lightning_layer: ui.layer_lightning,
        wind_layer: ui.layer_wind,
    };
    if matches!(settings.radar.basemap, Basemap::CartoDark | Basemap::CartoVoyager) && config.carto_key.is_none() {
        tracing::info!("[App] No CARTO key yet, the own map stands in");
    }
    let setup = Setup {
        http: http.clone(),
        cache_dir: paths.cache.clone(),
        runtime: runtime.handle().clone(),
        view,
        config,
    };
    let radar = engine::start(setup, |event| with_app(move |app| radar_ui::on_event(app, event)))?;

    let start = weather::load_cached(&paths.cache).unwrap_or_default();
    let bg = cli.bg_color.as_deref().and_then(Rgb::parse);
    let fg = cli.fg_color.as_deref().and_then(Rgb::parse);
    let app = Rc::new(App {
        main,
        settings_window,
        flyout,
        mini,
        paths,
        settings: RefCell::new(settings),
        secrets: RefCell::new(secrets_store),
        ui: RefCell::new(ui),
        runtime,
        http,
        radar,
        view: Cell::new(view),
        snapshot: RefCell::new(start.clone()),
        notifier: RefCell::new(Notifier::default()),
        settings_tx,
        control: RefCell::new(None),
        tray_tx,
        cli_bg: bg,
        cli_fg: fg,
        position: Cell::new(0.0),
        frame_count: Cell::new(0),
        live: Cell::new(0),
        times: RefCell::new(Vec::new()),
        playing: Cell::new(false),
        play_timer: slint::Timer::default(),
        play_last: Cell::new(Instant::now()),
        toast_timer: slint::Timer::default(),
        search_results: RefCell::new(Vec::new()),
        flyout_closed_at: Cell::new(Instant::now() - Duration::from_secs(5)),
    });
    APP.with(|a| *a.borrow_mut() = Some(Rc::clone(&app)));

    radar_ui::wire(&app);
    settings_ui::wire(&app);
    app.main.set_page(app.ui.borrow().page);

    let control = weather::spawn(
        app.runtime.handle(),
        app.http.clone(),
        app.paths.cache.clone(),
        settings_rx,
        start,
        |snapshot| {
            with_app(move |app| {
                *app.snapshot.borrow_mut() = snapshot;
                app.show_weather();
                settings_ui::refresh_health(app);
            });
        },
    );
    *app.control.borrow_mut() = Some(control);
    app.show_weather();

    // Behind a locked screen the radar neither fetches nor animates; it catches up when the user is back.
    let away_handle = app.runtime.handle().clone();
    let away_radar = app.radar.clone();
    away_handle.spawn(async move {
        loop {
            let away = tokio::task::spawn_blocking(presence::away).await.unwrap_or(false);
            away_radar.send(Command::Away(away));
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });

    // Tray events arrive on a plain channel; a thread hands them to the UI thread.
    std::thread::Builder::new().name("tray-events".into()).spawn(move || {
        while let Ok(event) = tray_inbox.recv() {
            with_app(move |app| app.on_tray(event));
        }
    })?;
    if !cli.allow_multiple {
        instance::listen(|| with_app(|app| app.open_main(None)));
    }
    let page = match cli.open.as_deref() {
        Some("now") => Some(0),
        Some("forecast") => Some(1),
        Some("radar") => Some(2),
        Some("alerts") => Some(3),
        _ => None,
    };
    if page.is_some() {
        app.open_main(page);
    }
    match cli.open.as_deref() {
        Some("settings") => app.open_settings(None),
        Some("flyout") => app.toggle_flyout(1000, 760),
        Some("mini") => {
            let _ = app.mini.show();
            app.update_radar_activity();
        }
        _ => {}
    }
    slint::run_event_loop_until_quit()?;
    Ok(())
}
