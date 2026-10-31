//! RustWeather without any UI: settings, weather sources, the radar engine, the map, the tray picture.

pub mod archive;
pub mod color;
pub mod error;
pub mod http;
pub mod legacy;
pub mod map;
pub mod model;
pub mod paths;
pub mod radar;
pub mod secrets;
pub mod settings;
pub mod sources;
pub mod store;
pub mod timeline;
pub mod tooltip;
pub mod traffic;
pub mod tray_icon;
pub mod ui_state;

pub const APP_ID: &str = "io.github.vommie.rustweather";
pub const APP_NAME: &str = "RustWeather";
