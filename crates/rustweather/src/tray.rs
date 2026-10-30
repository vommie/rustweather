//! The tray icon, through the StatusNotifierItem interface directly: Slint's own tray has no rich tooltip and no scroll.

use std::sync::mpsc::{Receiver, Sender};

use ksni::blocking::TrayMethods as _;
use ksni::menu::{MenuItem, StandardItem, SubMenu};
use ksni::{Icon, ToolTip};
use rustweather_core::tray_icon::{self, Look};

#[derive(Debug, Clone)]
pub enum TrayEvent {
    Clicked(i32, i32),
    MiddleClicked,
    Scrolled(i32),
    OpenWindow,
    OpenRadar,
    OpenCard,
    FetchNow,
    SelectPlace(usize),
    TogglePause,
    OpenSettings,
    OpenLog,
    Quit,
    Failed(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrayData {
    pub look: Look,
    pub title: String,
    pub description: String,
    pub places: Vec<String>,
    pub active_place: usize,
    pub paused: bool,
}

struct AppTray {
    data: TrayData,
    events: Sender<TrayEvent>,
}

fn pixmap(look: &Look, size: i32) -> Icon {
    let mut data = tray_icon::render(look, size.unsigned_abs());
    // Straight RGBA to the ARGB in network order the interface wants.
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.rotate_right(1);
    }
    Icon {
        width: size,
        height: size,
        data,
    }
}

impl AppTray {
    fn entry(&self, label: &str, event: TrayEvent) -> MenuItem<Self> {
        StandardItem {
            label: label.into(),
            activate: Box::new(move |tray: &mut Self| {
                let _ = tray.events.send(event.clone());
            }),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for AppTray {
    fn id(&self) -> String {
        "rustweather".into()
    }

    fn title(&self) -> String {
        "RustWeather".into()
    }

    fn activate(&mut self, x: i32, y: i32) {
        let _ = self.events.send(TrayEvent::Clicked(x, y));
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        let _ = self.events.send(TrayEvent::MiddleClicked);
    }

    fn scroll(&mut self, delta: i32, _orientation: ksni::Orientation) {
        let _ = self.events.send(TrayEvent::Scrolled(delta));
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        [22, 44, 64].map(|size| pixmap(&self.data.look, size)).into()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.data.title.clone(),
            description: self.data.description.clone(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let places: Vec<MenuItem<Self>> = self
            .data
            .places
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let mark = if i == self.data.active_place { "● " } else { "" };
                self.entry(&format!("{mark}{name}"), TrayEvent::SelectPlace(i))
            })
            .collect();
        vec![
            self.entry("Open RustWeather", TrayEvent::OpenWindow),
            self.entry("Radar", TrayEvent::OpenRadar),
            self.entry("Weather card", TrayEvent::OpenCard),
            self.entry("Update now", TrayEvent::FetchNow),
            MenuItem::Separator,
            SubMenu {
                label: "Places".into(),
                submenu: places,
                ..Default::default()
            }
            .into(),
            self.entry(
                if self.data.paused {
                    "Resume notifications"
                } else {
                    "Pause notifications"
                },
                TrayEvent::TogglePause,
            ),
            MenuItem::Separator,
            self.entry("Settings", TrayEvent::OpenSettings),
            self.entry("Open log", TrayEvent::OpenLog),
            self.entry("Quit", TrayEvent::Quit),
        ]
    }
}

pub fn start(events: Sender<TrayEvent>, updates: Receiver<TrayData>) {
    let run = move || {
        let tray = AppTray {
            data: TrayData::default(),
            events: events.clone(),
        };
        let handle = match tray.assume_sni_available(true).spawn() {
            Ok(handle) => handle,
            Err(e) => {
                let _ = events.send(TrayEvent::Failed(e.to_string()));
                return;
            }
        };
        while let Ok(mut data) = updates.recv() {
            while let Ok(newer) = updates.try_recv() {
                data = newer;
            }
            handle.update(move |tray: &mut AppTray| tray.data = data);
        }
    };
    if let Err(e) = std::thread::Builder::new().name("tray".into()).spawn(run) {
        tracing::error!(error = %e, "[Tray] No tray thread");
    }
}
