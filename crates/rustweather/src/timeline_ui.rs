//! The Forecast page: keeps the stretch of the band on screen and turns the core's drawing into what the page shows.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use chrono::{DateTime, Local, Timelike as _, Utc};
use rustweather_core::archive::Typical;
use rustweather_core::color::Rgb;
use rustweather_core::settings::Units;
use rustweather_core::timeline::{self, DaySummary, Filter, Input, Slot, Stretch, Warning};
use rustweather_core::tooltip;
use slint::{ComponentHandle as _, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::{
    DayCard, FactCard, FilterChip, FilterRow, ForecastState, SpanChip, Theme, TimelineLabel, TimelineShape, models,
};

const OVERVIEW_HEIGHT: f32 = 24.0;
const SPANS: [(&str, i32); 3] = [("2 days", 48), ("7 days", 168), ("16 days", 384)];
const MIN_SPAN: f64 = 12.0;
/// The share of the band that lies before now when the page jumps there.
const NOW_AT: f64 = 0.25;
/// Width of a filter button, roughly, to break them into rows.
const CHAR_WIDTH: f32 = 6.2;
const CHIP_EXTRA: f32 = 36.0;
const MIN_TILE: f32 = 128.0;
const GUST_SHOWN: f32 = 30.0;
const SUN_SHOWN: f32 = 0.5;
/// What a tile keeps around its curve.
const TILE_MARGIN: f32 = 16.0;
const CURVE_HEIGHT: f32 = 26.0;

/// What the page has on screen. Lives in the app, survives page changes.
#[derive(Default)]
pub struct Band {
    slots: Vec<Slot>,
    days: Vec<DaySummary>,
    start: Option<f64>,
    span: f64,
    width: f32,
    on: HashSet<Filter>,
    warnings: Vec<Warning>,
    typical: Option<Typical>,
}

impl Band {
    pub fn from_saved(span: f64, on: &[String], grid_known: bool) -> RefCell<Self> {
        let mut chosen: HashSet<Filter> = if on.is_empty() {
            Filter::ALL.into_iter().filter(|f| f.starts_on()).collect()
        } else {
            on.iter().filter_map(|k| Filter::from_key(k)).collect()
        };
        if !grid_known {
            chosen.insert(Filter::Grid);
        }
        RefCell::new(Self {
            span: span.max(MIN_SPAN),
            on: chosen,
            ..Self::default()
        })
    }

    /// Slots before this are empty: the archive is still growing.
    fn first_known(&self) -> f64 {
        self.slots.iter().position(|s| s.temperature.is_some()).unwrap_or(0) as f64
    }

    fn all(&self) -> f64 {
        self.slots.len().max(1) as f64
    }

    fn settle(&mut self) {
        let first = self.first_known();
        let room = (self.all() - 1.0 - first).max(MIN_SPAN);
        self.span = self.span.clamp(MIN_SPAN, room);
        let latest = (self.all() - 1.0 - self.span).max(first);
        self.start = self.start.map(|s| s.clamp(first, latest));
    }

    fn now_slot(&self) -> f64 {
        timeline::slot_of(&self.slots, Utc::now())
    }

    fn go_to_now(&mut self) {
        self.start = Some(self.now_slot() - self.span * NOW_AT);
        self.settle();
    }
}

fn color(rgb: Rgb) -> slint::Color {
    slint::Color::from_rgb_u8(rgb.0, rgb.1, rgb.2)
}

fn tinted(rgb: Rgb, alpha: f32) -> slint::Color {
    color(rgb).with_alpha(alpha)
}

fn draw(app: &App) {
    let state = app.main.global::<ForecastState>();
    let band = app.band.borrow();
    let Some(start) = band.start.filter(|_| band.width > 0.0 && !band.slots.is_empty()) else {
        return;
    };
    let settings = app.settings.borrow();
    let drawing = timeline::draw(&Input {
        slots: &band.slots,
        stretch: Stretch { start, span: band.span },
        width: band.width,
        on: &band.on,
        warnings: &band.warnings,
        typical: band.typical,
        units: &settings.units,
        now: Utc::now(),
        dark: app.main.global::<Theme>().get_dark(),
    });
    let shapes: Vec<TimelineShape> = drawing
        .shapes
        .into_iter()
        .filter(|s| !s.commands.is_empty())
        .map(|s| TimelineShape {
            commands: s.commands.into(),
            stroke: s.stroke.map_or(slint::Color::default(), |(c, a)| tinted(c, a)),
            fill: s.fill.map_or(slint::Color::default(), |(c, a)| tinted(c, a)),
            width: s.width,
        })
        .collect();
    let labels: Vec<TimelineLabel> = drawing
        .labels
        .into_iter()
        .map(|l| TimelineLabel {
            x: l.x,
            y: l.y,
            text: l.text.into(),
            color: l.color.map_or(slint::Color::default(), color),
            plain: l.color.is_none(),
            bold: l.bold,
            right: l.right,
        })
        .collect();
    state.set_shapes(ModelRc::new(VecModel::from(shapes)));
    state.set_labels(ModelRc::new(VecModel::from(labels)));
    state.set_band_height(drawing.height);
    state.set_now_x(drawing.now_x.unwrap_or(-10.0));
    let all = band.all();
    state.set_window_start((start / all) as f32);
    state.set_window_end(((start + band.span) / all) as f32);
    state.set_now_share((band.now_slot() / all) as f32);
    let spans: Vec<SpanChip> = SPANS
        .iter()
        .map(|(name, hours)| (*name, *hours))
        .chain([("All", 0)])
        .map(|(name, hours)| SpanChip {
            name: name.into(),
            hours,
            on: if hours == 0 {
                band.span >= all - 1.0 - band.first_known()
            } else {
                (band.span - f64::from(hours)).abs() < 1.0
            },
        })
        .collect();
    state.set_spans(ModelRc::new(VecModel::from(spans)));
}

fn filter_rows(band: &Band, dark: bool) -> Vec<FilterRow> {
    let (mut rows, mut row, mut used) = (Vec::new(), Vec::new(), 0.0);
    for (index, filter) in Filter::ALL.into_iter().enumerate() {
        let wide = filter.name().len() as f32 * CHAR_WIDTH + CHIP_EXTRA;
        if used + wide > band.width && !row.is_empty() {
            rows.push(FilterRow {
                items: ModelRc::new(VecModel::from(std::mem::take(&mut row))),
            });
            used = 0.0;
        }
        used += wide;
        row.push(FilterChip {
            name: filter.name().into(),
            tip: filter.tip().into(),
            color: color(filter.swatch(dark)),
            on: band.on.contains(&filter),
            index: index as i32,
        });
    }
    if !row.is_empty() {
        rows.push(FilterRow {
            items: ModelRc::new(VecModel::from(row)),
        });
    }
    rows
}

fn show_filters(app: &App) {
    let rows = filter_rows(&app.band.borrow(), app.main.global::<Theme>().get_dark());
    app.main
        .global::<ForecastState>()
        .set_filters(ModelRc::new(VecModel::from(rows)));
}

/// The course of a day over the tile's full width.
fn curve(values: &[f32], width: f32) -> String {
    let (low, high) = values
        .iter()
        .fold((f32::MAX, f32::MIN), |m, v| (m.0.min(*v), m.1.max(*v)));
    let span = (high - low).max(1.0);
    let step = width / (values.len().max(2) - 1) as f32;
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            format!(
                "{}{:.1} {:.1}",
                if i == 0 { "M" } else { "L" },
                i as f32 * step,
                CURVE_HEIGHT - 2.0 - (v - low) / span * (CURVE_HEIGHT - 4.0)
            )
        })
        .collect()
}

fn day_card(day: &DaySummary, low: f32, high: f32, units: &Units, curve_width: f32) -> DayCard {
    let temperature = |v: f32| tooltip::temperature(v, units.temperature);
    let span = (high - low).max(1.0);
    let label = if day.today {
        "Today".to_owned()
    } else {
        day.date.format("%a %-d").to_string()
    };
    let rain = if day.rain >= 0.1 {
        format!("{:.1} mm", day.rain)
    } else {
        "dry".to_owned()
    };
    let chance = day
        .chance
        .filter(|c| *c >= 10.0)
        .map_or(String::new(), |c| format!("{c:.0} % chance"));
    let gust = day
        .gust
        .filter(|g| *g >= GUST_SHOWN)
        .map_or(String::new(), |g| format!("Gusts {}", tooltip::wind(g, units.wind)));
    let sun = day
        .sunshine
        .filter(|s| *s >= SUN_SHOWN)
        .map_or(String::new(), |s| format!("{s:.0} h sun"));
    let wet = (51..=67).contains(&day.code.unwrap_or(0)) || (80..=99).contains(&day.code.unwrap_or(0));
    // The code of a day can speak of showers that hardly reach the ground: the symbol follows the amount beside it.
    let code = match day.code {
        Some(_) if wet && day.rain < 0.1 => 3,
        Some(code) => code,
        None if day.rain >= 0.1 => 61,
        None => 3,
    };
    let tip = [
        format!(
            "{}: {} to {}",
            day.date.format("%A, %B %-d"),
            temperature(day.low),
            temperature(day.high)
        ),
        rain.clone(),
        chance.clone(),
        gust.clone(),
        sun.clone(),
    ]
    .into_iter()
    .filter(|t| !t.is_empty())
    .collect::<Vec<_>>()
    .join(", ");
    DayCard {
        label: label.into(),
        weather: models::weather_kind(code, false),
        high: temperature(day.high).into(),
        low: temperature(day.low).into(),
        high_color: color(timeline::tint(day.high)),
        curve: curve(&day.curve, curve_width).into(),
        rain: rain.into(),
        chance: chance.into(),
        gust: gust.into(),
        sun: sun.into(),
        past: day.past,
        today: day.today,
        slot: day.first as i32,
        start: (day.low - low) / span,
        end: (day.high - low) / span,
        tip: tip.into(),
    }
}

fn when(slot: &Slot) -> String {
    let today = Local::now().date_naive();
    let date = slot.time.date_naive();
    let day = if date == today {
        "today".to_owned()
    } else {
        date.format("%a %-d").to_string()
    };
    format!("{day}, {}:00", slot.time.hour())
}

fn show_summary(app: &App, band: &Band) {
    let state = app.main.global::<ForecastState>();
    let settings = app.settings.borrow();
    let units = &settings.units;
    let today = Local::now().date_naive();
    let facts = timeline::facts(&band.slots, &band.days, today, band.now_slot());
    let temperature = |v: f32| tooltip::temperature(v, units.temperature);
    let name = |d: &DaySummary| {
        if d.today {
            "today".to_owned()
        } else {
            d.date.format("%A").to_string()
        }
    };
    let mut cards = Vec::new();
    let mut sentence = Vec::new();
    if let Some(day) = band.days.iter().find(|d| d.today) {
        sentence.push(format!("Today {} to {}", temperature(day.low), temperature(day.high)));
    }
    if let Some(day) = facts.warmest.and_then(|i| band.days.get(i)) {
        sentence.push(format!("warmest {}", name(day)));
        cards.push(FactCard {
            title: "Warmest day".into(),
            value: format!("{}, {}", name(day), temperature(day.high)).into(),
            slot: day.first as i32,
        });
    }
    if let Some(day) = facts.coldest.and_then(|i| band.days.get(i)) {
        sentence.push(format!("coldest {}", name(day)));
        cards.push(FactCard {
            title: "Coldest day".into(),
            value: format!("{}, {}", name(day), temperature(day.low)).into(),
            slot: day.first as i32,
        });
    }
    match facts.next_rain {
        Some(i) => {
            let slot = &band.slots[i];
            sentence.push(format!("next rain {}", when(slot)));
            cards.insert(
                0,
                FactCard {
                    title: "Next rain".into(),
                    value: when(slot).into(),
                    slot: i as i32,
                },
            );
        }
        None => sentence.push("no rain in sight".to_owned()),
    }
    if let Some(i) = facts.next_frost {
        cards.push(FactCard {
            title: "Next frost".into(),
            value: when(&band.slots[i]).into(),
            slot: i as i32,
        });
    }
    state.set_sentence(sentence.join(", ").into());
    state.set_facts(ModelRc::new(VecModel::from(cards)));
    let (low, high) = band
        .days
        .iter()
        .fold((f32::MAX, f32::MIN), |m, d| (m.0.min(d.low), m.1.max(d.high)));
    let curve_width = (band.width / 7.0).max(MIN_TILE) - TILE_MARGIN;
    let cards: Vec<DayCard> = band
        .days
        .iter()
        .map(|d| day_card(d, low, high, units, curve_width))
        .collect();
    state.set_curve_width(curve_width);
    state.set_details_row(
        band.days
            .iter()
            .any(|d| d.gust.is_some_and(|g| g >= GUST_SHOWN) || d.sunshine.is_some_and(|s| s >= SUN_SHOWN)),
    );
    state.set_today_index(band.days.iter().position(|d| d.today).unwrap_or(0) as i32);
    state.set_days(ModelRc::new(VecModel::from(cards)));
    state.set_overview(timeline::overview(&band.slots, band.width.max(1.0), OVERVIEW_HEIGHT).into());
}

fn stamp(at: Option<DateTime<Utc>>) -> String {
    at.map_or("–".to_owned(), |t| {
        t.with_timezone(&Local).format("%H:%M").to_string()
    })
}

/// New data: the band starts over from it, on the same stretch.
pub fn refresh(app: &App) {
    {
        let snapshot = app.snapshot.borrow();
        let mut band = app.band.borrow_mut();
        let now = Utc::now();
        // Measured hours can arrive before the forecast, and the band has no now until that is there.
        let first_data = band.slots.iter().all(|s| s.measured || s.temperature.is_none());
        band.slots = timeline::slots(snapshot.forecast.as_ref(), &snapshot.archive, now);
        band.days = timeline::days(&band.slots, snapshot.forecast.as_ref(), Local::now().date_naive());
        band.typical = snapshot.archive.typical;
        band.warnings = snapshot
            .alerts
            .iter()
            .filter_map(|a| {
                Some(Warning {
                    from: a.onset?.with_timezone(&Local),
                    to: a.expires?.with_timezone(&Local),
                    name: a.event.clone(),
                    severe: a.level >= 3,
                })
            })
            .collect();
        if band.start.is_none() || first_data {
            band.go_to_now();
        }
        band.settle();
        let state = app.main.global::<ForecastState>();
        state.set_stamp(format!("Forecast from {}", stamp(snapshot.forecast_at)).into());
        state.set_stamp_tip(
            format!(
                "Fetched at {}, the next one comes at {}. The interval is in the settings, on the page General",
                stamp(snapshot.forecast_at),
                stamp(snapshot.forecast_next)
            )
            .into(),
        );
        show_summary(app, &band);
    }
    show_filters(app);
    draw(app);
}

fn remember(app: &App) {
    {
        let band = app.band.borrow();
        let mut ui = app.ui.borrow_mut();
        ui.forecast_span = band.span;
        ui.forecast_grid_known = true;
        ui.forecast_filters = Filter::ALL
            .into_iter()
            .filter(|f| band.on.contains(f))
            .map(|f| f.key().to_owned())
            .collect();
    }
    app.save_ui();
}

fn hover(app: &App, x: f32) {
    let state = app.main.global::<ForecastState>();
    let band = app.band.borrow();
    let Some(start) = band.start.filter(|_| band.width > 0.0) else {
        return;
    };
    let index = (start + f64::from(x / band.width) * band.span).round();
    let Some(slot) = band.slots.get(index.max(0.0) as usize) else {
        return;
    };
    let (title, lines) = timeline::tip(slot, &app.settings.borrow().units);
    let lines: Vec<SharedString> = lines.into_iter().map(Into::into).collect();
    state.set_tip_title(title.into());
    state.set_tip_lines(ModelRc::new(VecModel::from(lines)));
    state.set_hover_x(((index - start) / band.span) as f32 * band.width);
}

pub fn wire(app: &Rc<App>) {
    // The line of now moves on while the page is open.
    let weak = Rc::downgrade(app);
    app.band_timer
        .start(slint::TimerMode::Repeated, Duration::from_secs(60), move || {
            if let Some(app) = weak.upgrade()
                && app.main.get_page() == crate::app::FORECAST
                && app.main.window().is_visible()
            {
                refresh(&app);
            }
        });
    let state = app.main.global::<ForecastState>();
    state.set_day_view(app.ui.borrow().forecast_day_view);
    let weak = Rc::downgrade(app);
    macro_rules! on {
        ($setter:ident, |$app:ident $(, $arg:ident)*| $body:block) => {{
            let weak = weak.clone();
            state.$setter(move |$($arg),*| {
                if let Some($app) = weak.upgrade() { $body }
            });
        }};
    }
    on!(on_resized, |app, width| {
        app.band.borrow_mut().width = width;
        refresh(&app);
    });
    on!(on_pan, |app, dx| {
        {
            let mut band = app.band.borrow_mut();
            let hours = f64::from(dx / band.width.max(1.0)) * band.span;
            band.start = band.start.map(|s| s - hours);
            band.settle();
        }
        draw(&app);
    });
    on!(on_pan_done, |app| {
        remember(&app);
    });
    on!(on_zoom, |app, steps, x| {
        {
            let mut band = app.band.borrow_mut();
            let at = f64::from(x / band.width.max(1.0));
            let before = band.span;
            let middle = band.start.unwrap_or(0.0) + at * before;
            band.span = before * if steps > 0.0 { 0.85 } else { 1.18 };
            band.settle();
            let span = band.span;
            band.start = Some(middle - at * span);
            band.settle();
        }
        draw(&app);
        remember(&app);
    });
    on!(on_hover, |app, x| {
        hover(&app, x);
    });
    on!(on_hover_end, |app| {
        app.main.global::<ForecastState>().set_hover_x(-10.0);
    });
    on!(on_seek, |app, share| {
        {
            let mut band = app.band.borrow_mut();
            let middle = f64::from(share) * band.all();
            let span = band.span;
            band.start = Some(middle - span / 2.0);
            band.settle();
        }
        draw(&app);
    });
    on!(on_pick_span, |app, hours| {
        {
            let mut band = app.band.borrow_mut();
            let middle = band.start.unwrap_or(0.0) + band.span / 2.0;
            band.span = if hours == 0 { f64::MAX / 4.0 } else { f64::from(hours) };
            band.settle();
            let span = band.span;
            band.start = Some(middle - span / 2.0);
            band.settle();
        }
        draw(&app);
        remember(&app);
    });
    on!(on_to_now, |app| {
        app.band.borrow_mut().go_to_now();
        draw(&app);
    });
    on!(on_page, |app, direction| {
        {
            let mut band = app.band.borrow_mut();
            let span = band.span;
            band.start = band.start.map(|s| s + f64::from(direction) * span);
            band.settle();
        }
        draw(&app);
    });
    on!(on_toggle, |app, index| {
        if let Some(filter) = usize::try_from(index).ok().and_then(|i| Filter::ALL.get(i).copied()) {
            let mut band = app.band.borrow_mut();
            if !band.on.remove(&filter) {
                band.on.insert(filter);
            }
        }
        show_filters(&app);
        draw(&app);
        remember(&app);
    });
    on!(on_jump, |app, slot| {
        {
            let mut band = app.band.borrow_mut();
            let span = band.span;
            band.start = Some(f64::from(slot) - span * 0.1);
            band.settle();
        }
        draw(&app);
    });
    on!(on_view_chosen, |app, view| {
        app.ui.borrow_mut().forecast_day_view = view;
        app.save_ui();
    });
}
