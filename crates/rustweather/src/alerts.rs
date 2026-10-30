//! Desktop notifications: decides what is news, keeps quiet when it should, and never says the same thing twice.

use std::collections::HashSet;

use chrono::{Local, TimeDelta, Timelike as _};
use rustweather_core::model::{Alert, Forecast, RainOutlook, rain_word};
use rustweather_core::settings::Alerts;

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: String,
    pub title: String,
    pub body: String,
    pub severe: bool,
}

/// Pure decision: what would be worth saying now.
pub fn candidates(
    settings: &Alerts,
    alerts: &[Alert],
    forecast: Option<&Forecast>,
    outlook: RainOutlook,
    was_raining: bool,
) -> Vec<Message> {
    let mut out = Vec::new();
    let today = Local::now().date_naive();
    if settings.dwd {
        for alert in alerts.iter().filter(|a| a.level >= settings.min_level) {
            out.push(Message {
                id: format!("dwd-{}", alert.id),
                title: alert.headline.clone(),
                body: alert.description.clone(),
                severe: alert.level >= 3,
            });
        }
    }
    match outlook {
        RainOutlook::Coming {
            in_minutes,
            for_minutes,
            peak_mmh,
        } if settings.rain && in_minutes <= settings.lead_minutes => {
            let length = for_minutes.map_or(String::new(), |m| format!(", about {m} min long"));
            out.push(Message {
                id: format!("rain-{}-{}", today, Local::now().hour() / 2),
                title: format!("Rain in {in_minutes} min"),
                body: format!("{}{length}", rain_word(peak_mmh)),
                severe: false,
            });
        }
        RainOutlook::Raining {
            ends_in_minutes: Some(m),
            ..
        } if settings.rain_end && m <= 15 && was_raining => {
            out.push(Message {
                id: format!("dry-{}-{}", today, Local::now().hour() / 2),
                title: format!("Rain ends in about {m} min"),
                body: String::new(),
                severe: false,
            });
        }
        _ => {}
    }
    if let Some(f) = forecast {
        let horizon = Local::now() + TimeDelta::hours(3);
        let soon = f
            .hours
            .iter()
            .filter(|h| h.time.is_some_and(|t| t >= Local::now() && t <= horizon));
        if settings.thunderstorm && soon.clone().any(|h| h.code >= 95) {
            out.push(Message {
                id: format!("storm-{today}"),
                title: "Thunderstorms in the next hours".into(),
                body: String::new(),
                severe: false,
            });
        }
        if let Some(day) = f.today() {
            if settings.frost && day.min < 0.0 {
                out.push(Message {
                    id: format!("frost-{today}"),
                    title: "Frost tonight".into(),
                    body: format!("Down to {:.0} °C", day.min),
                    severe: false,
                });
            }
            if settings.heat && day.max >= 30.0 {
                out.push(Message {
                    id: format!("heat-{today}"),
                    title: "Hot today".into(),
                    body: format!("Up to {:.0} °C", day.max),
                    severe: false,
                });
            }
        }
    }
    out
}

fn quiet(settings: &Alerts, hour: u32) -> bool {
    let (from, to) = (settings.quiet_from_hour, settings.quiet_to_hour);
    settings.quiet_at_night
        && if from > to {
            hour >= from || hour < to
        } else {
            hour >= from && hour < to
        }
}

#[derive(Default)]
pub struct Notifier {
    said: HashSet<String>,
    was_raining: bool,
}

impl Notifier {
    pub fn update(&mut self, settings: &Alerts, alerts: &[Alert], forecast: Option<&Forecast>, outlook: RainOutlook) {
        let raining = matches!(outlook, RainOutlook::Raining { .. });
        let messages = candidates(settings, alerts, forecast, outlook, self.was_raining);
        self.was_raining = raining;
        if settings.paused {
            return;
        }
        let hushed =
            quiet(settings, Local::now().hour()) || (settings.quiet_when_fullscreen && crate::presence::fullscreen());
        for message in messages {
            if self.said.contains(&message.id) || (hushed && !(message.severe && settings.severe_ignores_quiet)) {
                continue;
            }
            let shown = notify_rust::Notification::new()
                .appname("RustWeather")
                .icon("rustweather")
                .hint(notify_rust::Hint::DesktopEntry(rustweather_core::APP_ID.to_owned()))
                .summary(&message.title)
                .body(&message.body)
                .show();
            if let Err(e) = shown {
                tracing::warn!(error = %e, "[Alerts] Could not show a notification");
            }
            self.said.insert(message.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn night_wraps_around_midnight() {
        let s = Alerts::default();
        assert!(quiet(&s, 23) && quiet(&s, 3) && !quiet(&s, 12));
    }

    #[test]
    fn warning_level_filters() {
        let alert = Alert {
            id: "a".into(),
            level: 1,
            ..Alert::default()
        };
        assert!(
            candidates(
                &Alerts::default(),
                std::slice::from_ref(&alert),
                None,
                RainOutlook::Dry,
                false
            )
            .is_empty()
        );
        let strong = Alert { level: 3, ..alert };
        let found = candidates(&Alerts::default(), &[strong], None, RainOutlook::Dry, false);
        assert!(found.len() == 1 && found[0].severe);
    }

    #[test]
    fn rain_only_within_the_lead_time() {
        let coming = |m| RainOutlook::Coming {
            in_minutes: m,
            for_minutes: None,
            peak_mmh: 1.0,
        };
        assert!(candidates(&Alerts::default(), &[], None, coming(45), false).is_empty());
        assert_eq!(candidates(&Alerts::default(), &[], None, coming(15), false).len(), 1);
    }
}
