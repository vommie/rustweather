//! The settings window: shows the settings and applies every change at once.

use std::rc::Rc;
use std::time::Duration;

use rustweather_core::map::tiles;
use rustweather_core::radar::engine::{Command, carto_key_works};
use rustweather_core::secrets;
use rustweather_core::settings::{Basemap, Place, RadarSource, RainPalette, Settings, TempUnit, ThemeMode, WindUnit};
use rustweather_core::sources::geocode;
use slint::{ComponentHandle as _, ModelRc, SharedString, VecModel};

use crate::app::{App, with_app};
use crate::desktop::State;
use crate::{DesktopRow, HealthRow, PlaceRow, SettingsData, Theme};

const REFRESH: [u32; 5] = [5, 10, 15, 30, 60];
const LOG_LEVELS: [&str; 4] = ["error", "warn", "info", "debug"];
const CACHE_MB: [u32; 5] = [100, 300, 500, 1000, 2000];
const RADAR_HOURS: [u32; 4] = [6, 24, 72, 168];
const QUIET_FROM: [u32; 7] = [18, 19, 20, 21, 22, 23, 0];
const QUIET_TO: [u32; 6] = [5, 6, 7, 8, 9, 10];

fn index_of<T: PartialEq>(list: &[T], value: &T, default: i32) -> i32 {
    list.iter()
        .position(|v| v == value)
        .and_then(|i| i32::try_from(i).ok())
        .unwrap_or(default)
}

fn pick<T: Copy>(list: &[T], index: i32, fallback: T) -> T {
    usize::try_from(index)
        .ok()
        .and_then(|i| list.get(i).copied())
        .unwrap_or(fallback)
}

fn data(app: &App, s: &Settings) -> SettingsData {
    let cache_used = tiles::folder_size(&app.paths.cache);
    let limit = u64::from(s.cache.max_mb) * 1_000_000;
    SettingsData {
        theme: match s.general.theme {
            ThemeMode::System => 0,
            ThemeMode::Dark => 1,
            ThemeMode::Light => 2,
        },
        refresh: index_of(&REFRESH, &s.general.refresh_minutes, 1),
        autostart: app.desktop.autostart_enabled(),
        log_level: index_of(&LOG_LEVELS, &s.general.log_level.as_str(), 2),
        station_url: s.general.station_url.as_str().into(),
        tile_bg: s.tray.background.as_str().into(),
        tile_fg: s.tray.foreground.as_str().into(),
        show_rain: s.tray.show_rain_dot,
        show_alert: s.tray.show_alert,
        show_stale: s.tray.show_stale,
        show_trend: s.tray.show_trend,
        line_feels: s.tray.line_feels_like,
        line_range: s.tray.line_min_max,
        line_rain: s.tray.line_rain,
        line_alerts: s.tray.line_alerts,
        line_wind: s.tray.line_wind,
        line_humidity: s.tray.line_humidity,
        line_sun: s.tray.line_sun,
        line_age: s.tray.line_age,
        line_next: s.tray.line_next,
        source: match s.radar.source {
            RadarSource::Auto => 0,
            RadarSource::Dwd => 1,
            RadarSource::RainViewer => 2,
        },
        basemap: match s.radar.basemap {
            Basemap::CartoDark => 0,
            Basemap::CartoVoyager => 1,
            Basemap::Own => 2,
            Basemap::Osm => 3,
        },
        palette: match s.radar.palette {
            RainPalette::Classic => 0,
            RainPalette::Calm => 1,
            RainPalette::Blue => 2,
        },
        opacity: s.radar.opacity as f32,
        speed: s.radar.frame_ms as f32,
        smooth: s.radar.smooth,
        looping: s.radar.loop_playback,
        labels: s.radar.labels_above_rain,
        has_carto: app.secrets.borrow().get(secrets::CARTO).is_some(),
        cache_limit: index_of(&CACHE_MB, &s.cache.max_mb, 1),
        radar_hours: index_of(&RADAR_HOURS, &s.cache.radar_hours, 1),
        cache_text: format!("In use: {} MB of {} MB", cache_used / 1_000_000, s.cache.max_mb).into(),
        cache_share: (cache_used as f32 / limit.max(1) as f32).min(1.0),
        al_paused: s.alerts.paused,
        al_rain: s.alerts.rain,
        al_end: s.alerts.rain_end,
        al_dwd: s.alerts.dwd,
        al_storm: s.alerts.thunderstorm,
        al_frost: s.alerts.frost,
        al_heat: s.alerts.heat,
        al_level: i32::from(s.alerts.min_level) - 1,
        al_lead: s.alerts.lead_minutes as f32,
        al_night: s.alerts.quiet_at_night,
        al_from: index_of(&QUIET_FROM, &s.alerts.quiet_from_hour, 4),
        al_to: index_of(&QUIET_TO, &s.alerts.quiet_to_hour, 2),
        al_severe: s.alerts.severe_ignores_quiet,
        al_full: s.alerts.quiet_when_fullscreen,
        temp_unit: i32::from(s.units.temperature == TempUnit::Fahrenheit),
        wind_unit: match s.units.wind {
            WindUnit::Kmh => 0,
            WindUnit::Ms => 1,
            WindUnit::Beaufort => 2,
        },
    }
}

fn places(s: &Settings) -> Vec<PlaceRow> {
    s.places
        .iter()
        .enumerate()
        .map(|(i, p)| PlaceRow {
            name: p.name.as_str().into(),
            active: i == s.active_place,
        })
        .collect()
}

pub fn refresh(app: &App) {
    let s = app.settings.borrow();
    let window = &app.settings_window;
    window.set_data(data(app, &s));
    window.set_places(ModelRc::new(VecModel::from(places(&s))));
    window.set_version(env!("CARGO_PKG_VERSION").into());
    drop(s);
    refresh_health(app);
    refresh_desktop(app);
}

/// The Desktop page: what is installed, and what the buttons can do.
pub fn refresh_desktop(app: &App) {
    let window = &app.settings_window;
    let status = app.desktop.status();
    let (title, detail, level, install) = match status.state {
        State::Installed => ("Installed", "RustWeather is part of your desktop.", 1, "Install again"),
        State::Missing => (
            "Not installed",
            "Add RustWeather to the application menu and give it its icon.",
            0,
            "Install",
        ),
        State::Outdated => (
            "Needs repair",
            "The menu entry belongs to a program in another place, or parts are missing.",
            2,
            "Repair",
        ),
        State::Package => (
            "Installed by a package",
            "Your package manager takes care of the menu entry and the icons.",
            1,
            "",
        ),
    };
    window.set_desktop_title(title.into());
    window.set_desktop_detail(detail.into());
    window.set_desktop_level(level);
    window.set_desktop_install(install.into());
    window.set_desktop_can_remove(status.removable());
    let rows: Vec<DesktopRow> = status
        .rows()
        .into_iter()
        .map(|(name, ok, detail)| DesktopRow {
            name: name.into(),
            ok,
            detail: detail.into(),
        })
        .collect();
    window.set_desktop_rows(ModelRc::new(VecModel::from(rows)));
    let mut data = window.get_data();
    data.autostart = app.desktop.autostart_enabled();
    window.set_data(data);
}

fn desktop_done(app: &App, result: std::io::Result<()>, done: &str) {
    match result {
        Ok(()) => app.toast(done, 1),
        Err(e) => app.toast(&format!("Desktop integration failed: {e}"), 3),
    }
    refresh_desktop(app);
}

pub fn refresh_health(app: &App) {
    let snapshot = app.snapshot.borrow();
    let rows: Vec<HealthRow> = snapshot
        .health
        .iter()
        .map(|h| HealthRow {
            name: h.name.as_str().into(),
            state: match h.ok {
                None => 0,
                Some(true) => 1,
                Some(false) => 3,
            },
            last: h
                .at
                .map_or(String::new(), |t| {
                    t.with_timezone(&chrono::Local).format("%H:%M").to_string()
                })
                .into(),
            latency: format!("{} ms", h.millis).into(),
            detail: h.detail.as_str().into(),
        })
        .collect();
    app.settings_window.set_health(ModelRc::new(VecModel::from(rows)));
}

fn set_bool(s: &mut Settings, key: &str, value: bool) {
    match key {
        "show-rain" => s.tray.show_rain_dot = value,
        "show-alert" => s.tray.show_alert = value,
        "show-stale" => s.tray.show_stale = value,
        "show-trend" => s.tray.show_trend = value,
        "line-feels" => s.tray.line_feels_like = value,
        "line-range" => s.tray.line_min_max = value,
        "line-rain" => s.tray.line_rain = value,
        "line-alerts" => s.tray.line_alerts = value,
        "line-wind" => s.tray.line_wind = value,
        "line-humidity" => s.tray.line_humidity = value,
        "line-sun" => s.tray.line_sun = value,
        "line-age" => s.tray.line_age = value,
        "line-next" => s.tray.line_next = value,
        "smooth" => s.radar.smooth = value,
        "looping" => s.radar.loop_playback = value,
        "labels" => s.radar.labels_above_rain = value,
        "al-paused" => s.alerts.paused = value,
        "al-rain" => s.alerts.rain = value,
        "al-end" => s.alerts.rain_end = value,
        "al-dwd" => s.alerts.dwd = value,
        "al-storm" => s.alerts.thunderstorm = value,
        "al-frost" => s.alerts.frost = value,
        "al-heat" => s.alerts.heat = value,
        "al-night" => s.alerts.quiet_at_night = value,
        "al-severe" => s.alerts.severe_ignores_quiet = value,
        "al-full" => s.alerts.quiet_when_fullscreen = value,
        _ => {}
    }
}

fn set_int(s: &mut Settings, key: &str, value: i32) {
    match key {
        "theme" => {
            s.general.theme = pick(
                &[ThemeMode::System, ThemeMode::Dark, ThemeMode::Light],
                value,
                ThemeMode::System,
            )
        }
        "refresh" => s.general.refresh_minutes = pick(&REFRESH, value, 10),
        "log-level" => s.general.log_level = pick(&LOG_LEVELS, value, "info").to_owned(),
        "source" => {
            s.radar.source = pick(
                &[RadarSource::Auto, RadarSource::Dwd, RadarSource::RainViewer],
                value,
                RadarSource::Auto,
            )
        }
        "basemap" => {
            s.radar.basemap = pick(
                &[Basemap::CartoDark, Basemap::CartoVoyager, Basemap::Own, Basemap::Osm],
                value,
                Basemap::Own,
            )
        }
        "palette" => {
            s.radar.palette = pick(
                &[RainPalette::Classic, RainPalette::Calm, RainPalette::Blue],
                value,
                RainPalette::Classic,
            )
        }
        "cache-limit" => s.cache.max_mb = pick(&CACHE_MB, value, 300),
        "radar-hours" => s.cache.radar_hours = pick(&RADAR_HOURS, value, 24),
        "al-level" => s.alerts.min_level = u8::try_from(value + 1).unwrap_or(2),
        "al-from" => s.alerts.quiet_from_hour = pick(&QUIET_FROM, value, 22),
        "al-to" => s.alerts.quiet_to_hour = pick(&QUIET_TO, value, 7),
        "temp-unit" => {
            s.units.temperature = if value == 1 {
                TempUnit::Fahrenheit
            } else {
                TempUnit::Celsius
            }
        }
        "wind-unit" => s.units.wind = pick(&[WindUnit::Kmh, WindUnit::Ms, WindUnit::Beaufort], value, WindUnit::Kmh),
        _ => {}
    }
}

fn apply(app: &App) {
    app.save_settings();
    let mode = match app.settings.borrow().general.theme {
        ThemeMode::System => 0,
        ThemeMode::Dark => 1,
        ThemeMode::Light => 2,
    };
    app.main.global::<Theme>().set_mode(mode);
    app.settings_window.global::<Theme>().set_mode(mode);
    app.flyout.global::<Theme>().set_mode(mode);
    app.mini.global::<Theme>().set_mode(mode);
    app.push_radar_config();
    app.show_weather();
}

pub fn wire(app: &Rc<App>) {
    let window = &app.settings_window;
    let weak = Rc::downgrade(app);
    macro_rules! on {
        ($setter:ident, |$app:ident $(, $arg:ident)*| $body:block) => {{
            let weak = weak.clone();
            window.$setter(move |$($arg),*| {
                if let Some($app) = weak.upgrade() { $body }
            });
        }};
    }
    on!(on_set_bool, |app, key, value| {
        if key == "autostart" {
            if let Err(e) = app.desktop.set_autostart(value) {
                app.toast(&format!("Could not change the autostart: {e}"), 3);
            }
            refresh_desktop(&app);
        } else {
            set_bool(&mut app.settings.borrow_mut(), &key, value);
            apply(&app);
        }
    });
    on!(on_set_int, |app, key, value| {
        set_int(&mut app.settings.borrow_mut(), &key, value);
        apply(&app);
    });
    on!(on_set_float, |app, key, value| {
        {
            let mut s = app.settings.borrow_mut();
            match key.as_str() {
                "opacity" => s.radar.opacity = value as u32,
                "speed" => s.radar.frame_ms = value as u32,
                "al-lead" => s.alerts.lead_minutes = value as u32,
                _ => {}
            }
        }
        apply(&app);
    });
    on!(on_set_text, |app, key, value| {
        {
            let mut s = app.settings.borrow_mut();
            match key.as_str() {
                "station-url" => s.general.station_url = value.to_string(),
                "tile-bg" => s.tray.background = value.to_string(),
                "tile-fg" => s.tray.foreground = value.to_string(),
                _ => {}
            }
        }
        app.save_settings();
        app.show_weather();
    });
    on!(on_search_place, |app, text| {
        let http = app.http.clone();
        let query = text.to_string();
        app.runtime.spawn(async move {
            let result = geocode::search(&http, &query).await;
            with_app(move |app| match result {
                Ok(found) if found.is_empty() => app.toast("No place found under this name.", 3),
                Ok(found) => {
                    let names: Vec<SharedString> = found.iter().map(|p| p.name.as_str().into()).collect();
                    app.settings_window
                        .set_search_results(ModelRc::new(VecModel::from(names)));
                    *app.search_results.borrow_mut() = found;
                }
                Err(e) => app.toast(&format!("The search failed: {e}"), 3),
            });
        });
    });
    on!(on_add_place, |app, index| {
        let found: Option<Place> = app
            .search_results
            .borrow()
            .get(usize::try_from(index).unwrap_or(0))
            .cloned();
        if let Some(place) = found {
            app.settings.borrow_mut().places.push(place);
            app.settings_window.set_search_results(ModelRc::default());
            app.save_settings();
            refresh(&app);
            app.show_weather();
        }
    });
    on!(on_remove_place, |app, index| {
        {
            let mut s = app.settings.borrow_mut();
            let i = usize::try_from(index).unwrap_or(0);
            if s.places.len() > 1 && i < s.places.len() {
                s.places.remove(i);
                s.active_place = s.active_place.min(s.places.len() - 1);
            }
        }
        app.save_settings();
        refresh(&app);
        app.show_weather();
    });
    on!(on_activate_place, |app, index| {
        app.select_place(usize::try_from(index).unwrap_or(0));
        refresh(&app);
    });
    on!(on_save_key, |app, key| {
        app.secrets.borrow_mut().set(secrets::CARTO, &key);
        match app.secrets.borrow().save(&app.paths.secrets_file()) {
            Ok(()) => app.settings_window.set_key_state("Saved.".into()),
            Err(e) => app.toast(&format!("Could not save the key: {e}"), 3),
        }
        app.push_radar_config();
        refresh(&app);
    });
    on!(on_test_key, |app| {
        let Some(key) = app.secrets.borrow().get(secrets::CARTO).map(str::to_owned) else {
            app.settings_window.set_key_state("No key stored yet.".into());
            return;
        };
        app.settings_window.set_key_state("Testing …".into());
        let http = app.http.clone();
        app.runtime.spawn(async move {
            let result = carto_key_works(&http, &key).await;
            with_app(move |app| {
                let (text, level) = match result {
                    Ok(true) => ("Key works.".to_owned(), 1),
                    Ok(false) => ("CARTO rejected this key.".to_owned(), 3),
                    Err(e) => (format!("Could not test the key: {e}"), 3),
                };
                app.settings_window.set_key_state(text.as_str().into());
                app.toast(&text, level);
            });
        });
    });
    on!(on_check_all, |app| {
        if let Some(control) = app.control.borrow().as_ref() {
            control.now.notify_one();
        }
        app.radar.send(Command::Reload);
    });
    on!(on_copy_report, |app| {
        let snapshot = app.snapshot.borrow();
        let mut text = format!("RustWeather {}\n", env!("CARGO_PKG_VERSION"));
        for h in &snapshot.health {
            text.push_str(&format!("{}: {:?} {} ms {}\n", h.name, h.ok, h.millis, h.detail));
        }
        app.settings_window.invoke_copy_text(text.into());
        app.toast("Report copied.", 1);
    });
    on!(on_open_log, |app| {
        app.open_log();
    });
    on!(on_install_desktop, |app| {
        let result = app.desktop.install();
        desktop_done(&app, result, "RustWeather is now part of your desktop.");
    });
    on!(on_remove_desktop, |app| {
        let result = app.desktop.uninstall();
        desktop_done(
            &app,
            result,
            "Menu entry, icons and autostart are removed. Your settings stay.",
        );
    });
    on!(on_clear_cache, |app| {
        let tiles_dir = app.paths.cache.join("tiles");
        if let Err(e) = std::fs::remove_dir_all(&tiles_dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            app.toast(&format!("Could not empty the cache: {e}"), 3);
        }
        refresh(&app);
    });
    on!(on_open_cache, |app| {
        if let Err(e) = opener::open(&app.paths.cache) {
            app.toast(&format!("Could not open the folder: {e}"), 3);
        }
    });
    on!(on_offline_map, |app| {
        app.radar.send(Command::PreloadGermany);
        app.toast("Fetching Germany's map in the background.", 0);
    });

    // Tidy the cache now and then; the limit applies without any help.
    let cache_dir = app.paths.cache.clone();
    let settings = app.settings_tx.subscribe();
    app.runtime.spawn(async move {
        loop {
            let (max, hours) = {
                let s = settings.borrow();
                (u64::from(s.cache.max_mb) * 1_000_000, u64::from(s.cache.radar_hours))
            };
            let dir = cache_dir.clone();
            let _ =
                tokio::task::spawn_blocking(move || tiles::prune(&dir, max, Duration::from_secs(hours * 3600))).await;
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}
