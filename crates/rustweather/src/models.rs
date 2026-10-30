//! Turns weather data into what the windows show.

use chrono::{DateTime, Datelike as _, Local, Timelike as _, Utc};
use rustweather_core::model::{self, Forecast};
use rustweather_core::settings::{Settings, TempUnit, WindUnit};
use rustweather_core::tooltip::{self, rain_line, temperature as temp_text};
use rustweather_core::tray_icon::Look;
use rustweather_core::{color, color::Rgb};
use slint::{Color, SharedString};

use crate::tray::TrayData;
use crate::weather::Snapshot;
use crate::{AlertData, DayData, HourData, NowData, RainBar};

const STALE_MINUTES: i64 = 60;

fn slint_color(c: Rgb) -> Color {
    Color::from_rgb_u8(c.0, c.1, c.2)
}

fn s(text: impl Into<SharedString>) -> SharedString {
    text.into()
}

fn opt(value: Option<f32>, format: impl Fn(f32) -> String) -> SharedString {
    value.map_or_else(|| s("–"), |v| s(format(v)))
}

fn wind(kmh: f32, unit: WindUnit) -> String {
    match unit {
        WindUnit::Kmh => format!("{kmh:.0} km/h"),
        WindUnit::Ms => format!("{:.1} m/s", kmh / 3.6),
        WindUnit::Beaufort => format!("{:.0} Bft", ((kmh / 3.6) / 0.836).powf(2.0 / 3.0).min(12.0)),
    }
}

fn compass(degrees: f32) -> &'static str {
    const NAMES: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    NAMES[((degrees.rem_euclid(360.0) + 22.5) / 45.0) as usize % 8]
}

/// The moon's phase from its mean cycle, measured from a known new moon.
pub fn moon_phase(now: DateTime<Utc>) -> &'static str {
    const CYCLE: f64 = 29.530_588;
    let known = DateTime::from_timestamp(947_182_440, 0).unwrap_or(now); // 2000-01-06 18:14 UTC
    let days = (now - known).num_seconds() as f64 / 86_400.0;
    match (days.rem_euclid(CYCLE) / CYCLE * 8.0).round() as u32 % 8 {
        0 => "New moon",
        1 => "Waxing crescent",
        2 => "First quarter",
        3 => "Waxing gibbous",
        4 => "Full moon",
        5 => "Waning gibbous",
        6 => "Last quarter",
        _ => "Waning crescent",
    }
}

fn air_word(aqi: f32) -> &'static str {
    match aqi {
        x if x <= 20.0 => "good",
        x if x <= 40.0 => "fair",
        x if x <= 60.0 => "moderate",
        x if x <= 80.0 => "poor",
        _ => "very poor",
    }
}

fn history(forecast: &Forecast, unit: TempUnit) -> (String, String) {
    let now = Local::now();
    let past: Vec<f32> = forecast
        .hours
        .iter()
        .filter(|h| h.time.is_some_and(|t| t <= now && (now - t).num_hours() < 24))
        .map(|h| h.temperature)
        .collect();
    if past.len() < 2 {
        return (String::new(), String::new());
    }
    let (low, high) = past
        .iter()
        .fold((f32::MAX, f32::MIN), |m, v| (m.0.min(*v), m.1.max(*v)));
    let span = (high - low).max(1.0);
    let step = 1000.0 / (past.len() - 1) as f32;
    let path = past
        .iter()
        .enumerate()
        .map(|(i, v)| {
            format!(
                "{}{:.1} {:.1}",
                if i == 0 { "M" } else { " L" },
                i as f32 * step,
                92.0 - (v - low) / span * 84.0
            )
        })
        .collect();
    (path, format!("{} to {}", temp_text(low, unit), temp_text(high, unit)))
}

pub fn now(snapshot: &Snapshot, settings: &Settings) -> NowData {
    let unit = settings.units.temperature;
    let c = snapshot.current.clone().unwrap_or_default();
    let f = snapshot.forecast.clone().unwrap_or_default();
    let today = f.today().cloned();
    let condition = match snapshot.forecast {
        Some(_) => model::condition(f.code).to_owned(),
        None => c.condition.clone(),
    };
    let (history_path, history_range) = history(&f, unit);
    let local = |t: Option<DateTime<Local>>| t.map_or_else(|| s("–"), |t| s(t.format("%H:%M").to_string()));
    NowData {
        has_data: snapshot.current.is_some(),
        place: s(settings.place().name),
        temperature: opt(c.temperature, |t| temp_text(t, unit)),
        condition: s(condition),
        feels: opt(f.feels_like, |t| temp_text(t, unit)),
        range: today.as_ref().map_or_else(SharedString::default, |d| {
            s(format!("{} to {}", temp_text(d.min, unit), temp_text(d.max, unit)))
        }),
        wind: opt(c.wind_kmh, |w| wind(w, settings.units.wind)),
        gust: match (c.gust_kmh, c.wind_dir) {
            (Some(g), Some(d)) => s(format!("Gusts {}, from {}", wind(g, settings.units.wind), compass(d))),
            (Some(g), None) => s(format!("Gusts {}", wind(g, settings.units.wind))),
            _ => SharedString::default(),
        },
        humidity: opt(c.humidity, |h| format!("{h:.0} %")),
        dew: opt(c.dew_point, |t| temp_text(t, unit)),
        pressure: opt(c.pressure, |p| format!("{p:.0} hPa")),
        visibility: opt(c.visibility_km, |v| format!("{v:.0} km")),
        precipitation: opt(c.precipitation_mm, |p| format!("{p:.1} mm")),
        clouds: c
            .cloud_cover
            .map_or_else(SharedString::default, |v| s(format!("Clouds {v:.0} %"))),
        uv: opt(f.uv, |u| format!("{u:.1}")),
        air: opt(f.air_quality, |a| format!("{a:.0}")),
        air_detail: f.air_quality.map_or_else(SharedString::default, |a| s(air_word(a))),
        pollen: if f.pollen.is_empty() {
            s("None")
        } else {
            s(f.pollen.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", "))
        },
        sunrise: local(today.as_ref().and_then(|d| d.sunrise)),
        sunset: local(today.as_ref().and_then(|d| d.sunset)),
        moon: s(moon_phase(Utc::now())),
        station: if c.station.is_empty() {
            SharedString::default()
        } else {
            s(format!("Station: {} ({})", c.station, c.source))
        },
        measured: c.measured_at.map_or_else(SharedString::default, |t| {
            s(format!("Measured {}", t.with_timezone(&Local).format("%H:%M")))
        }),
        rain_line: s(rain_line(snapshot.outlook).unwrap_or_default()),
        tile: slint_color(c.temperature.map_or(Rgb(0x5c, 0x60, 0x70), color::tile)),
        history_path: s(history_path),
        history_range: s(history_range),
    }
}

/// `count` hours from now on.
pub fn hours(snapshot: &Snapshot, count: usize, step: usize) -> Vec<HourData> {
    let Some(forecast) = &snapshot.forecast else {
        return Vec::new();
    };
    let now = Local::now();
    let ahead: Vec<_> = forecast
        .hours
        .iter()
        .filter(|h| h.time.is_some_and(|t| t >= now))
        .step_by(step.max(1))
        .take(count)
        .collect();
    let (low, high) = ahead.iter().fold((f32::MAX, f32::MIN), |m, h| {
        (m.0.min(h.temperature), m.1.max(h.temperature))
    });
    let span = (high - low).max(1.0);
    let rain_top = ahead.iter().map(|h| h.precipitation).fold(1.0, f32::max);
    ahead
        .iter()
        .map(|h| HourData {
            label: s(h.time.map_or(String::new(), |t| t.hour().to_string())),
            temperature: s(format!("{:.0}°", h.temperature)),
            rain: h.precipitation / rain_top,
            probability: if h.probability >= 10.0 {
                s(format!("{:.0} %", h.probability))
            } else {
                SharedString::default()
            },
            height: 0.15 + (h.temperature - low) / span * 0.75,
        })
        .collect()
}

pub fn days(snapshot: &Snapshot, settings: &Settings) -> Vec<DayData> {
    let Some(forecast) = &snapshot.forecast else {
        return Vec::new();
    };
    let unit = settings.units.temperature;
    let today = Local::now().date_naive();
    let shown: Vec<_> = forecast
        .days
        .iter()
        .filter(|d| d.date.is_some_and(|x| x >= today))
        .collect();
    let low = shown.iter().map(|d| d.min).fold(f32::MAX, f32::min);
    let high = shown.iter().map(|d| d.max).fold(f32::MIN, f32::max);
    let span = (high - low).max(1.0);
    shown
        .iter()
        .map(|d| DayData {
            label: s(d.date.map_or(String::new(), |x| {
                if x == today {
                    "Today".to_owned()
                } else {
                    format!("{} {}.", x.format("%a"), x.day())
                }
            })),
            condition: s(model::condition(d.code)),
            low: s(temp_text(d.min, unit)),
            high: s(temp_text(d.max, unit)),
            rain: if d.precipitation >= 0.1 {
                s(format!("{:.1} mm", d.precipitation))
            } else {
                SharedString::default()
            },
            start: (d.min - low) / span,
            end: (d.max - low) / span,
            color: slint_color(color::temperature((d.min + d.max) / 2.0)),
        })
        .collect()
}

pub fn alerts(snapshot: &Snapshot) -> Vec<AlertData> {
    snapshot
        .alerts
        .iter()
        .map(|a| {
            let time = |t: Option<DateTime<Utc>>| {
                t.map_or(String::new(), |t| {
                    t.with_timezone(&Local).format("%a %H:%M").to_string()
                })
            };
            AlertData {
                title: s(a.headline.clone()),
                text: s(a.description.clone()),
                time: s(format!("{} until {}", time(a.onset), time(a.expires))),
                level: i32::from(a.level),
            }
        })
        .collect()
}

/// The rain curve of the place.
pub fn bars(snapshot: &Snapshot) -> Vec<RainBar> {
    let now = Utc::now();
    let top = snapshot.series.iter().map(|p| p.1).fold(1.0, f32::max);
    snapshot
        .series
        .iter()
        .map(|(t, rate)| RainBar {
            height: rate / top,
            past: *t <= now,
        })
        .collect()
}

pub fn tray(snapshot: &Snapshot, settings: &Settings, background: Option<Rgb>, foreground: Option<Rgb>) -> TrayData {
    let current = snapshot.current.as_ref();
    let temperature = current.and_then(|c| c.temperature);
    let age = current
        .and_then(|c| c.measured_at)
        .map(|t| (Utc::now() - t).num_minutes());
    let stale = settings.tray.show_stale && (snapshot.fetched_at.is_none() || age.is_some_and(|a| a > STALE_MINUTES));
    let rain_soon = matches!(
        snapshot.outlook,
        model::RainOutlook::Coming { in_minutes, .. } if in_minutes <= 60
    ) || matches!(snapshot.outlook, model::RainOutlook::Raining { .. });
    let trend = match (snapshot.forecast.as_ref(), temperature, settings.tray.show_trend) {
        (Some(f), Some(t), true) => f.trend(t),
        _ => model::Trend::Steady,
    };
    let look = Look {
        temperature,
        stale,
        rain_soon: rain_soon && settings.tray.show_rain_dot,
        alert: !snapshot.alerts.is_empty() && settings.tray.show_alert,
        trend,
        background,
        foreground,
    };
    let place = settings.place().name;
    let input = tooltip::Input {
        place: &place,
        current,
        forecast: snapshot.forecast.as_ref(),
        alerts: &snapshot.alerts,
        outlook: snapshot.outlook,
        error: snapshot.error.as_deref(),
        next_fetch: snapshot.next_at.map(|t| t.with_timezone(&Local)),
    };
    let (title, description) = tooltip::build(&input, &settings.tray, &settings.units);
    TrayData {
        look,
        title,
        description,
        places: settings.places.iter().map(|p| p.name.clone()).collect(),
        active_place: settings.active_place,
        paused: settings.alerts.paused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compass_and_moon() {
        assert_eq!(compass(0.0), "N");
        assert_eq!(compass(359.0), "N");
        assert_eq!(compass(225.0), "SW");
        // 2026-10-26 was a full moon (±1 step of the eight phases).
        let t = DateTime::from_timestamp(1_792_987_200, 0).unwrap();
        assert!(["Full moon", "Waxing gibbous", "Waning gibbous"].contains(&moon_phase(t)));
    }

    #[test]
    fn wind_units() {
        assert_eq!(wind(36.0, WindUnit::Kmh), "36 km/h");
        assert_eq!(wind(36.0, WindUnit::Ms), "10.0 m/s");
        assert_eq!(wind(36.0, WindUnit::Beaufort), "5 Bft");
    }
}
