//! What the app remembers between runs without being asked: windows, page, map view, layers.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiState {
    pub page: i32,
    pub main_window: Option<WindowState>,
    pub settings_page: i32,
    pub mini_window: Option<WindowState>,
    pub view_lon: f64,
    pub view_lat: f64,
    pub view_zoom: f64,
    pub views: Vec<SavedView>,
    pub layer_alerts: bool,
    pub layer_wind: bool,
    pub layer_clouds: bool,
    pub layer_lightning: bool,
    pub layer_temps: bool,
    pub layer_heat: bool,
    pub legacy_import_done: bool,
    /// Where the tray icon was clicked last: the tray protocol tells it only then.
    pub tray_position: Option<(i32, i32)>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            page: 0,
            main_window: None,
            settings_page: 0,
            mini_window: None,
            view_lon: 10.45,
            view_lat: 51.2,
            view_zoom: 5.6,
            views: vec![
                SavedView {
                    name: "Germany".into(),
                    lon: 10.45,
                    lat: 51.2,
                    zoom: 5.6,
                },
                SavedView {
                    name: "Berlin".into(),
                    lon: 13.4,
                    lat: 52.5,
                    zoom: 7.5,
                },
            ],
            layer_alerts: true,
            layer_wind: false,
            layer_clouds: false,
            layer_lightning: false,
            layer_temps: false,
            layer_heat: false,
            legacy_import_done: false,
            tray_position: None,
        }
    }
}

/// Where a window was. Position and size are those of its last windowed state, so un-maximizing after a restart works.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedView {
    pub name: String,
    pub lon: f64,
    pub lat: f64,
    pub zoom: f64,
}
