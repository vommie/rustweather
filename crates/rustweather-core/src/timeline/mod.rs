//! The Forecast page's time band: one hour after the other from a week ago to the end of the forecast, measured where it is known. Turns them
//! into lines, areas and numbers for any stretch and width, so the page only has to draw.

use std::collections::BTreeMap;

use chrono::{DateTime, Local, NaiveDate, Timelike as _, Utc};

use crate::archive::{Archive, KEEP};
use crate::color::{self, Rgb};
use crate::model::Forecast;
use crate::settings::Units;
use crate::tooltip;

mod draw;

pub use draw::{Drawing, Input, Label, Shape, Stretch, Warning, draw, overview};

/// From here on a forecast is only a rough direction.
pub const CERTAIN_HOURS: i64 = 7 * 24;
const DRY_HOURS: usize = 3;
const DRY_CHANCE: f32 = 30.0;
const WET_MM: f32 = 0.1;

#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub time: DateTime<Local>,
    pub measured: bool,
    pub night: bool,
    pub temperature: Option<f32>,
    pub feels: Option<f32>,
    pub rain: f32,
    pub chance: Option<f32>,
    pub snow: f32,
    pub wind: Option<f32>,
    pub gust: Option<f32>,
    pub humidity: Option<f32>,
    pub cloud: Option<f32>,
    pub pressure: Option<f32>,
    pub uv: Option<f32>,
    pub code: Option<u8>,
    pub announced: Option<f32>,
    pub spread: Option<(f32, f32)>,
}

/// Everything that can be shown or hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Filter {
    Temperature,
    Feels,
    Rain,
    Chance,
    Snow,
    Wind,
    Gust,
    Humidity,
    Cloud,
    Uv,
    Pressure,
    Announced,
    Spread,
    Typical,
    Night,
    Dry,
    Warnings,
}

impl Filter {
    pub const ALL: [Self; 17] = [
        Self::Temperature,
        Self::Feels,
        Self::Rain,
        Self::Chance,
        Self::Snow,
        Self::Wind,
        Self::Gust,
        Self::Humidity,
        Self::Cloud,
        Self::Uv,
        Self::Pressure,
        Self::Announced,
        Self::Spread,
        Self::Typical,
        Self::Night,
        Self::Dry,
        Self::Warnings,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Temperature => "temperature",
            Self::Feels => "feels",
            Self::Rain => "rain",
            Self::Chance => "chance",
            Self::Snow => "snow",
            Self::Wind => "wind",
            Self::Gust => "gust",
            Self::Humidity => "humidity",
            Self::Cloud => "cloud",
            Self::Uv => "uv",
            Self::Pressure => "pressure",
            Self::Announced => "announced",
            Self::Spread => "spread",
            Self::Typical => "typical",
            Self::Night => "night",
            Self::Dry => "dry",
            Self::Warnings => "warnings",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Temperature => "Temperature",
            Self::Feels => "Feels like",
            Self::Rain => "Rain",
            Self::Chance => "Chance of rain",
            Self::Snow => "Snow",
            Self::Wind => "Wind",
            Self::Gust => "Gusts",
            Self::Humidity => "Humidity",
            Self::Cloud => "Clouds",
            Self::Uv => "UV",
            Self::Pressure => "Pressure",
            Self::Announced => "Announced",
            Self::Spread => "Uncertainty",
            Self::Typical => "Usual range",
            Self::Night => "Night",
            Self::Dry => "Dry spells",
            Self::Warnings => "Warnings",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Self::Temperature => "Air temperature: measured in the past, forecast ahead",
            Self::Feels => "How warm it feels with wind and humidity",
            Self::Rain => "Rain per hour in millimeters",
            Self::Chance => "How likely rain is in this hour, as a pale area",
            Self::Snow => "Fresh snow per hour in centimeters",
            Self::Wind => "Mean wind speed",
            Self::Gust => "The strongest gusts",
            Self::Humidity => "Relative humidity of the air",
            Self::Cloud => "How much of the sky is covered",
            Self::Uv => "Strength of the sun: from 3 on, skin needs protection",
            Self::Pressure => "Air pressure: rising often means better weather",
            Self::Announced => "What the forecast said a day before, next to what was measured",
            Self::Spread => "Where the temperature will likely be: the wider the area, the less certain",
            Self::Typical => "How warm it usually gets here at this time of the year",
            Self::Night => "Dark bands from sunset to sunrise",
            Self::Dry => "Green bars: at least three hours in a row stay dry",
            Self::Warnings => "Official warnings over the time they hold for",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.key() == key)
    }

    /// What is on the first time the page opens.
    pub fn starts_on(self) -> bool {
        !matches!(self, Self::Cloud | Self::Uv | Self::Pressure | Self::Announced)
    }

    /// Its color on the theme's background.
    pub fn color(self, dark: bool) -> Rgb {
        let (on_dark, on_light) = match self {
            Self::Temperature | Self::Spread => (0xfab387, 0xfe640b),
            Self::Feels | Self::Uv | Self::Warnings => (0xf9e2af, 0xdf8e1d),
            Self::Rain | Self::Announced => (0x89b4fa, 0x1e66f5),
            Self::Chance => (0x74c7ec, 0x209fb5),
            Self::Snow => (0xcba6f7, 0x8839ef),
            Self::Wind | Self::Typical | Self::Dry => (0xa6e3a1, 0x40a02b),
            Self::Gust => (0x94e2d5, 0x179299),
            Self::Humidity => (0x89dceb, 0x04a5e5),
            Self::Cloud => (0x9399b2, 0x7c7f93),
            Self::Pressure => (0xf5c2e7, 0xea76cb),
            Self::Night => (0x585b70, 0xbcc0cc),
        };
        let value: u32 = if dark { on_dark } else { on_light };
        Rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }
}

/// Every hour from a week ago on: what was measured where it is known, the forecast for the rest.
pub fn slots(forecast: Option<&Forecast>, archive: &Archive, now: DateTime<Utc>) -> Vec<Slot> {
    let hours: BTreeMap<i64, _> = forecast
        .map(|f| f.hours.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|h| Some((h.time?.timestamp(), h)))
        .collect();
    let sun: Vec<_> = forecast
        .map(|f| f.days.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|d| Some((d.sunrise?.date_naive(), d.sunrise?, d.sunset?)))
        .collect();
    let hour = |t: i64| t - t.rem_euclid(3600);
    let first = hour((now - KEEP).timestamp());
    let last = hours
        .keys()
        .next_back()
        .copied()
        .unwrap_or_else(|| hour(now.timestamp()));
    (0..)
        .map(|k| first + k * 3600)
        .take_while(|t| *t <= last)
        .filter_map(|t| {
            let time = DateTime::from_timestamp(t, 0)?.with_timezone(&Local);
            let (planned, seen) = (hours.get(&t).copied(), archive.measured.get(&t));
            let measured = seen.is_some_and(|m| m.temperature.is_some());
            let or = |a: Option<f32>, b: Option<f32>| a.or(b);
            let night = sun
                .iter()
                .find(|s| s.0 == time.date_naive())
                .map_or(time.hour() < 7 || time.hour() >= 18, |s| time < s.1 || time >= s.2);
            Some(Slot {
                time,
                measured,
                night,
                temperature: or(seen.and_then(|m| m.temperature), planned.map(|h| h.temperature)),
                feels: planned.map(|h| h.feels),
                rain: or(seen.and_then(|m| m.rain), planned.map(|h| h.precipitation)).unwrap_or_default(),
                chance: if measured {
                    None
                } else {
                    planned.and_then(|h| h.probability)
                },
                snow: planned.map_or(0.0, |h| h.snow),
                wind: or(seen.and_then(|m| m.wind), planned.map(|h| h.wind)),
                gust: or(seen.and_then(|m| m.gust), planned.map(|h| h.gust)),
                humidity: or(seen.and_then(|m| m.humidity), planned.map(|h| h.humidity)),
                cloud: or(seen.and_then(|m| m.cloud), planned.map(|h| h.cloud)),
                pressure: or(seen.and_then(|m| m.pressure), planned.map(|h| h.pressure)),
                uv: planned.map(|h| h.uv),
                code: planned.map(|h| h.code),
                announced: archive.announced.get(&t).copied(),
                spread: archive.spread.get(&t).copied(),
            })
        })
        .collect()
}

/// Stretches ahead of `from` that stay dry for a few hours in a row, as slot numbers.
pub fn dry_spells(slots: &[Slot], from: f64) -> Vec<(usize, usize)> {
    let dry = |s: &Slot| s.rain + s.snow < WET_MM && s.chance.is_some_and(|c| c < DRY_CHANCE);
    let (mut out, mut start) = (Vec::new(), None);
    for (i, slot) in slots.iter().enumerate().skip(from.max(0.0).floor() as usize) {
        match (dry(slot), start) {
            (true, None) => start = Some(i),
            (false, Some(a)) => {
                if i - a >= DRY_HOURS {
                    out.push((a, i - 1));
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(a) = start.filter(|a| slots.len() - a >= DRY_HOURS) {
        out.push((a, slots.len() - 1));
    }
    out
}

/// The slot of a moment, as a fraction.
pub fn slot_of(slots: &[Slot], time: DateTime<Utc>) -> f64 {
    slots
        .first()
        .map_or(0.0, |s| (time - s.time.with_timezone(&Utc)).num_minutes() as f64 / 60.0)
}

/// One day of the band.
#[derive(Debug, Clone, PartialEq)]
pub struct DaySummary {
    pub date: NaiveDate,
    pub first: usize,
    pub low: f32,
    pub high: f32,
    pub rain: f32,
    pub chance: Option<f32>,
    pub gust: Option<f32>,
    pub sunshine: Option<f32>,
    pub code: Option<u8>,
    pub past: bool,
    pub today: bool,
    /// The temperature of each hour of the day.
    pub curve: Vec<f32>,
}

pub fn days(slots: &[Slot], forecast: Option<&Forecast>, today: NaiveDate) -> Vec<DaySummary> {
    let mut out: Vec<DaySummary> = Vec::new();
    for (i, slot) in slots.iter().enumerate() {
        let date = slot.time.date_naive();
        if out.last().is_none_or(|d| d.date != date) {
            let day = forecast.and_then(|f| f.days.iter().find(|d| d.date == Some(date)));
            out.push(DaySummary {
                date,
                first: i,
                low: f32::MAX,
                high: f32::MIN,
                rain: 0.0,
                chance: day.and_then(|d| d.probability),
                gust: None,
                sunshine: day.map(|d| d.sunshine_hours),
                code: day.map(|d| d.code),
                past: date < today,
                today: date == today,
                curve: Vec::new(),
            });
        }
        if let Some(day) = out.last_mut() {
            if let Some(t) = slot.temperature {
                day.low = day.low.min(t);
                day.high = day.high.max(t);
                day.curve.push(t);
            }
            day.rain += slot.rain;
            day.gust = match (day.gust, slot.gust) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
        }
    }
    out.retain(|d| !d.curve.is_empty());
    out
}

/// The handful of things people look for first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    pub next_rain: Option<usize>,
    pub next_frost: Option<usize>,
    pub warmest: Option<usize>,
    pub coldest: Option<usize>,
}

/// Looks at the next seven days of `days` for warmest and coldest.
pub fn facts(slots: &[Slot], days: &[DaySummary], today: NaiveDate, now: f64) -> Facts {
    let ahead: Vec<(usize, &DaySummary)> = days
        .iter()
        .enumerate()
        .filter(|(_, d)| d.date >= today)
        .take(7)
        .collect();
    let by = |f: fn(&DaySummary) -> f32| ahead.iter().max_by(|a, b| f(a.1).total_cmp(&f(b.1))).map(|(i, _)| *i);
    let from = now.max(0.0).ceil() as usize;
    let find = |test: fn(&Slot) -> bool| {
        slots
            .iter()
            .enumerate()
            .skip(from)
            .find(|(_, s)| test(s))
            .map(|(i, _)| i)
    };
    Facts {
        next_rain: find(|s| s.rain >= WET_MM),
        next_frost: find(|s| s.temperature.is_some_and(|t| t < 0.0)),
        warmest: by(|d| d.high),
        coldest: by(|d| -d.low),
    }
}

/// How warm a value looks in numbers of the day views.
pub fn tint(celsius: f32) -> Rgb {
    color::tile(celsius)
}

/// The floating window for one hour: its title and lines.
pub fn tip(slot: &Slot, units: &Units) -> (String, Vec<String>) {
    let temperature = |v: f32| tooltip::temperature(v, units.temperature);
    let source = if slot.measured { "measured" } else { "forecast" };
    let mut lines = Vec::new();
    if let Some(t) = slot.temperature {
        let felt = slot
            .feels
            .map_or(String::new(), |f| format!(", feels like {}", temperature(f)));
        lines.push(format!("{}{felt}", temperature(t)));
    }
    if let Some((low, high)) = slot.spread.filter(|_| !slot.measured) {
        lines.push(format!("Likely {} to {}", temperature(low), temperature(high)));
    }
    if let Some(announced) = slot.announced.filter(|_| slot.measured) {
        lines.push(format!("Announced: {}", temperature(announced)));
    }
    if let Some(code) = slot.code {
        lines.push(crate::model::condition(code).to_owned());
    }
    let chance = slot.chance.map_or(String::new(), |c| format!(", chance {c:.0} %"));
    lines.push(format!("Rain {:.1} mm{chance}", slot.rain));
    if slot.snow > 0.0 {
        lines.push(format!("Snow {:.1} cm", slot.snow));
    }
    if let Some(wind) = slot.wind {
        let gust = slot
            .gust
            .map_or(String::new(), |g| format!(", gusts {}", tooltip::wind(g, units.wind)));
        lines.push(format!("Wind {}{gust}", tooltip::wind(wind, units.wind)));
    }
    if let Some(humidity) = slot.humidity {
        lines.push(format!("Humidity {humidity:.0} %"));
    }
    if let Some(cloud) = slot.cloud {
        lines.push(format!("Clouds {cloud:.0} %"));
    }
    (format!("{} ({source})", slot.time.format("%a %-d, %H:%M")), lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use chrono::TimeDelta;

    use crate::archive::Measured;
    use crate::model::{Day, Hour};
    use crate::settings::Units;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-31T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn forecast() -> Forecast {
        let hours = (-48..24 * 16)
            .map(|k: i64| Hour {
                time: Some((now() + TimeDelta::hours(k)).with_timezone(&Local)),
                temperature: 10.0 + (k as f32 / 6.0).sin(),
                feels: 8.0,
                precipitation: if k == 5 { 1.0 } else { 0.0 },
                probability: Some(if k == 5 { 80.0 } else { 5.0 }),
                wind: 12.0,
                gust: 25.0,
                humidity: 70.0,
                ..Hour::default()
            })
            .collect();
        Forecast {
            hours,
            ..Forecast::default()
        }
    }

    fn input<'a>(slots: &'a [Slot], on: &'a HashSet<Filter>, units: &'a Units) -> Input<'a> {
        let middle = slot_of(slots, now());
        Input {
            slots,
            stretch: Stretch {
                start: middle - 24.0,
                span: 96.0,
            },
            width: 1000.0,
            on,
            warnings: &[],
            typical: None,
            units,
            now: now(),
            dark: true,
        }
    }

    fn all() -> HashSet<Filter> {
        Filter::ALL.into_iter().collect()
    }

    #[test]
    fn the_band_runs_from_a_week_ago_to_the_end_of_the_forecast() {
        let slots = slots(Some(&forecast()), &Archive::default(), now());
        assert_eq!(slots.first().map(|s| s.time.with_timezone(&Utc)), Some(now() - KEEP));
        assert_eq!(slots.len(), (7 + 16) * 24);
        assert!(slots[0].temperature.is_none(), "nothing known that far back yet");
    }

    #[test]
    fn measured_beats_forecast_and_is_marked() {
        let mut archive = Archive::default();
        let hour = (now() - TimeDelta::hours(5)).timestamp();
        archive.measured.insert(
            hour,
            Measured {
                temperature: Some(3.0),
                ..Measured::default()
            },
        );
        let slots = slots(Some(&forecast()), &archive, now());
        let slot = slots.iter().find(|s| s.time.timestamp() == hour).unwrap();
        assert!(slot.measured);
        assert_eq!(slot.temperature, Some(3.0));
        assert_eq!(slot.chance, None);
    }

    #[test]
    fn a_new_hidden_filter_takes_its_lane_away() {
        let slots = slots(Some(&forecast()), &Archive::default(), now());
        let units = Units::default();
        let full = all();
        let mut fewer = all();
        fewer.remove(&Filter::Humidity);
        let tall = draw(&input(&slots, &full, &units)).height;
        let short = draw(&input(&slots, &fewer, &units)).height;
        assert!(short < tall);
    }

    #[test]
    fn the_past_and_the_future_are_named_on_the_band() {
        let slots = slots(Some(&forecast()), &Archive::default(), now());
        let units = Units::default();
        let on = all();
        let mut far = input(&slots, &on, &units);
        far.stretch.span = 400.0;
        let drawing = draw(&far);
        for name in ["Measured", "Forecast", "Only a rough direction from here"] {
            assert!(drawing.labels.iter().any(|l| l.text == name), "{name}");
        }
    }

    #[test]
    fn numbers_stay_apart_at_every_zoom() {
        let slots = slots(Some(&forecast()), &Archive::default(), now());
        let units = Units::default();
        let on = HashSet::from([Filter::Temperature]);
        for span in [48.0, 96.0, 168.0, 384.0, 550.0] {
            let mut stretch = input(&slots, &on, &units);
            stretch.stretch.start = (slot_of(&slots, now()) - span / 2.0).max(0.0);
            stretch.stretch.span = span;
            let drawing = draw(&stretch);
            let mut xs: Vec<f32> = drawing
                .labels
                .iter()
                .filter(|l| l.text.contains('°'))
                .map(|l| l.x)
                .collect();
            xs.sort_by(f32::total_cmp);
            assert!(xs.windows(2).all(|p| p[1] - p[0] >= 30.0), "span {span}: {xs:?}");
            assert!(!xs.is_empty(), "span {span}");
        }
    }

    #[test]
    fn dry_spells_need_some_hours_and_a_known_chance() {
        let slots = slots(Some(&forecast()), &Archive::default(), now());
        let start = slot_of(&slots, now());
        let spells = dry_spells(&slots, start);
        assert_eq!(spells[0].1, start as usize + 4, "the rain ends the first spell");
        assert!(spells.iter().all(|(a, b)| b - a + 1 >= DRY_HOURS));
    }

    #[test]
    fn days_add_up_and_know_their_place() {
        let mut data = forecast();
        let today = now().with_timezone(&Local).date_naive();
        data.days.push(Day {
            date: Some(today),
            sunshine_hours: 3.0,
            probability: Some(40.0),
            ..Day::default()
        });
        let slots = slots(Some(&data), &Archive::default(), now());
        let days = days(&slots, Some(&data), today);
        let day = days.iter().find(|d| d.today).unwrap();
        assert_eq!((day.sunshine, day.chance), (Some(3.0), Some(40.0)));
        assert!(day.high >= day.low);
        assert!(days.iter().any(|d| d.past));
    }

    #[test]
    fn facts_point_at_the_next_rain() {
        let slots = slots(Some(&forecast()), &Archive::default(), now());
        let today = now().with_timezone(&Local).date_naive();
        let days = days(&slots, None, today);
        let start = slot_of(&slots, now());
        let facts = facts(&slots, &days, today, start);
        assert_eq!(facts.next_rain, Some(start as usize + 5));
        assert_eq!(facts.next_frost, None);
        assert!(facts.warmest.is_some() && facts.coldest.is_some());
    }

    #[test]
    fn every_filter_finds_itself_by_its_key() {
        for filter in Filter::ALL {
            assert_eq!(Filter::from_key(filter.key()), Some(filter));
            assert!(!filter.name().is_empty() && !filter.tip().is_empty());
        }
    }
}
