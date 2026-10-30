//! The Network page: what the app is sending right now, per server and request by request.

use std::rc::Rc;
use std::time::Duration;

use rustweather_core::traffic::{self, Entry, HostStats};
use slint::{ComponentHandle as _, ModelRc, VecModel};

use crate::app::App;
use crate::{TrafficEntry, TrafficHost};

const REFRESH: Duration = Duration::from_millis(500);
/// The Network page's place in the settings.
const PAGE: i32 = 11;
/// Rows on the page; more would only slow it down.
const ROWS: usize = 60;

fn size(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{:.0} kB", bytes as f64 / 1e3),
        _ => format!("{:.1} MB", bytes as f64 / 1e6),
    }
}

fn host_row(stats: &HostStats) -> TrafficHost {
    let answered = stats.requests.max(1);
    TrafficHost {
        provider: traffic::provider(&stats.host).into(),
        host: stats.host.as_str().into(),
        running: stats.running as i32,
        minute: stats.last_minute.to_string().into(),
        today: stats.today.to_string().into(),
        limit: traffic::limit(&stats.host).into(),
        session: stats.requests.to_string().into(),
        failed: stats.failed as i32,
        data: size(stats.bytes).into(),
        time: if stats.requests == 0 {
            String::new().into()
        } else {
            format!("{} ms", stats.millis / answered).into()
        },
    }
}

fn entry_row(entry: &Entry) -> TrafficEntry {
    TrafficEntry {
        time: entry.at.format("%H:%M:%S").to_string().into(),
        provider: traffic::provider(&entry.host).into(),
        what: entry.what.as_str().into(),
        outcome: entry.outcome.as_str().into(),
        failed: entry.failed,
        time_taken: format!("{} ms", entry.millis).into(),
        size: size(entry.bytes).into(),
    }
}

fn show(app: &App) {
    let snapshot = app.http.traffic().snapshot();
    let window = &app.settings_window;
    let sum = |f: fn(&HostStats) -> u64| snapshot.hosts.iter().map(f).sum::<u64>();
    window.set_traffic_summary(
        format!(
            "{} running · {} in the last minute · {} since the start · {} · {} today",
            sum(|h| h.running),
            sum(|h| h.last_minute),
            sum(|h| h.requests),
            size(sum(|h| h.bytes)),
            sum(|h| h.today),
        )
        .into(),
    );
    let hosts: Vec<TrafficHost> = snapshot.hosts.iter().map(host_row).collect();
    window.set_traffic_hosts(ModelRc::new(VecModel::from(hosts)));
    if !window.get_traffic_paused() {
        let rows: Vec<TrafficEntry> = snapshot.recent.iter().take(ROWS).map(entry_row).collect();
        window.set_traffic_entries(ModelRc::new(VecModel::from(rows)));
    }
}

/// Tells the main window which services do not answer as they should.
pub fn show_problems(app: &App) {
    let mut lines: Vec<String> = app
        .http
        .traffic()
        .troubles()
        .iter()
        .map(|(host, outcome)| {
            let what = match outcome.as_str() {
                "429" => "limit reached".to_owned(),
                "timeout" => "not answering".to_owned(),
                "offline" => "no connection".to_owned(),
                "key" => "key rejected".to_owned(),
                other => format!("error {other}"),
            };
            format!("{}: {what}", traffic::provider(host))
        })
        .collect();
    lines.sort();
    lines.dedup();
    app.main.set_net_problem(lines.join(" · ").into());
}

pub fn wire(app: &Rc<App>) {
    let weak = Rc::downgrade(app);
    app.main.on_open_network(move || {
        if let Some(app) = weak.upgrade() {
            app.open_settings(Some(PAGE));
        }
    });
    let weak = Rc::downgrade(app);
    app.settings_window.on_traffic_reset(move || {
        if let Some(app) = weak.upgrade() {
            app.http.traffic().reset();
            show(&app);
        }
    });
    let weak = Rc::downgrade(app);
    app.traffic_timer.start(slint::TimerMode::Repeated, REFRESH, move || {
        if let Some(app) = weak.upgrade()
            && app.settings_window.window().is_visible()
            && app.settings_window.get_page() == PAGE
        {
            show(&app);
        }
    });
}
