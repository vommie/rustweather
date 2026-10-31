//! The weather loop. In the background it only keeps the temperature for the tray up to date; everything else is fetched while the app is
//! in use. Behind a locked screen, a screen saver or a fullscreen program nothing is fetched at all.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use parking_lot::Mutex;
use rustweather_core::archive::{self, Archive, Measured, Typical};
use rustweather_core::error::Error;
use rustweather_core::http::Http;
use rustweather_core::model::{Alert, Current, Forecast, RainOutlook};
use rustweather_core::radar::outlook;
use rustweather_core::settings::{Place, Settings};
use rustweather_core::sources::{self, brightsky, openmeteo};
use serde_json::json;
use tokio::sync::{Notify, watch};

const TYPICAL_YEARS: i64 = 10;
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
    /// When the long forecast was fetched: the page shows it, and it decides when the next is due.
    pub forecast_at: Option<DateTime<Utc>>,
    pub forecast_next: Option<DateTime<Utc>>,
    pub archive: Archive,
    pub health: Vec<Health>,
}

#[derive(Clone)]
pub struct Control {
    pub now: Arc<Notify>,
    /// True while the Forecast page is on screen: only then the past is looked at.
    pub forecast_open: Arc<watch::Sender<bool>>,
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
        forecast_at: serde_json::from_value(value.get("forecast_at")?.clone()).ok(),
        ..Snapshot::default()
    })
}

/// Brings what the Forecast page shows of the past up to date, each part only when it is due. True when something changed.
async fn look_back(
    http: &Http,
    place: &Place,
    state: &Mutex<Snapshot>,
    forecast_every: TimeDelta,
    typical_tried: &mut Option<Instant>,
) -> bool {
    let now = Utc::now();
    let (history_from, spread_due, typical_due) = {
        let s = state.lock();
        (
            s.archive.wants_history(now).then(|| s.archive.history_from(now)),
            s.archive.spread_at.is_none_or(|t| now - t >= forecast_every),
            s.archive.typical.is_none() && typical_tried.is_none_or(|t| t.elapsed() >= Duration::from_secs(3600)),
        )
    };
    let mut changed = false;
    if let Some(from) = history_from {
        let result = brightsky::history(http, place, from, now).await;
        let mut s = state.lock();
        s.archive.history_at = Some(now);
        match result {
            Ok(hours) => {
                for (time, measured) in hours.into_iter().filter(|h| h.0 <= now) {
                    s.archive.measured.insert(time.timestamp(), measured);
                }
            }
            Err(error) => tracing::warn!(%error, "[Weather] Measured values failed"),
        }
        changed = true;
    }
    if spread_due {
        let result = openmeteo::spread(http, place).await;
        let mut s = state.lock();
        s.archive.spread_at = Some(now);
        match result {
            Ok(hours) => {
                s.archive.spread = hours
                    .into_iter()
                    .map(|(t, low, high)| (t.timestamp(), (low, high)))
                    .collect();
            }
            Err(error) => tracing::warn!(%error, "[Weather] The spread of the forecast failed"),
        }
        changed = true;
    }
    if typical_due {
        *typical_tried = Some(Instant::now());
        if let Some(typical) = usual_range(http, place, now).await {
            state.lock().archive.typical = Some(typical);
            changed = true;
        }
    }
    changed
}

/// How warm the two weeks around this date got in the last ten years. Ten requests, and then not again for a week.
async fn usual_range(http: &Http, place: &Place, now: DateTime<Utc>) -> Option<Typical> {
    let (mut lows, mut highs) = (Vec::new(), Vec::new());
    for year in 1..=TYPICAL_YEARS {
        let middle = now - TimeDelta::days(year * 365);
        let (from, to) = (middle - TimeDelta::days(7), middle + TimeDelta::days(7));
        match brightsky::history(http, place, from, to).await {
            Ok(hours) => {
                let (low, high) = daily_means(&hours)?;
                lows.push(low);
                highs.push(high);
            }
            Err(error) => tracing::debug!(%error, year, "[Weather] A year of the usual range failed"),
        }
    }
    (lows.len() >= 3).then(|| Typical {
        low: lows.iter().sum::<f32>() / lows.len() as f32,
        high: highs.iter().sum::<f32>() / highs.len() as f32,
        made: now,
    })
}

/// The mean of each day's lowest and highest temperature.
fn daily_means(hours: &[(DateTime<Utc>, Measured)]) -> Option<(f32, f32)> {
    let mut days: BTreeMap<_, (f32, f32)> = BTreeMap::new();
    for (time, measured) in hours {
        if let Some(t) = measured.temperature {
            let day = days.entry(time.date_naive()).or_insert((t, t));
            *day = (day.0.min(t), day.1.max(t));
        }
    }
    let count = days.len() as f32;
    (count > 0.0).then(|| {
        (
            days.values().map(|d| d.0).sum::<f32>() / count,
            days.values().map(|d| d.1).sum::<f32>() / count,
        )
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
    let (forecast_open, mut page) = watch::channel(false);
    let control = Control {
        now: Arc::clone(&now),
        forecast_open: Arc::new(forecast_open),
    };
    let state = Arc::new(Mutex::new(start));
    runtime.spawn(async move {
        let mut last: Option<Instant> = None;
        let mut details: Option<Instant> = None;
        let mut warnings: Option<Instant> = None;
        let mut archive_file = None;
        let mut typical_tried: Option<Instant> = None;
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
                changed = page.changed() => { if changed.is_err() { return; } false }
            };
            let page_open = *page.borrow_and_update();
            let mode = *presence.borrow_and_update();
            let current_settings = settings.borrow().clone();
            // Dragging a slider in Settings must not fetch anything; another place must.
            let place_changed = last_place.as_ref().is_some_and(|p| *p != current_settings.place());
            let manual = manual || place_changed;
            last_place = Some(current_settings.place());
            let forecast_every = TimeDelta::minutes(i64::from(current_settings.general.forecast_minutes.max(1)));
            let file = archive::file(&cache_dir, &current_settings.place());
            if archive_file.as_ref() != Some(&file) {
                state.lock().archive = Archive::load(&file);
                archive_file = Some(file.clone());
            }
            if page_open && mode == Presence::Active {
                let changed = look_back(&http, &current_settings.place(), &state, forecast_every, &mut typical_tried).await;
                if changed {
                    let snapshot = {
                        let mut s = state.lock();
                        s.archive.prune(Utc::now());
                        s.archive.save(&file);
                        s.clone()
                    };
                    publish(snapshot);
                }
            }
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
            let long_due = state.lock().forecast_at.is_none_or(|t| Utc::now() - t >= forecast_every);
            if full || current_settings.alerts.background_warnings {
                alerts = timed(&mut health, "Warnings (DWD)", brightsky::alerts(&http, &place)).await;
            }
            if full {
                if manual || long_due {
                    forecast = timed(&mut health, "Forecast (Open-Meteo)", openmeteo::forecast(&http, &place)).await;
                }
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
                    s.archive.note_forecast(&value, Utc::now());
                    s.forecast = Some(value);
                    s.forecast_at = Some(Utc::now());
                }
                s.forecast_next = s.forecast_at.map(|t| t + forecast_every);
                if let Some(value) = alerts {
                    s.alerts = value;
                    warnings = Some(Instant::now());
                }
                if let Some(value) = series {
                    s.outlook = outlook::read(&value, Utc::now());
                    s.series = value;
                }
                if !full && warnings.is_none_or(|t| t.elapsed() >= DETAILS_MAX_AGE) {
                    s.alerts.clear();
                }
                if !full && details.is_none_or(|t| t.elapsed() >= DETAILS_MAX_AGE) {
                    s.outlook = RainOutlook::default();
                    s.series.clear();
                }
                s.error = error;
                s.health.retain(|old| health.iter().all(|new| new.name != old.name));
                s.health.extend(health);
                s.next_at = chrono::TimeDelta::from_std(every).ok().map(|d| Utc::now() + d);
                s.clone()
            };
            if let Some(file) = &archive_file {
                snapshot.archive.save(file);
            }
            let saved = json!({ "current": snapshot.current, "forecast": snapshot.forecast, "forecast_at": snapshot.forecast_at });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hour(day: i64, h: i64, temperature: f32) -> (DateTime<Utc>, Measured) {
        let time = DateTime::UNIX_EPOCH + TimeDelta::days(day) + TimeDelta::hours(h);
        let measured = Measured {
            temperature: Some(temperature),
            ..Measured::default()
        };
        (time, measured)
    }

    #[test]
    fn the_usual_range_is_the_mean_of_each_days_extremes() {
        let hours = [hour(0, 3, 2.0), hour(0, 14, 10.0), hour(1, 3, 4.0), hour(1, 14, 14.0)];
        assert_eq!(daily_means(&hours), Some((3.0, 12.0)));
        assert_eq!(daily_means(&[]), None);
    }
}
