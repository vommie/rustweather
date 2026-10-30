//! What the app sends over the network, for the Network page: a running list and statistics per server. Only the path and the name of a
//! map layer are kept: a query string can carry an API key.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::store;

/// How many requests the list keeps.
const RECENT: usize = 200;
const SAVE_EVERY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct Entry {
    pub at: DateTime<Local>,
    pub host: String,
    pub what: String,
    /// "200", "429", "timeout", …
    pub outcome: String,
    pub failed: bool,
    pub millis: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct HostStats {
    pub host: String,
    pub requests: u64,
    pub failed: u64,
    pub bytes: u64,
    pub millis: u64,
    pub running: u64,
    pub last_minute: u64,
    pub today: u64,
    pub today_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Most requests first.
    pub hosts: Vec<HostStats>,
    /// Newest first.
    pub recent: Vec<Entry>,
}

#[derive(Default)]
struct Counters {
    requests: u64,
    failed: u64,
    bytes: u64,
    millis: u64,
    running: u64,
    minute: VecDeque<Instant>,
}

/// The day's counts survive a restart, because that is how the services count.
#[derive(Serialize, Deserialize, Default)]
struct Today {
    day: Option<NaiveDate>,
    hosts: BTreeMap<String, (u64, u64)>,
}

#[derive(Default)]
struct Inner {
    hosts: BTreeMap<String, Counters>,
    recent: VecDeque<Entry>,
    today: Today,
    file: Option<PathBuf>,
    saved: Option<Instant>,
}

impl Inner {
    fn roll_day(&mut self) {
        let day = Local::now().date_naive();
        if self.today.day != Some(day) {
            self.today = Today {
                day: Some(day),
                hosts: BTreeMap::new(),
            };
        }
    }
}

/// Shared by all clones of the HTTP client.
#[derive(Clone, Default)]
pub struct Traffic {
    inner: Arc<Mutex<Inner>>,
}

/// A request on its way. When it is dropped unfinished, it no longer counts as running.
pub struct Flight {
    traffic: Traffic,
    host: String,
    started: Instant,
}

impl Drop for Flight {
    fn drop(&mut self) {
        if let Some(counters) = self.traffic.inner.lock().hosts.get_mut(&self.host) {
            counters.running = counters.running.saturating_sub(1);
        }
    }
}

/// What a URL asks for, without anything that could be secret.
fn describe(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let (place, query) = rest.split_once('?').unwrap_or((rest, ""));
    let path = place.find('/').map_or("/", |at| &place[at..]);
    let layer = query.split('&').find_map(|pair| pair.strip_prefix("layers="));
    match layer {
        Some(layer) => format!("{path} {layer}"),
        None => path.to_owned(),
    }
}

fn outcome_of<T>(result: &Result<T>) -> String {
    match result {
        Ok(_) => "200".to_owned(),
        Err(Error::Http { status, .. }) => status.to_string(),
        Err(Error::RateLimited { .. }) => "429".to_owned(),
        Err(Error::BadKey { .. }) => "key".to_owned(),
        Err(Error::Timeout { .. }) => "timeout".to_owned(),
        Err(Error::Offline { .. }) => "offline".to_owned(),
        Err(_) => "failed".to_owned(),
    }
}

impl Traffic {
    /// From now on the day's counts live in this file.
    pub fn persist_to(&self, file: PathBuf) {
        let today = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Today>(&bytes).ok())
            .unwrap_or_default();
        let mut inner = self.inner.lock();
        inner.today = today;
        inner.file = Some(file);
    }

    pub fn begin(&self, host: &str) -> Flight {
        self.inner.lock().hosts.entry(host.to_owned()).or_default().running += 1;
        Flight {
            traffic: self.clone(),
            host: host.to_owned(),
            started: Instant::now(),
        }
    }

    pub fn finish<T>(&self, flight: Flight, url: &str, result: &Result<T>, bytes: u64) {
        let millis = flight.started.elapsed().as_millis() as u64;
        let entry = Entry {
            at: Local::now(),
            host: flight.host.clone(),
            what: describe(url),
            outcome: outcome_of(result),
            failed: result.is_err(),
            millis,
            bytes,
        };
        tracing::debug!(host = %entry.host, what = %entry.what, outcome = %entry.outcome, millis, "[Http] Request");
        let save = {
            let mut inner = self.inner.lock();
            inner.roll_day();
            let today = inner.today.hosts.entry(entry.host.clone()).or_default();
            today.0 += 1;
            today.1 += bytes;
            let counters = inner.hosts.entry(entry.host.clone()).or_default();
            counters.requests += 1;
            counters.failed += u64::from(entry.failed);
            counters.bytes += bytes;
            counters.millis += millis;
            counters.minute.push_back(flight.started);
            inner.recent.push_front(entry);
            inner.recent.truncate(RECENT);
            let due = inner.saved.is_none_or(|t| t.elapsed() >= SAVE_EVERY);
            if due {
                inner.saved = Some(Instant::now());
            }
            inner
                .file
                .clone()
                .filter(|_| due)
                .and_then(|file| Some((file, serde_json::to_vec(&inner.today).ok()?)))
        };
        if let Some((file, json)) = save
            && let Err(error) = store::write_atomic(&file, &json)
        {
            tracing::debug!(%error, "[Traffic] Could not save the day's counts");
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut inner = self.inner.lock();
        inner.roll_day();
        let Inner {
            hosts, recent, today, ..
        } = &mut *inner;
        let mut list: Vec<HostStats> = today
            .hosts
            .keys()
            .filter(|host| !hosts.contains_key(*host))
            .map(|host| HostStats {
                host: host.clone(),
                ..HostStats::default()
            })
            .collect();
        for (host, counters) in hosts.iter_mut() {
            while counters
                .minute
                .front()
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(60))
            {
                counters.minute.pop_front();
            }
            list.push(HostStats {
                host: host.clone(),
                requests: counters.requests,
                failed: counters.failed,
                bytes: counters.bytes,
                millis: counters.millis,
                running: counters.running,
                last_minute: counters.minute.len() as u64,
                ..HostStats::default()
            });
        }
        for stats in &mut list {
            (stats.today, stats.today_bytes) = today.hosts.get(&stats.host).copied().unwrap_or_default();
        }
        list.sort_by_key(|s| (std::cmp::Reverse(s.today), s.host.clone()));
        Snapshot {
            hosts: list,
            recent: recent.iter().cloned().collect(),
        }
    }

    /// Starts the list and the session's statistics over; the day's counts stay, the services go on counting too.
    pub fn reset(&self) {
        let mut inner = self.inner.lock();
        inner.recent.clear();
        for counters in inner.hosts.values_mut() {
            let running = counters.running;
            *counters = Counters {
                running,
                ..Counters::default()
            };
        }
    }
}

/// Who runs a server, by its name.
pub fn provider(host: &str) -> &'static str {
    const NAMES: [(&str, &str); 11] = [
        ("open-meteo.com", "Open-Meteo"),
        ("brightsky.dev", "Bright Sky"),
        ("dwd.de", "DWD"),
        ("rainviewer.com", "RainViewer"),
        ("cartocdn.com", "CARTO"),
        ("arcgisonline.com", "Esri"),
        ("eox.at", "EOX"),
        ("opentopomap.org", "OpenTopoMap"),
        ("openstreetmap.org", "OpenStreetMap"),
        ("geodatenzentrum.de", "BKG"),
        ("wetterdienst.de", "wetterdienst.de"),
    ];
    NAMES
        .iter()
        .find(|(domain, _)| host == *domain || host.ends_with(&format!(".{domain}")))
        .map_or("Other", |(_, name)| name)
}

/// The free limit a service states.
pub fn limit(host: &str) -> &'static str {
    match provider(host) {
        "Open-Meteo" => "10,000 a day",
        "RainViewer" => "100 a minute",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(traffic: &Traffic, url: &str, host: &str, result: &Result<()>, bytes: u64) {
        traffic.finish(traffic.begin(host), url, result, bytes);
    }

    #[test]
    fn a_key_in_the_query_never_reaches_the_list() {
        let traffic = Traffic::default();
        done(
            &traffic,
            "https://basemaps.cartocdn.com/rastertiles/dark_all/1/2/3.png?key=SECRET",
            "basemaps.cartocdn.com",
            &Ok(()),
            10,
        );
        let snapshot = traffic.snapshot();
        assert_eq!(snapshot.recent[0].what, "/rastertiles/dark_all/1/2/3.png");
    }

    #[test]
    fn a_map_layer_is_named() {
        let url = "https://maps.dwd.de/geoserver/dwd/wms?service=WMS&layers=dwd:Warnungen_Landkreise&bbox=1,2,3,4";
        assert_eq!(describe(url), "/geoserver/dwd/wms dwd:Warnungen_Landkreise");
    }

    #[test]
    fn statistics_add_up_per_server() {
        let traffic = Traffic::default();
        done(&traffic, "https://a.example/x", "a.example", &Ok(()), 100);
        done(
            &traffic,
            "https://a.example/y",
            "a.example",
            &Err(Error::Timeout {
                host: "a.example".into(),
            }),
            0,
        );
        let stats = traffic.snapshot().hosts.remove(0);
        assert_eq!((stats.requests, stats.failed, stats.bytes), (2, 1, 100));
        assert_eq!((stats.today, stats.last_minute, stats.running), (2, 2, 0));
        assert_eq!(traffic.snapshot().recent[0].outcome, "timeout");
    }

    #[test]
    fn a_running_request_shows_and_ends_even_when_dropped() {
        let traffic = Traffic::default();
        let flight = traffic.begin("a.example");
        assert_eq!(traffic.snapshot().hosts[0].running, 1);
        drop(flight);
        assert_eq!(traffic.snapshot().hosts[0].running, 0);
    }

    #[test]
    fn reset_keeps_the_day() {
        let traffic = Traffic::default();
        done(&traffic, "https://a.example/x", "a.example", &Ok(()), 100);
        traffic.reset();
        let snapshot = traffic.snapshot();
        assert!(snapshot.recent.is_empty());
        assert_eq!((snapshot.hosts[0].requests, snapshot.hosts[0].today), (0, 1));
    }

    #[test]
    fn the_day_survives_a_restart() {
        let file = std::env::temp_dir().join(format!("rustweather-traffic-{}.json", std::process::id()));
        let traffic = Traffic::default();
        traffic.persist_to(file.clone());
        done(&traffic, "https://a.example/x", "a.example", &Ok(()), 100);
        let again = Traffic::default();
        again.persist_to(file.clone());
        assert_eq!(again.snapshot().hosts[0].today, 1);
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn servers_get_their_provider() {
        assert_eq!(provider("api.open-meteo.com"), "Open-Meteo");
        assert_eq!(provider("tile.openstreetmap.org"), "OpenStreetMap");
        assert_eq!(provider("evil-dwd.de.example"), "Other");
        assert_eq!(limit("api.rainviewer.com"), "100 a minute");
    }
}
