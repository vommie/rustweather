//! What the Forecast page keeps for a place: measured hours, what had been announced for them, how far forecasts spread, and the usual
//! temperatures of the season. Nothing older than a week stays, so the file stays small and the disk tidy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::model::Forecast;
use crate::settings::Place;
use crate::store;

/// History reaches this far back, for all data of the app.
pub const KEEP: TimeDelta = TimeDelta::days(7);
/// Measured values arrive once an hour and do not change, so this is often enough.
const HISTORY_EVERY: TimeDelta = TimeDelta::hours(3);
/// A forecast is only worth comparing when it was made a good while ahead.
const COMPARE_AHEAD: TimeDelta = TimeDelta::hours(12);

/// One measured hour. Stations do not report everything.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Measured {
    pub temperature: Option<f32>,
    pub rain: Option<f32>,
    pub wind: Option<f32>,
    pub gust: Option<f32>,
    pub humidity: Option<f32>,
    pub cloud: Option<f32>,
    pub pressure: Option<f32>,
}

/// How warm the days around this date usually got in the past years.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Typical {
    pub low: f32,
    pub high: f32,
    pub made: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Archive {
    /// By the hour's Unix time.
    pub measured: BTreeMap<i64, Measured>,
    /// The temperature announced for an hour, at least half a day before.
    pub announced: BTreeMap<i64, f32>,
    /// Lowest and highest of the likely temperatures of an hour.
    pub spread: BTreeMap<i64, (f32, f32)>,
    pub typical: Option<Typical>,
    pub history_at: Option<DateTime<Utc>>,
    pub spread_at: Option<DateTime<Utc>>,
}

/// The file of a place.
pub fn file(cache_dir: &Path, place: &Place) -> PathBuf {
    cache_dir.join(format!("archive-{:.2}-{:.2}.json", place.lat, place.lon))
}

impl Archive {
    pub fn load(file: &Path) -> Self {
        std::fs::read(file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, file: &Path) {
        if let Ok(bytes) = serde_json::to_vec(self)
            && let Err(error) = store::write_atomic(file, &bytes)
        {
            tracing::warn!(%error, "[Archive] Could not save");
        }
    }

    /// Drops all that is older than a week.
    pub fn prune(&mut self, now: DateTime<Utc>) {
        let oldest = (now - KEEP).timestamp();
        self.measured.retain(|hour, _| *hour >= oldest);
        self.announced.retain(|hour, _| *hour >= oldest);
        self.spread.retain(|hour, _| *hour >= oldest);
        if self.typical.is_some_and(|t| now - t.made >= KEEP) {
            self.typical = None;
        }
    }

    /// True when a look at the measured values could bring something new.
    pub fn wants_history(&self, now: DateTime<Utc>) -> bool {
        self.history_at.is_none_or(|t| now - t >= HISTORY_EVERY)
    }

    /// Where the next look at the measured values has to start: after what is there, at most a week back.
    pub fn history_from(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        let oldest = now - KEEP;
        let complete = self
            .measured
            .keys()
            .next()
            .is_some_and(|first| *first <= oldest.timestamp() + 3600);
        let newest = self
            .measured
            .keys()
            .next_back()
            .and_then(|t| DateTime::from_timestamp(*t, 0));
        match newest {
            Some(newest) if complete => newest,
            _ => oldest,
        }
    }

    /// Keeps what a forecast says about hours still a good while ahead, so it can be compared once they have passed.
    pub fn note_forecast(&mut self, forecast: &Forecast, now: DateTime<Utc>) {
        for hour in &forecast.hours {
            if let Some(time) = hour.time.map(|t| t.with_timezone(&Utc))
                && time >= now + COMPARE_AHEAD
            {
                self.announced.insert(time.timestamp(), hour.temperature);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Hour;

    fn at(hours_ago: i64) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH + TimeDelta::days(1000) - TimeDelta::hours(hours_ago)
    }

    #[test]
    fn nothing_stays_longer_than_a_week() {
        let now = at(0);
        let mut archive = Archive::default();
        for key in [at(24 * 8).timestamp(), at(24 * 6).timestamp()] {
            archive.measured.insert(key, Measured::default());
            archive.announced.insert(key, 1.0);
            archive.spread.insert(key, (1.0, 2.0));
        }
        archive.typical = Some(Typical {
            low: 1.0,
            high: 2.0,
            made: at(24 * 8),
        });
        archive.prune(now);
        assert_eq!(
            (archive.measured.len(), archive.announced.len(), archive.spread.len()),
            (1, 1, 1)
        );
        assert!(archive.typical.is_none());
    }

    #[test]
    fn history_starts_where_it_ended_once_the_week_is_complete() {
        let now = at(0);
        let mut archive = Archive::default();
        assert_eq!(archive.history_from(now), now - KEEP);
        archive.measured.insert(at(24 * 3).timestamp(), Measured::default());
        assert_eq!(
            archive.history_from(now),
            now - KEEP,
            "the start of the week is missing"
        );
        archive.measured.insert((now - KEEP).timestamp(), Measured::default());
        assert_eq!(archive.history_from(now), at(24 * 3));
    }

    #[test]
    fn history_is_asked_for_every_few_hours_only() {
        let now = at(0);
        let mut archive = Archive::default();
        assert!(archive.wants_history(now));
        archive.history_at = Some(now - TimeDelta::hours(1));
        assert!(!archive.wants_history(now));
        archive.history_at = Some(now - TimeDelta::hours(4));
        assert!(archive.wants_history(now));
    }

    #[test]
    fn only_forecasts_made_well_ahead_are_kept_for_comparing() {
        let now = at(0);
        let hour = |ahead: i64, temperature: f32| Hour {
            time: Some((now + TimeDelta::hours(ahead)).with_timezone(&chrono::Local)),
            temperature,
            ..Hour::default()
        };
        let forecast = Forecast {
            hours: vec![hour(2, 5.0), hour(24, 6.0)],
            ..Forecast::default()
        };
        let mut archive = Archive::default();
        archive.note_forecast(&forecast, now);
        assert_eq!(archive.announced.values().copied().collect::<Vec<_>>(), [6.0]);
    }

    #[test]
    fn a_broken_file_is_an_empty_archive() {
        assert_eq!(Archive::load(Path::new("/nonexistent/file")), Archive::default());
    }
}
