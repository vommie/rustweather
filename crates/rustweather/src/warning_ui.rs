//! The warnings of a place on the map: the panel, the marked district, and the district under the pointer.

use std::cell::RefCell;

use rustweather_core::map::{in_germany, to_unit};
use rustweather_core::sources::warnings::{self, Outline};
use slint::{ModelRc, VecModel};

use crate::app::{App, with_app};
use crate::{models, radar_ui};

fn show_outline(app: &App, outline: Option<Outline>) {
    let (commands, width, height) = outline.as_ref().map_or((String::new(), 0.0, 0.0), Outline::path);
    app.main.set_warning_path(commands.into());
    app.main.set_warning_path_width(width);
    app.main.set_warning_path_height(height);
    *app.warning_outline.borrow_mut() = outline;
    radar_ui::place_layers(app);
}

fn show_hover(app: &App, outline: Option<Outline>) {
    let (commands, width, height) = outline.as_ref().map_or((String::new(), 0.0, 0.0), Outline::path);
    app.main.set_hover_path(commands.into());
    app.main.set_hover_path_width(width);
    app.main.set_hover_path_height(height);
    *app.hover_outline.borrow_mut() = outline;
    radar_ui::place_layers(app);
}

pub fn close(app: &App) {
    app.main.set_warning_area("".into());
    app.main.set_warning_panel(ModelRc::default());
    show_outline(app, None);
}

/// A click on the map: the panel opens when there is a warning, and what was open closes when there is none.
pub fn show_at(app: &App, lat: f64, lon: f64) {
    if !app.ui.borrow().layer_alerts {
        return;
    }
    let http = app.http.clone();
    app.runtime.spawn(async move {
        let result = warnings::at(&http, lon, lat).await;
        with_app(move |app| match result {
            Ok(found) if found.alerts.is_empty() => close(app),
            Ok(found) => {
                app.main.set_warning_area(found.area.into());
                app.main
                    .set_warning_panel(ModelRc::new(VecModel::from(models::alert_rows(&found.alerts))));
                show_outline(app, found.outline);
            }
            Err(e) => tracing::warn!(error = %e, "[Radar] Warnings for the clicked place failed"),
        });
    });
}

/// The pointer came to rest. Nothing is asked while it stays in the district it already marks, or outside of Germany.
pub fn hover_at(app: &App, lat: f64, lon: f64) {
    let (x, y) = to_unit(lon, lat);
    let inside = |outline: &RefCell<Option<Outline>>| outline.borrow().as_ref().is_some_and(|o| o.contains(x, y));
    if inside(&app.hover_outline) {
        return;
    }
    let turn = app.hover_turn.get() + 1;
    app.hover_turn.set(turn);
    if inside(&app.warning_outline) || !in_germany(lon, lat) {
        show_hover(app, None);
        return;
    }
    let http = app.http.clone();
    app.runtime.spawn(async move {
        let result = warnings::at(&http, lon, lat).await;
        with_app(move |app| {
            if app.hover_turn.get() != turn {
                return;
            }
            match result {
                Ok(found) => show_hover(app, found.outline),
                Err(e) => tracing::debug!(error = %e, "[Radar] Warnings under the pointer failed"),
            }
        });
    });
}
