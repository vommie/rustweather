//! The text of the tray tooltip. Plasma shows simple HTML in the description.

use chrono::{DateTime, Local, Utc};

use crate::model::{Alert, Current, Forecast, RainOutlook, rain_word};
use crate::settings::{TempUnit, Tray, Units};

pub fn temperature(celsius: f32, unit: TempUnit) -> String {
    match unit {
        TempUnit::Celsius => format!("{celsius:.0} °C"),
        TempUnit::Fahrenheit => format!("{:.0} °F", celsius * 9.0 / 5.0 + 32.0),
    }
}

fn age(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    match (now - at).num_minutes().max(0) {
        0 => "just now".into(),
        m if m < 60 => format!("{m} min ago"),
        m => format!("{} h {} min ago", m / 60, m % 60),
    }
}

pub fn rain_line(outlook: RainOutlook) -> Option<String> {
    match outlook {
        RainOutlook::Coming {
            in_minutes,
            for_minutes,
            peak_mmh,
        } => {
            let length = for_minutes.map_or(String::new(), |m| format!(", about {m} min long"));
            Some(format!("Rain in {in_minutes} min, {}{length}", rain_word(peak_mmh)))
        }
        RainOutlook::Raining {
            ends_in_minutes: Some(m),
            peak_mmh,
        } => Some(format!("Raining, {}, ends in about {m} min", rain_word(peak_mmh))),
        RainOutlook::Raining {
            ends_in_minutes: None,
            peak_mmh,
        } => Some(format!("Raining, {}", rain_word(peak_mmh))),
        RainOutlook::Dry | RainOutlook::Unknown => None,
    }
}

pub struct Input<'a> {
    pub place: &'a str,
    pub current: Option<&'a Current>,
    pub forecast: Option<&'a Forecast>,
    pub alerts: &'a [Alert],
    pub outlook: RainOutlook,
    pub error: Option<&'a str>,
    pub next_fetch: Option<DateTime<Local>>,
}

/// Title and description. An error goes first; the temperature always stays.
pub fn build(input: &Input<'_>, tray: &Tray, units: &Units) -> (String, String) {
    let now = Utc::now();
    let temp = input.current.and_then(|c| c.temperature);
    // The forecast's words say more than the station's ("Overcast" instead of "Dry").
    let condition = match input.forecast {
        Some(f) => crate::model::condition(f.code),
        None => input.current.map_or("", |c| c.condition.as_str()),
    };
    let title = match temp {
        Some(t) => format!("{} · {condition} · {}", temperature(t, units.temperature), input.place),
        None => input.place.to_owned(),
    };
    let mut lines = Vec::new();
    if let Some(error) = input.error {
        lines.push(format!("<font color='#da4453'>{error}</font>"));
    }
    if tray.line_alerts {
        for alert in input.alerts.iter().take(3) {
            lines.push(format!("<font color='#fdbc4b'>{}</font>", alert.headline));
        }
    }
    if tray.line_rain
        && let Some(rain) = rain_line(input.outlook)
    {
        lines.push(format!("<font color='#3daee9'>{rain}</font>"));
    }
    let mut facts = Vec::new();
    if let (true, Some(f)) = (tray.line_feels_like, input.forecast.and_then(|f| f.feels_like)) {
        facts.push(format!("feels like {}", temperature(f, units.temperature)));
    }
    if let (true, Some(day)) = (tray.line_min_max, input.forecast.and_then(Forecast::today)) {
        let (lo, hi) = (
            temperature(day.min, units.temperature),
            temperature(day.max, units.temperature),
        );
        facts.push(format!("today {lo} to {hi}"));
    }
    if !facts.is_empty() {
        lines.push(facts.join(" · "));
    }
    if tray.line_wind
        && let Some(c) = input.current
        && let Some(wind) = c.wind_kmh
    {
        let gust = c.gust_kmh.map_or(String::new(), |g| format!(", gusts {g:.0}"));
        lines.push(format!("Wind {wind:.0} km/h{gust}"));
    }
    if tray.line_humidity
        && let Some(c) = input.current
    {
        let parts: Vec<String> = [
            c.humidity.map(|h| format!("Humidity {h:.0} %")),
            c.pressure.map(|p| format!("{p:.0} hPa")),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !parts.is_empty() {
            lines.push(parts.join(" · "));
        }
    }
    if tray.line_sun
        && let Some(day) = input.forecast.and_then(Forecast::today)
    {
        let local = Local::now();
        match (day.sunrise, day.sunset) {
            (Some(rise), _) if local < rise => lines.push(format!("Sunrise {}", rise.format("%H:%M"))),
            (_, Some(set)) if local < set => lines.push(format!("Sunset {}", set.format("%H:%M"))),
            _ => {}
        }
    }
    let mut status = Vec::new();
    if let (true, Some(at)) = (tray.line_age, input.current.and_then(|c| c.measured_at)) {
        status.push(format!("measured {}", age(at, now)));
    }
    if let (true, Some(next)) = (tray.line_next, input.next_fetch) {
        status.push(format!("next update {}", next.format("%H:%M")));
    }
    if !status.is_empty() {
        lines.push(format!("<font color='#a1a9b1'>{}</font>", status.join(" · ")));
    }
    (title, lines.join("<br>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rain_sentence() {
        let line = rain_line(RainOutlook::Coming {
            in_minutes: 40,
            for_minutes: Some(25),
            peak_mmh: 0.5,
        });
        assert_eq!(line.as_deref(), Some("Rain in 40 min, light, about 25 min long"));
        assert_eq!(rain_line(RainOutlook::Dry), None);
    }

    #[test]
    fn an_error_keeps_the_temperature() {
        let current = Current {
            temperature: Some(9.2),
            condition: "Cloudy".into(),
            ..Current::default()
        };
        let input = Input {
            place: "Home",
            current: Some(&current),
            forecast: None,
            alerts: &[],
            outlook: RainOutlook::Unknown,
            error: Some("Offline"),
            next_fetch: None,
        };
        let (title, text) = build(&input, &Tray::default(), &Units::default());
        assert!(title.starts_with("9 °C"));
        assert!(text.contains("Offline"));
    }
}
