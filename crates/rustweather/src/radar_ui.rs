//! Connects the radar page and the small windows to the radar engine.

use std::time::{Duration, Instant};

use chrono::Local;
use rustweather_core::map::View;
use rustweather_core::radar::FrameState;
use rustweather_core::radar::engine::{Command, Event, Notice, Picture};
use rustweather_core::ui_state::SavedView;
use slint::winit_030::WinitWindowAccessor as _;
use slint::{ComponentHandle as _, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use crate::app::App;
use crate::{CityLabel, FrameCell, ViewChip};

fn image(picture: &Picture) -> Image {
    Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        &picture.rgba,
        picture.width,
        picture.height,
    ))
}

fn cell(state: FrameState, forecast: bool) -> FrameCell {
    let state = match state {
        FrameState::Missing => 0,
        FrameState::Loading => 1,
        FrameState::Ready => 2,
        FrameState::Failed => 3,
    };
    FrameCell { state, forecast }
}

pub fn on_event(app: &App, event: Event) {
    let w = &app.main;
    match event {
        Event::Base(p) => {
            let img = image(&p);
            w.set_base(img.clone());
            app.mini.set_base(img.clone());
            app.flyout.set_map(img);
        }
        Event::Overlay(p) => w.set_overlay(p.as_ref().map_or_else(Image::default, image)),
        Event::Rain(p) => {
            let img = image(&p);
            w.set_rain(img.clone());
            app.mini.set_rain(img.clone());
            app.flyout.set_rain(img);
        }
        Event::Labels(p) => w.set_labels(p.as_ref().map_or_else(Image::default, image)),
        Event::Cities(list) => {
            let scale = w.window().scale_factor();
            let cities: Vec<CityLabel> = list
                .into_iter()
                .map(|(name, x, y)| CityLabel {
                    name: name.into(),
                    x: x / scale,
                    y: y / scale,
                })
                .collect();
            w.set_cities(ModelRc::new(VecModel::from(cities)));
        }
        Event::Timeline { frames, live, source } => {
            let cells: Vec<FrameCell> = frames.iter().map(|f| cell(f.state, f.forecast)).collect();
            app.frame_count.set(cells.len());
            app.live.set(live);
            *app.times.borrow_mut() = frames.iter().map(|f| f.time).collect();
            w.set_frames(ModelRc::new(VecModel::from(cells)));
            w.set_live(i32::try_from(live).unwrap_or(0));
            w.set_source_label(source.into());
            if app.position.get() as usize >= frames.len() || app.position.get() == 0.0 {
                set_position(app, live as f32);
            }
            show_time(app);
        }
        Event::Frame { index, state } => {
            use slint::Model as _;
            let model = w.get_frames();
            if let Some(mut c) = model.row_data(index) {
                let new = cell(state, c.forecast);
                if new.state != c.state {
                    c.state = new.state;
                    model.set_row_data(index, c);
                }
            }
        }
        Event::Notice(notice) => on_notice(app, notice),
        Event::Probe { title, text } => {
            w.set_probe_title(title.into());
            w.set_probe_text(text.into());
        }
    }
}

fn set_status(app: &App, text: &str, level: i32) {
    app.main.set_status(text.into());
    app.main.set_status_level(level);
}

fn set_banner(app: &App, text: &str, error: bool, action: &str) {
    app.main.set_banner(text.into());
    app.main.set_banner_error(error);
    app.main.set_banner_action(action.into());
}

fn on_notice(app: &App, notice: Notice) {
    match notice {
        Notice::Ready { source, time } => {
            app.main.set_loading(0.0);
            if app.main.get_banner_action() == "Try now" {
                set_banner(app, "", false, "");
            }
            let at = time.map_or(String::new(), |t| t.with_timezone(&Local).format("%H:%M").to_string());
            set_status(app, &format!("Radar: {source}, {at}"), 1);
        }
        Notice::Loading { done, total } => {
            app.main.set_loading(if total == 0 {
                0.0
            } else {
                ((done + 1) as f32 / (total + 1) as f32).min(0.98)
            });
            let text = if total > 1 {
                format!("Loading radar picture {done} of {total}")
            } else {
                "Loading the radar".to_owned()
            };
            set_status(app, &text, 4);
        }
        Notice::Slow => {
            set_status(app, "The network is slow, the radar keeps loading", 2);
            app.toast("The network is slow. What is here keeps playing, the rest follows.", 0);
        }
        Notice::Offline { data_from, retry_in } => {
            app.main.set_loading(0.0);
            let from = data_from.map_or("the last known state".to_owned(), |t| {
                t.with_timezone(&Local).format("%H:%M").to_string()
            });
            tracing::debug!(retry_in, "[Radar] Waiting to try again");
            set_banner(
                app,
                &format!("No connection. Showing data from {from}. Trying again by itself."),
                false,
                "Try now",
            );
            set_status(app, &format!("Offline, showing data from {from}"), 2);
        }
        Notice::BackOnline => {
            set_banner(app, "", false, "");
            app.toast("Back online, the radar is up to date.", 1);
        }
        Notice::RateLimited { host, retry_in } => {
            set_status(app, &format!("{host} limit reached, continuing in {retry_in} s"), 2)
        }
        Notice::BadKey { service } => {
            set_banner(
                app,
                &format!("The {service} key was rejected. The own map stands in."),
                true,
                "Settings",
            );
            app.toast(&format!("The {service} key was rejected."), 3);
        }
        Notice::MissingKey { service } => {
            set_banner(
                app,
                &format!("{service} maps need a key. The own map stands in."),
                false,
                "Settings",
            );
        }
        Notice::Fallback { text } => set_status(app, &text, 2),
    }
}

pub fn set_position(app: &App, position: f32) {
    app.position.set(position);
    app.main.set_position(position);
    app.radar.send(Command::Position(position));
    show_time(app);
}

fn show_time(app: &App) {
    let times = app.times.borrow();
    let index = app.position.get().round() as usize;
    let Some(time) = times.get(index.min(times.len().saturating_sub(1))) else {
        return;
    };
    let minutes = (i64::try_from(index).unwrap_or(0) - i64::try_from(app.live.get()).unwrap_or(0)) * 5;
    let clock = time.with_timezone(&Local).format("%H:%M");
    let text = match minutes {
        0 => format!("{clock} now"),
        m if m > 0 => format!("{clock} +{m} min"),
        m => format!("{clock} {m} min"),
    };
    app.main.set_time_label(text.as_str().into());
    app.mini.set_time_label(text.into());
}

pub fn stop(app: &App) {
    app.playing.set(false);
    app.main.set_playing(false);
    app.play_timer.stop();
}

fn play(app: &std::rc::Rc<App>) {
    app.playing.set(true);
    app.main.set_playing(true);
    app.play_last.set(Instant::now());
    let weak = std::rc::Rc::downgrade(app);
    app.play_timer
        .start(slint::TimerMode::Repeated, Duration::from_millis(33), move || {
            let Some(app) = weak.upgrade() else { return };
            let elapsed = app.play_last.get().elapsed().as_secs_f32() * 1000.0;
            app.play_last.set(Instant::now());
            let (frame_ms, looping, smooth) = {
                let s = app.settings.borrow();
                (s.radar.frame_ms.max(30) as f32, s.radar.loop_playback, s.radar.smooth)
            };
            let last = app.frame_count.get().saturating_sub(1) as f32;
            let mut next = app.position.get() + elapsed / frame_ms;
            if next > last {
                if looping {
                    next = 0.0;
                } else {
                    next = last;
                    stop(&app);
                }
            }
            // Without smoothing the picture only changes on whole frames.
            app.position.set(next);
            app.main.set_position(next);
            app.radar
                .send(Command::Position(if smooth { next } else { next.floor() }));
            show_time(&app);
        });
}

fn apply_view(app: &App, view: View) {
    app.view.set(view);
    app.radar.send(Command::View(view));
    let scale = f64::from(app.main.window().scale_factor());
    let (x, y) = view.pixel_of(app.settings.borrow().place().lon, app.settings.borrow().place().lat);
    app.main.set_home_x((x / scale) as f32);
    app.main.set_home_y((y / scale) as f32);
    // A scale bar of a round length, at most 110 px.
    let meters_per_px = 40_075_016.0 * view.lat.to_radians().cos() / view.scale() * scale;
    let want = meters_per_px * 110.0;
    let magnitude = 10f64.powf(want.log10().floor());
    let nice = [5.0, 2.0, 1.0]
        .iter()
        .map(|m| m * magnitude)
        .find(|v| *v <= want)
        .unwrap_or(magnitude);
    app.main.set_scale_width((nice / meters_per_px) as f32);
    app.main.set_scale_label(
        if nice >= 1000.0 {
            format!("{:.0} km", nice / 1000.0)
        } else {
            format!("{nice:.0} m")
        }
        .into(),
    );
    show_views(app);
}

fn show_views(app: &App) {
    let view = app.view.get();
    let chips: Vec<ViewChip> = app
        .ui
        .borrow()
        .views
        .iter()
        .map(|v| ViewChip {
            name: v.name.as_str().into(),
            active: (v.lon - view.lon).abs() < 0.02
                && (v.lat - view.lat).abs() < 0.02
                && (v.zoom - view.zoom).abs() < 0.05,
        })
        .collect();
    app.main.set_views(ModelRc::new(VecModel::from(chips)));
}

fn show_layers(app: &App) {
    let ui = app.ui.borrow();
    app.main.set_layer_alerts(ui.layer_alerts);
    app.main.set_layer_clouds(ui.layer_clouds);
    app.main.set_layer_lightning(ui.layer_lightning);
}

/// Places the small panel above the icon, inside the screen. Wayland decides by itself where it goes.
pub fn place_flyout(app: &App, x: i32, y: i32) {
    let size = app.flyout.window().size();
    let (w, h) = (
        i32::try_from(size.width).unwrap_or(320),
        i32::try_from(size.height).unwrap_or(372),
    );
    let monitor = app
        .main
        .window()
        .with_winit_window(|win| {
            win.available_monitors()
                .find(|m| {
                    let (p, s) = (m.position(), m.size());
                    x >= p.x
                        && y >= p.y
                        && x < p.x + i32::try_from(s.width).unwrap_or(0)
                        && y < p.y + i32::try_from(s.height).unwrap_or(0)
                })
                .or_else(|| win.primary_monitor())
        })
        .flatten();
    let (left, top, right, bottom) = monitor.map_or((0, 0, 1920, 1080), |m| {
        let (p, s) = (m.position(), m.size());
        (
            p.x,
            p.y,
            p.x + i32::try_from(s.width).unwrap_or(1920),
            p.y + i32::try_from(s.height).unwrap_or(1080),
        )
    });
    let px = (x - w / 2).clamp(left + 8, (right - w - 8).max(left + 8));
    let py = if y > (top + bottom) / 2 { y - h - 16 } else { y + 16 };
    app.flyout.window().set_position(slint::PhysicalPosition::new(
        px,
        py.clamp(top + 8, (bottom - h - 8).max(top + 8)),
    ));
}

pub fn wire(app: &std::rc::Rc<App>) {
    let w = &app.main;
    let weak = std::rc::Rc::downgrade(app);
    macro_rules! on {
        ($setter:ident, |$app:ident $(, $arg:ident)*| $body:block) => {{
            let weak = weak.clone();
            w.$setter(move |$($arg),*| {
                if let Some($app) = weak.upgrade() { $body }
            });
        }};
    }
    on!(on_page_changed, |app, _page| {
        app.update_radar_activity();
        app.save_ui();
    });
    on!(on_open_settings, |app| {
        app.open_settings(None);
    });
    on!(on_resized, |app, width, height| {
        let scale = app.main.window().scale_factor();
        let (w, h) = ((width * scale) as u32, (height * scale) as u32);
        if w > 10 && h > 10 {
            apply_view(
                &app,
                View {
                    width: w,
                    height: h,
                    ..app.view.get()
                },
            );
        }
    });
    on!(on_pan, |app, dx, dy| {
        let scale = app.main.window().scale_factor();
        apply_view(
            &app,
            app.view.get().panned(f64::from(dx * scale), f64::from(dy * scale)),
        );
    });
    on!(on_zoom, |app, steps, x, y| {
        let scale = app.main.window().scale_factor();
        apply_view(
            &app,
            app.view
                .get()
                .zoomed(f64::from(steps), f64::from(x * scale), f64::from(y * scale)),
        );
    });
    on!(on_probe, |app, x, y| {
        let scale = app.main.window().scale_factor();
        let (lon, lat) = app.view.get().lonlat_of(f64::from(x * scale), f64::from(y * scale));
        app.main.set_probe_x(x);
        app.main.set_probe_y(y);
        app.radar.send(Command::Probe { lon, lat });
    });
    on!(on_probe_close, |app| {
        app.main.set_probe_title("".into());
    });
    on!(on_seek, |app, position| {
        stop(&app);
        set_position(&app, position);
    });
    on!(on_toggle_play, |app| {
        if app.playing.get() { stop(&app) } else { play(&app) }
    });
    on!(on_toggle_loop, |app| {
        let on = {
            let mut s = app.settings.borrow_mut();
            s.radar.loop_playback = !s.radar.loop_playback;
            s.radar.loop_playback
        };
        app.main.set_looping(on);
        app.save_settings();
    });
    on!(on_step, |app, by| {
        stop(&app);
        let last = app.frame_count.get().saturating_sub(1) as f32;
        set_position(&app, (app.position.get().round() + by as f32).clamp(0.0, last));
    });
    on!(on_to_live, |app| {
        stop(&app);
        set_position(&app, app.live.get() as f32);
    });
    on!(on_reload, |app| {
        app.radar.send(Command::Reload);
    });
    on!(on_pick_view, |app, index| {
        let saved = app.ui.borrow().views.get(usize::try_from(index).unwrap_or(0)).cloned();
        if let Some(v) = saved {
            apply_view(
                &app,
                View {
                    lon: v.lon,
                    lat: v.lat,
                    zoom: v.zoom,
                    ..app.view.get()
                },
            );
        }
    });
    on!(on_add_view, |app| {
        let view = app.view.get();
        {
            let mut ui = app.ui.borrow_mut();
            let name = format!("View {}", ui.views.len() + 1);
            ui.views.push(SavedView {
                name,
                lon: view.lon,
                lat: view.lat,
                zoom: view.zoom,
            });
        }
        show_views(&app);
        app.save_ui();
    });
    on!(on_toggle_layer, |app, layer| {
        {
            let mut ui = app.ui.borrow_mut();
            match layer {
                0 => ui.layer_alerts = !ui.layer_alerts,
                1 => ui.layer_clouds = !ui.layer_clouds,
                _ => ui.layer_lightning = !ui.layer_lightning,
            }
        }
        show_layers(&app);
        app.push_radar_config();
        app.save_ui();
    });
    on!(on_toggle_full, |app| {
        let full = !app.main.get_full();
        app.main.set_full(full);
        app.main.window().set_fullscreen(full);
    });
    on!(on_mini, |app| {
        let _ = app.mini.show();
        let _ = app.main.hide();
        app.update_radar_activity();
    });
    on!(on_banner_act, |app| {
        if app.main.get_banner_action() == "Settings" {
            app.open_settings(Some(4));
        } else {
            app.radar.send(Command::Reload);
        }
    });

    let weak_mini = weak.clone();
    app.mini.on_restore(move || {
        if let Some(app) = weak_mini.upgrade() {
            let _ = app.mini.hide();
            app.open_main(Some(2));
        }
    });
    let weak_mini = weak.clone();
    app.mini.on_drag(move |dx, dy| {
        if let Some(app) = weak_mini.upgrade() {
            let scale = app.mini.window().scale_factor();
            let p = app.mini.window().position();
            app.mini.window().set_position(slint::PhysicalPosition::new(
                p.x + (dx * scale) as i32,
                p.y + (dy * scale) as i32,
            ));
        }
    });

    let weak_fly = weak.clone();
    app.flyout.on_open_page(move |page| {
        if let Some(app) = weak_fly.upgrade() {
            let _ = app.flyout.hide();
            app.open_main(Some(page));
        }
    });
    let weak_fly = weak.clone();
    app.flyout.on_settings(move || {
        if let Some(app) = weak_fly.upgrade() {
            let _ = app.flyout.hide();
            app.open_settings(None);
            app.update_radar_activity();
        }
    });
    let weak_fly = weak.clone();
    app.flyout.on_refresh(move || {
        if let Some(app) = weak_fly.upgrade()
            && let Some(control) = app.control.borrow().as_ref()
        {
            control.now.notify_one();
        }
    });

    // The windows hide instead of closing: the app lives in the tray.
    let weak_close = weak.clone();
    app.main.window().on_close_requested(move || {
        if let Some(app) = weak_close.upgrade() {
            app.save_ui();
            let _ = app.main.hide();
            app.update_radar_activity();
        }
        slint::CloseRequestResponse::HideWindow
    });
    let weak_close = weak.clone();
    // Clicking elsewhere closes the panel. A window can report "not focused" before it ever had the focus: that does not count.
    let had_focus = std::cell::Cell::new(false);
    app.flyout.window().on_winit_window_event(move |_, event| {
        if let slint::winit_030::winit::event::WindowEvent::Focused(focused) = event {
            tracing::debug!(focused, "[Flyout] Focus changed");
            if *focused {
                had_focus.set(true);
            } else if had_focus.replace(false)
                && let Some(app) = weak_close.upgrade()
            {
                app.flyout_closed_at.set(Instant::now());
                let _ = app.flyout.hide();
                app.update_radar_activity();
            }
        }
        slint::winit_030::EventResult::Propagate
    });

    app.main.set_looping(app.settings.borrow().radar.loop_playback);
    apply_view(app, app.view.get());
    show_layers(app);
}
