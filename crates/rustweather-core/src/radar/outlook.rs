//! Will it rain here? Asks only for the few grid cells around the place, so this runs with the weather while the app sits in
//! the tray, without ever loading the full radar.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::Result;
use crate::http::Http;
use crate::model::RainOutlook;
use crate::settings::Place;

#[derive(Deserialize)]
struct Answer {
    radar: Vec<Frame>,
}

#[derive(Deserialize)]
struct Frame {
    timestamp: DateTime<Utc>,
    /// Rows of values, 0.01 mm per 5 minutes.
    precipitation_5: Vec<Vec<u16>>,
}

/// mm/h counted as rain.
const WET: f32 = 0.15;

/// The rain rate at the place per frame, strongest cell of the small window, and its time.
pub async fn series(http: &Http, place: &Place) -> Result<Vec<(DateTime<Utc>, f32)>> {
    let url = format!(
        "https://api.brightsky.dev/radar?lat={}&lon={}&distance=2000&format=plain",
        place.lat, place.lon
    );
    let answer: Answer = http.json(&url).await?;
    Ok(answer
        .radar
        .iter()
        .map(|f| {
            let peak = f.precipitation_5.iter().flatten().copied().max().unwrap_or(0);
            (f.timestamp, f32::from(peak) * 0.12)
        })
        .collect())
}

/// Reads the series: frames at or after `now` decide.
pub fn read(series: &[(DateTime<Utc>, f32)], now: DateTime<Utc>) -> RainOutlook {
    let ahead: Vec<(u32, f32)> = series
        .iter()
        .filter(|(t, _)| *t + chrono::TimeDelta::minutes(5) > now)
        .map(|(t, r)| (u32::try_from((*t - now).num_minutes().max(0)).unwrap_or(0), *r))
        .collect();
    let Some(first) = ahead.first() else {
        return RainOutlook::Unknown;
    };
    if first.1 >= WET {
        let end = ahead.iter().find(|(_, r)| *r < WET).map(|(m, _)| *m);
        let peak = ahead
            .iter()
            .take_while(|(_, r)| *r >= WET)
            .map(|(_, r)| *r)
            .fold(0.0, f32::max);
        return RainOutlook::Raining {
            ends_in_minutes: end,
            peak_mmh: peak,
        };
    }
    let Some(start) = ahead.iter().position(|(_, r)| *r >= WET) else {
        return RainOutlook::Dry;
    };
    let wet = ahead.get(start..).unwrap_or_default();
    let in_minutes = wet.first().map_or(0, |w| w.0);
    let end = wet.iter().find(|(_, r)| *r < WET).map(|(m, _)| *m);
    let peak = wet
        .iter()
        .take_while(|(_, r)| *r >= WET)
        .map(|(_, r)| *r)
        .fold(0.0, f32::max);
    RainOutlook::Coming {
        in_minutes,
        for_minutes: end.map(|m| m - in_minutes),
        peak_mmh: peak,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    fn series(rates: &[f32], now: DateTime<Utc>) -> Vec<(DateTime<Utc>, f32)> {
        rates
            .iter()
            .enumerate()
            .map(|(i, r)| (now + TimeDelta::minutes(5 * i as i64), *r))
            .collect()
    }

    #[test]
    fn rain_is_coming_and_ends() {
        let now = Utc::now();
        let s = series(&[0.0, 0.0, 0.0, 0.5, 2.0, 0.4, 0.0, 0.0], now);
        assert_eq!(
            read(&s, now),
            RainOutlook::Coming {
                in_minutes: 15,
                for_minutes: Some(15),
                peak_mmh: 2.0
            }
        );
    }

    #[test]
    fn raining_now() {
        let now = Utc::now();
        assert_eq!(
            read(&series(&[1.0, 1.0, 0.0], now), now),
            RainOutlook::Raining {
                ends_in_minutes: Some(10),
                peak_mmh: 1.0
            }
        );
    }

    #[test]
    fn dry_and_unknown() {
        let now = Utc::now();
        assert_eq!(read(&series(&[0.0, 0.0], now), now), RainOutlook::Dry);
        assert_eq!(read(&[], now), RainOutlook::Unknown);
    }
}
