//! What the app remembers between runs without being asked: windows, page, map view, layers.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiState {
    pub page: i32,
    pub window_width: f32,
    pub window_height: f32,
    pub view_lon: f64,
    pub view_lat: f64,
    pub view_zoom: f64,
    pub views: Vec<SavedView>,
    pub layer_alerts: bool,
    pub layer_wind: bool,
    pub layer_clouds: bool,
    pub layer_lightning: bool,
    pub legacy_import_done: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            page: 0,
            window_width: 1100.0,
            window_height: 760.0,
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
            legacy_import_done: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedView {
    pub name: String,
    pub lon: f64,
    pub lat: f64,
    pub zoom: f64,
}
