//! The weather loop. In the background it only keeps the temperature for the tray up to date; everything else is fetched while the app is
//! in use. Behind a locked screen, a screen saver or a fullscreen program nothing is fetched at all.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rustweather_core::error::Error;
use rustweather_core::http::Http;
use rustweather_core::model::{Alert, Current, Forecast, RainOutlook};
use rustweather_core::radar::outlook;
use rustweather_core::settings::Settings;
use rustweather_core::sources::{self, brightsky, openmeteo};
use serde_json::json;
use tokio::sync::{Notify, watch};

/// How long rain and warnings stay valid without a new look. After that the tray says nothing about them rather than something old.
const DETAILS_MAX_AGE: Duration = Duration::from_secs(30 * 60);

/// What the app may fetch right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// The screen is locked, a screen saver runs or a fullscreen program has the screen: nothing.
    Quiet,
    /// Only the tray is there: the temperature, nothing else.
    Background,
    /// A window of the app is on screen: everything.
    Active,
}

/// How the last call to one service went, for Settings > Diagnostics.
#[derive(Debug, Clone, Default)]
pub struct Health {
    pub name: String,
    pub ok: Option<bool>,
    pub at: Option<DateTime<Utc>>,
    pub millis: u128,
    pub detail: String,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub current: Option<Current>,
    pub forecast: Option<Forecast>,
    pub alerts: Vec<Alert>,
    pub outlook: RainOutlook,
    pub series: Vec<(DateTime<Utc>, f32)>,
    pub error: Option<String>,
    pub fetched_at: Option<DateTime<Utc>>,
    pub next_at: Option<DateTime<Utc>>,
    pub health: Vec<Health>,
}

#[derive(Clone)]
pub struct Control {
    pub now: Arc<Notify>,
}

async fn timed<T>(health: &mut Vec<Health>, name: &str, call: impl Future<Output = Result<T, Error>>) -> Option<T> {
    let started = Instant::now();
    let result = call.await;
    let mut entry = Health {
        name: name.to_owned(),
        at: Some(Utc::now()),
        millis: started.elapsed().as_millis(),
        ..Health::default()
    };
    let value = match result {
        Ok(value) => {
            entry.ok = Some(true);
            Some(value)
        }
        Err(e) => {
            tracing::warn!(service = name, error = %e, "[Weather] Fetch failed");
            entry.ok = Some(false);
            entry.detail = e.to_string();
            None
        }
    };
    health.push(entry);
    value
}

fn cache_file(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("weather.json")
}

/// The last weather from disk, so the tray has a temperature before the first answer.
pub fn load_cached(dir: &std::path::Path) -> Option<Snapshot> {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(cache_file(dir)).ok()?).ok()?;
    Some(Snapshot {
        current: serde_json::from_value(value.get("current")?.clone()).ok(),
        forecast: serde_json::from_value(value.get("forecast")?.clone()).ok(),
        ..Snapshot::default()
    })
}

pub fn spawn(
    runtime: &tokio::runtime::Handle,
    http: Http,
    cache_dir: std::path::PathBuf,
    settings: watch::Receiver<Settings>,
    presence: watch::Receiver<Presence>,
    start: Snapshot,
    publish: impl Fn(Snapshot) + Send + 'static,
) -> Control {
    let now = Arc::new(Notify::new());
    let control = Control { now: Arc::clone(&now) };
    let state = Arc::new(Mutex::new(start));
    runtime.spawn(async move {
        let mut last: Option<Instant> = None;
        let mut details: Option<Instant> = None;
        let mut failures = 0u32;
        let mut settings = settings;
        let mut presence = presence;
        let mut last_place = None;
        loop {
            let manual = tokio::select! {
                () = now.notified() => true,
                () = tokio::time::sleep(Duration::from_secs(20)) => false,
                changed = settings.changed() => { if changed.is_err() { return; } false }
                changed = presence.changed() => { if changed.is_err() { return; } false }
            };
            let mode = *presence.borrow_and_update();
            let current_settings = settings.borrow().clone();
            // Dragging a slider in Settings must not fetch anything; another place must.
            let place_changed = last_place.as_ref().is_some_and(|p| *p != current_settings.place());
            let manual = manual || place_changed;
            last_place = Some(current_settings.place());
            let every = Duration::from_secs(u64::from(current_settings.general.refresh_minutes.max(1)) * 60);
            // Failed fetches retry soon, but with growing gaps.
            let wait = if failures == 0 {
                every
            } else {
                Duration::from_secs(30 * u64::from(failures.min(10))).min(every)
            };
            // A click on "Update now" or another place is use of the app, even from the tray.
            let full = manual || mode == Presence::Active;
            let due = last.is_none_or(|t| t.elapsed() >= wait) || (full && details.is_none_or(|t| t.elapsed() >= wait));
            if mode == Presence::Quiet || (!manual && !due) {
                continue;
            }
            last = Some(Instant::now());
            let place = current_settings.place();
            let mut health = Vec::new();
            let current = timed(
                &mut health,
                "Weather (DWD)",
                sources::current(&http, &place, &current_settings.general.station_url),
            )
            .await;
            let (mut forecast, mut alerts, mut series) = (None, None, None);
            if full {
                forecast = timed(&mut health, "Forecast (Open-Meteo)", openmeteo::forecast(&http, &place)).await;
                alerts = timed(&mut health, "Warnings (DWD)", brightsky::alerts(&http, &place)).await;
                series = timed(&mut health, "Rain outlook (DWD)", outlook::series(&http, &place)).await;
                details = Some(Instant::now());
            }
            let error = health.iter().find(|h| h.ok == Some(false)).map(|h| h.detail.clone());
            failures = if current.is_some() { 0 } else { failures + 1 };
            let snapshot = {
                let mut s = state.lock();
                if let Some(value) = current {
                    s.current = Some(value);
                    s.fetched_at = Some(Utc::now());
                }
                if let Some(value) = forecast {
                    s.forecast = Some(value);
                }
                if let Some(value) = alerts {
                    s.alerts = value;
                }
                if let Some(value) = series {
                    s.outlook = outlook::read(&value, Utc::now());
                    s.series = value;
                }
                if !full && details.is_none_or(|t| t.elapsed() >= DETAILS_MAX_AGE) {
                    s.alerts.clear();
                    s.outlook = RainOutlook::default();
                    s.series.clear();
                }
                s.error = error;
                if full {
                    s.health = health;
                } else {
                    s.health.retain(|old| health.iter().all(|new| new.name != old.name));
                    s.health.extend(health);
                }
                s.next_at = chrono::TimeDelta::from_std(every).ok().map(|d| Utc::now() + d);
                s.clone()
            };
            let saved = json!({ "current": snapshot.current, "forecast": snapshot.forecast });
            if let Ok(bytes) = serde_json::to_vec(&saved)
                && let Err(e) = rustweather_core::store::write_atomic(&cache_file(&cache_dir), &bytes)
            {
                tracing::warn!(error = %e, "[Weather] Could not cache the weather");
            }
            publish(snapshot);
        }
    });
    control
}
