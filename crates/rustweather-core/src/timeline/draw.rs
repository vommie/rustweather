//! Turns a stretch of the band into lines, areas and numbers for a given width.

use std::collections::HashSet;

use chrono::{DateTime, Datelike as _, Local, Timelike as _, Utc};

use super::{CERTAIN_HOURS, Filter, Slot, dry_spells, slot_of};
use crate::archive::Typical;
use crate::color::Rgb;
use crate::settings::{TempUnit, Units};
use crate::tooltip;

const AXIS: f32 = 22.0;
const LANE_GAP: f32 = 8.0;
const MIN_LABEL_GAP: f32 = 46.0;
const DAY_NAME_WIDTH: f32 = 50.0;
const GRID_STEPS: [f32; 5] = [1.0, 2.0, 5.0, 10.0, 20.0];
const MOST_GRID_LINES: f32 = 6.0;
/// Beside the numbers at the lane's top and bottom, which share its right edge.
const MIDDLE_LABEL_AT: f32 = 56.0;

/// The stretch of hours on screen; `start` and `span` count slots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stretch {
    pub start: f64,
    pub span: f64,
}

/// One shape: a line where `stroke` is set, an area where `fill` is.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub commands: String,
    pub stroke: Option<(Rgb, f32)>,
    pub fill: Option<(Rgb, f32)>,
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub x: f32,
    pub y: f32,
    pub text: String,
    /// Empty means the theme's text.
    pub color: Option<Rgb>,
    pub bold: bool,
    /// Left of `x` instead of right of it.
    pub right: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawing {
    pub height: f32,
    pub shapes: Vec<Shape>,
    pub labels: Vec<Label>,
    pub now_x: Option<f32>,
}

/// A warning for the band.
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub from: DateTime<Local>,
    pub to: DateTime<Local>,
    pub name: String,
    pub severe: bool,
}

pub struct Input<'a> {
    pub slots: &'a [Slot],
    pub stretch: Stretch,
    pub width: f32,
    pub on: &'a HashSet<Filter>,
    pub warnings: &'a [Warning],
    pub typical: Option<Typical>,
    pub units: &'a Units,
    pub now: DateTime<Utc>,
    pub dark: bool,
}

fn line_shape(commands: String, color: Rgb, width: f32) -> Shape {
    Shape {
        commands,
        stroke: Some((color, 1.0)),
        fill: None,
        width,
    }
}

fn area_shape(commands: String, color: Rgb, alpha: f32) -> Shape {
    Shape {
        commands,
        stroke: None,
        fill: Some((color, alpha)),
        width: 0.0,
    }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> String {
    format!("M{x:.1} {y:.1}h{w:.1}v{h:.1}h{:.1}Z", -w)
}

/// A line through the points; a gap ends it and starts it anew.
fn polyline(points: impl Iterator<Item = Option<(f32, f32)>>) -> String {
    let (mut out, mut open) = (String::new(), false);
    for point in points {
        match point {
            Some((x, y)) => {
                out += &format!("{}{x:.1} {y:.1}", if open { "L" } else { "M" });
                open = true;
            }
            None => open = false,
        }
    }
    out
}

/// The same with dashes.
fn dashed(points: &[(f32, f32)], on: f32, off: f32) -> String {
    let mut out = String::new();
    let (mut drawn, mut pen) = (0.0, true);
    for pair in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        let length = (x1 - x0).hypot(y1 - y0);
        let mut at = 0.0;
        while at < length {
            let limit = if pen { on } else { off };
            let step = (limit - drawn).min(length - at);
            let (a, b) = (at / length, (at + step) / length);
            if pen {
                out += &format!(
                    "M{:.1} {:.1}L{:.1} {:.1}",
                    x0 + (x1 - x0) * a,
                    y0 + (y1 - y0) * a,
                    x0 + (x1 - x0) * b,
                    y0 + (y1 - y0) * b
                );
            }
            at += step;
            drawn += step;
            if drawn >= limit {
                drawn = 0.0;
                pen = !pen;
            }
        }
    }
    out
}

/// How a label looks.
#[derive(Clone, Copy, Default)]
struct Ink {
    color: Option<Rgb>,
    bold: bool,
    right: bool,
}

const PLAIN: Ink = Ink {
    color: None,
    bold: false,
    right: false,
};
const BOLD: Ink = Ink {
    color: None,
    bold: true,
    right: false,
};
const RIGHT: Ink = Ink {
    color: None,
    bold: false,
    right: true,
};

fn colored(color: Rgb) -> Ink {
    Ink {
        color: Some(color),
        ..Ink::default()
    }
}

struct Lane {
    top: f32,
    height: f32,
}

struct Painter<'a> {
    input: &'a Input<'a>,
    first: usize,
    last: usize,
    now_slot: f64,
    out: Drawing,
}

impl Painter<'_> {
    fn x(&self, slot: f64) -> f32 {
        ((slot - self.input.stretch.start) / self.input.stretch.span) as f32 * self.input.width
    }

    fn on(&self, filter: Filter) -> bool {
        self.input.on.contains(&filter)
    }

    fn visible(&self) -> impl Iterator<Item = (usize, &Slot)> {
        (self.first..=self.last).filter_map(|i| Some((i, self.input.slots.get(i)?)))
    }

    fn text(&mut self, x: f32, y: f32, text: String, ink: Ink) {
        self.out.labels.push(Label {
            x,
            y,
            text,
            color: ink.color,
            bold: ink.bold,
            right: ink.right,
        });
    }

    fn title(&mut self, lane: &Lane, text: &str) {
        self.text(4.0, lane.top - 2.0, text.to_owned(), PLAIN);
    }

    fn series(&mut self, lane: &Lane, filter: Filter, top: f32, bottom: f32, value: impl Fn(&Slot) -> Option<f32>) {
        let y = |v: f32| lane.top + lane.height - (v - bottom) / (top - bottom).max(0.1) * lane.height;
        let points: Vec<_> = self
            .visible()
            .map(|(i, s)| value(s).map(|v| (self.x(i as f64), y(v))))
            .collect();
        let commands = polyline(points.into_iter());
        self.out
            .shapes
            .push(line_shape(commands, filter.color(self.input.dark), 1.6));
    }

    /// The numbers at the top and the bottom of a lane.
    fn scale(&mut self, lane: &Lane, top: String, bottom: String) {
        let w = self.input.width - 4.0;
        self.text(w, lane.top - 2.0, top, RIGHT);
        self.text(w, lane.top + lane.height - 11.0, bottom, RIGHT);
    }

    fn zones(&mut self, height: f32) {
        let width = self.input.width;
        let now_x = self.x(self.now_slot).clamp(0.0, width);
        if now_x > 0.0 {
            self.out.shapes.push(area_shape(
                rect(0.0, 0.0, now_x, height),
                Filter::Night.color(self.input.dark),
                0.1,
            ));
            self.text(now_x - 5.0, 13.0, "Measured".into(), Ink { right: true, ..BOLD });
        }
        let far = self.x(self.now_slot + CERTAIN_HOURS as f64);
        if now_x < width {
            self.text(now_x + 5.0, 13.0, "Forecast".into(), BOLD);
        }
        if far < width {
            let from = far.max(0.0);
            self.out.shapes.push(area_shape(
                rect(from, 0.0, width - from, height),
                Filter::Night.color(self.input.dark),
                0.07,
            ));
            self.text(from + 5.0, 13.0, "Only a rough direction from here".into(), BOLD);
        }
    }

    fn night(&mut self, height: f32) {
        let mut commands = String::new();
        let step = self.x(1.0) - self.x(0.0);
        for (i, slot) in self.visible() {
            if slot.night {
                commands += &rect(self.x(i as f64) - step / 2.0, AXIS, step, height - AXIS);
            }
        }
        self.out
            .shapes
            .push(area_shape(commands, Filter::Night.color(self.input.dark), 0.16));
    }

    fn axis(&mut self, height: f32) {
        let per_hour = self.x(1.0) - self.x(0.0);
        let every = [1, 2, 3, 6, 12, 24]
            .into_iter()
            .find(|k| *k as f32 * per_hour >= 32.0)
            .unwrap_or(48);
        let mut grid = String::new();
        let marks: Vec<_> = self.visible().map(|(i, s)| (i, s.time)).collect();
        let mut day_x = f32::MIN;
        for (i, time) in marks {
            let x = self.x(i as f64);
            if time.hour() == 0 {
                day_x = x;
                grid += &format!("M{x:.1} {AXIS}V{height}");
                let name = if per_hour * 24.0 >= 44.0 {
                    time.format("%a %-d").to_string()
                } else {
                    time.format("%-d").to_string()
                };
                self.text(x + 3.0, 0.0, name, BOLD);
            } else if every < 24 && (time.hour() as usize).is_multiple_of(every) && x - day_x >= DAY_NAME_WIDTH {
                self.text(
                    x - 6.0,
                    0.0,
                    time.hour().to_string(),
                    colored(Filter::Cloud.color(self.input.dark)),
                );
            }
        }
        self.out
            .shapes
            .push(line_shape(grid, Filter::Night.color(self.input.dark), 1.0));
    }

    fn temperature(&mut self, lane: &Lane) {
        let (mut low, mut high) = (f32::MAX, f32::MIN);
        let mut see = |v: f32| {
            low = low.min(v);
            high = high.max(v);
        };
        let (feels, spread, typical) = (
            self.on(Filter::Feels),
            self.on(Filter::Spread),
            self.on(Filter::Typical),
        );
        for (_, slot) in self.visible() {
            slot.temperature.into_iter().for_each(&mut see);
            slot.feels.filter(|_| feels).into_iter().for_each(&mut see);
            if let Some((a, b)) = slot.spread.filter(|_| spread) {
                see(a);
                see(b);
            }
        }
        if let Some(t) = self.input.typical.filter(|_| typical) {
            see(t.low);
            see(t.high);
        }
        if low > high {
            return;
        }
        let (low, high) = (low - 1.0, high + 1.0);
        let y = |v: f32| lane.top + lane.height - (v - low) / (high - low) * lane.height;
        if self.on(Filter::Grid) {
            self.temperature_grid(low, high, &y);
        }
        if let Some(t) = self.input.typical.filter(|_| typical) {
            let shape = rect(0.0, y(t.high), self.input.width, y(t.low) - y(t.high));
            self.out
                .shapes
                .push(area_shape(shape, Filter::Typical.color(self.input.dark), 0.1));
            let unit = self.input.units.temperature;
            let text = format!(
                "Usual here: {} to {}",
                tooltip::temperature(t.low, unit),
                tooltip::temperature(t.high, unit)
            );
            self.text(
                4.0,
                y(t.high) + 1.0,
                text,
                colored(Filter::Typical.color(self.input.dark)),
            );
        }
        if spread {
            let top: Vec<_> = self
                .visible()
                .filter_map(|(i, s)| Some((self.x(i as f64), y(s.spread?.1))))
                .collect();
            let bottom: Vec<_> = self
                .visible()
                .filter_map(|(i, s)| Some((self.x(i as f64), y(s.spread?.0))))
                .collect();
            if !top.is_empty() {
                let mut commands = polyline(top.into_iter().map(Some));
                commands += &polyline(bottom.into_iter().rev().map(Some)).replacen('M', "L", 1);
                self.out
                    .shapes
                    .push(area_shape(commands + "Z", Filter::Spread.color(self.input.dark), 0.18));
            }
        }
        if feels {
            let points: Vec<_> = self
                .visible()
                .filter_map(|(i, s)| Some((self.x(i as f64), y(s.feels?))))
                .collect();
            self.out.shapes.push(line_shape(
                dashed(&points, 2.0, 3.0),
                Filter::Feels.color(self.input.dark),
                1.0,
            ));
        }
        if self.on(Filter::Announced) {
            let points: Vec<_> = self
                .visible()
                .filter(|(_, s)| s.measured)
                .filter_map(|(i, s)| Some((self.x(i as f64), y(s.announced?))))
                .collect();
            self.out.shapes.push(line_shape(
                dashed(&points, 5.0, 3.0),
                Filter::Announced.color(self.input.dark),
                1.4,
            ));
        }
        if self.on(Filter::Temperature) {
            self.temperature_line(lane, low, high);
        }
    }

    fn temperature_grid(&mut self, low: f32, high: f32, y: &dyn Fn(f32) -> f32) {
        let unit = self.input.units.temperature;
        let width = self.input.width;
        let color = Filter::Grid.color(self.input.dark);
        let mut lines = String::new();
        for value in round_values(shown(low, unit), shown(high, unit)) {
            let celsius = from_shown(value, unit);
            lines += &format!("M0 {:.1}H{width}", y(celsius));
            self.text(width - 4.0, y(celsius) - 13.0, format!("{value:.0}°"), RIGHT);
        }
        self.out.shapes.push(Shape {
            commands: lines,
            stroke: Some((color, 0.45)),
            fill: None,
            width: 1.0,
        });
        if low < 0.0 && high > 0.0 {
            self.out
                .shapes
                .push(line_shape(format!("M0 {:.1}H{width}", y(0.0)), color, 1.6));
        }
    }

    /// A line through the middle of a lane, with its value.
    fn middle(&mut self, lane: &Lane, text: String) {
        if !self.on(Filter::Grid) {
            return;
        }
        let (y, width) = (lane.top + lane.height / 2.0, self.input.width);
        self.out.shapes.push(Shape {
            commands: format!("M0 {y:.1}H{width}"),
            stroke: Some((Filter::Grid.color(self.input.dark), 0.45)),
            fill: None,
            width: 1.0,
        });
        self.text(width - MIDDLE_LABEL_AT, y - 13.0, text, RIGHT);
    }

    fn temperature_line(&mut self, lane: &Lane, low: f32, high: f32) {
        let y = |v: f32| lane.top + lane.height - (v - low) / (high - low) * lane.height;
        let color = Filter::Temperature.color(self.input.dark);
        let (now, far) = (self.now_slot, self.now_slot + CERTAIN_HOURS as f64);
        let pick = |from: f64, to: f64| -> Vec<(f32, f32)> {
            self.visible()
                .filter(|(i, _)| (*i as f64) >= from && (*i as f64) <= to)
                .filter_map(|(i, s)| Some((self.x(i as f64), y(s.temperature?))))
                .collect()
        };
        let (past, near, rough) = (pick(f64::MIN, now.ceil()), pick(now.floor(), far), pick(far, f64::MAX));
        self.out
            .shapes
            .push(line_shape(polyline(past.into_iter().map(Some)), color, 2.4));
        self.out.shapes.push(line_shape(dashed(&near, 9.0, 3.0), color, 2.0));
        self.out.shapes.push(line_shape(dashed(&rough, 3.0, 4.0), color, 1.6));
        let per_hour = self.x(1.0) - self.x(0.0);
        let every = [1usize, 2, 3, 6, 12, 24, 48]
            .into_iter()
            .find(|k| *k as f32 * per_hour >= MIN_LABEL_GAP)
            .unwrap_or(96);
        let unit = self.input.units.temperature;
        let marks: Vec<_> = self
            .visible()
            .filter(|(_, s)| {
                (s.time.hour() as usize).is_multiple_of(every.min(24))
                    && (every < 48 || s.time.ordinal0().is_multiple_of(every as u32 / 24))
            })
            .filter_map(|(i, s)| Some((self.x(i as f64), y(s.temperature?), s.temperature?)))
            .collect();
        for (x, top, value) in marks {
            self.text(x - 8.0, top - 15.0, tooltip::temperature(value, unit), BOLD);
        }
    }

    fn warnings(&mut self, lane: &Lane) {
        if !self.on(Filter::Warnings) {
            return;
        }
        let first = self.input.slots.first().map(|s| s.time);
        let Some(first) = first else { return };
        for warning in self.input.warnings {
            let slot = |t: DateTime<Local>| (t - first).num_minutes() as f64 / 60.0;
            let (a, b) = (
                self.x(slot(warning.from)).max(0.0),
                self.x(slot(warning.to)).min(self.input.width),
            );
            if b <= a {
                continue;
            }
            let color = if warning.severe {
                Rgb(0xf3, 0x8b, 0xa8)
            } else {
                Filter::Warnings.color(self.input.dark)
            };
            self.out
                .shapes
                .push(area_shape(rect(a, lane.top - 3.0, b - a, 5.0), color, 0.9));
            if b - a > 90.0 {
                self.text(
                    a + 2.0,
                    lane.top + 3.0,
                    warning.name.clone(),
                    Ink {
                        bold: true,
                        ..colored(color)
                    },
                );
            }
        }
    }

    fn rain(&mut self, lane: &Lane) {
        let step = (self.x(1.0) - self.x(0.0)).max(1.0);
        let bar = (step - 0.5).max(0.8);
        let top = self.visible().map(|(_, s)| s.rain + s.snow).fold(1.0, f32::max);
        let bottom = lane.top + lane.height;
        self.middle(lane, format!("{:.1}", top / 2.0));
        if self.on(Filter::Chance) {
            let mut commands = String::new();
            for (i, slot) in self.visible() {
                if let Some(chance) = slot.chance.filter(|c| *c > 0.0) {
                    let h = chance / 100.0 * lane.height;
                    commands += &rect(self.x(i as f64) - bar / 2.0, bottom - h, bar, h);
                }
            }
            self.out
                .shapes
                .push(area_shape(commands, Filter::Chance.color(self.input.dark), 0.16));
        }
        for (filter, value) in [(Filter::Rain, 0), (Filter::Snow, 1)] {
            if !self.on(filter) {
                continue;
            }
            let mut commands = String::new();
            for (i, slot) in self.visible() {
                let v = if value == 0 { slot.rain } else { slot.snow };
                if v > 0.0 {
                    let h = (v / top * lane.height).max(1.5);
                    commands += &rect(self.x(i as f64) - bar / 2.0, bottom - h, bar, h);
                }
            }
            self.out
                .shapes
                .push(area_shape(commands, filter.color(self.input.dark), 1.0));
        }
        if self.on(Filter::Dry) {
            let mut commands = String::new();
            for (from, to) in dry_spells(self.input.slots, self.now_slot) {
                let (a, b) = (self.x(from as f64).max(0.0), self.x(to as f64).min(self.input.width));
                if b > a {
                    commands += &rect(a, lane.top + lane.height + 1.0, b - a, 3.0);
                }
            }
            self.out
                .shapes
                .push(area_shape(commands, Filter::Dry.color(self.input.dark), 0.8));
        }
        self.title(lane, "Rain, mm");
        self.scale(lane, format!("{top:.1}"), "0".into());
    }

    fn wind(&mut self, lane: &Lane) {
        let top = self
            .visible()
            .filter_map(|(_, s)| s.gust.or(s.wind))
            .fold(30.0, f32::max);
        self.middle(lane, tooltip::wind(top / 2.0, self.input.units.wind));
        for (filter, value) in [
            (Filter::Gust, (|s: &Slot| s.gust) as fn(&Slot) -> Option<f32>),
            (Filter::Wind, (|s: &Slot| s.wind) as fn(&Slot) -> Option<f32>),
        ] {
            if self.on(filter) {
                self.series(lane, filter, top, 0.0, value);
            }
        }
        self.title(lane, "Wind");
        let units = self.input.units;
        self.scale(lane, tooltip::wind(top, units.wind), "0".into());
    }

    fn percent(&mut self, lane: &Lane, filter: Filter, title: &str, value: fn(&Slot) -> Option<f32>) {
        self.middle(lane, "50 %".into());
        self.series(lane, filter, 100.0, 0.0, value);
        self.title(lane, title);
        self.scale(lane, "100 %".into(), "0 %".into());
    }

    fn pressure(&mut self, lane: &Lane) {
        let (low, high) = self
            .visible()
            .filter_map(|(_, s)| s.pressure)
            .fold((f32::MAX, f32::MIN), |m, v| (m.0.min(v), m.1.max(v)));
        if low > high {
            return;
        }
        self.middle(lane, format!("{:.0}", (high + low) / 2.0));
        self.series(lane, Filter::Pressure, high + 1.0, low - 1.0, |s| s.pressure);
        self.title(lane, "Pressure, hPa");
        self.scale(lane, format!("{:.0}", high + 1.0), format!("{:.0}", low - 1.0));
    }

    fn uv(&mut self, lane: &Lane) {
        let top = self.visible().filter_map(|(_, s)| s.uv).fold(4.0, f32::max);
        self.middle(lane, format!("{:.0}", top / 2.0));
        self.series(lane, Filter::Uv, top, 0.0, |s| s.uv);
        self.title(lane, "UV");
        self.scale(lane, format!("{top:.0}"), "0".into());
    }
}

/// A temperature in the unit on screen.
fn shown(celsius: f32, unit: TempUnit) -> f32 {
    match unit {
        TempUnit::Celsius => celsius,
        TempUnit::Fahrenheit => celsius * 9.0 / 5.0 + 32.0,
    }
}

fn from_shown(value: f32, unit: TempUnit) -> f32 {
    match unit {
        TempUnit::Celsius => value,
        TempUnit::Fahrenheit => (value - 32.0) * 5.0 / 9.0,
    }
}

/// Round values between two ends: few enough to stay calm, whatever the range.
fn round_values(low: f32, high: f32) -> Vec<f32> {
    let step = GRID_STEPS
        .into_iter()
        .find(|s| (high - low) / s <= MOST_GRID_LINES)
        .unwrap_or(50.0);
    let first = (low / step).ceil() as i32;
    let last = (high / step).floor() as i32;
    (first..=last).map(|k| k as f32 * step).collect()
}

pub fn draw(input: &Input<'_>) -> Drawing {
    let on = |f: Filter| input.on.contains(&f);
    let tall = [
        (on(Filter::Temperature) || on(Filter::Feels), 170.0),
        (on(Filter::Rain) || on(Filter::Chance) || on(Filter::Snow), 60.0),
        (on(Filter::Wind) || on(Filter::Gust), 56.0),
        (on(Filter::Humidity), 44.0),
        (on(Filter::Cloud), 44.0),
        (on(Filter::Uv), 44.0),
        (on(Filter::Pressure), 44.0),
    ];
    let height = AXIS + tall.iter().filter(|t| t.0).map(|t| t.1 + LANE_GAP + 14.0).sum::<f32>() + 4.0;
    let first = input.stretch.start.floor().max(0.0) as usize;
    let last = ((input.stretch.start + input.stretch.span).ceil() as usize).min(input.slots.len().saturating_sub(1));
    let now_slot = slot_of(input.slots, input.now);
    let mut p = Painter {
        input,
        first,
        last,
        now_slot,
        out: Drawing {
            height,
            ..Drawing::default()
        },
    };
    p.zones(height);
    if on(Filter::Night) {
        p.night(height);
    }
    p.axis(height);
    let mut y = AXIS;
    for (index, _) in tall.iter().enumerate().filter(|(_, t)| t.0) {
        let lane = Lane {
            top: y + LANE_GAP + 8.0,
            height: tall[index].1 - 8.0,
        };
        match index {
            0 => {
                p.temperature(&lane);
                p.warnings(&lane);
            }
            1 => p.rain(&lane),
            2 => p.wind(&lane),
            3 => p.percent(&lane, Filter::Humidity, "Humidity", |s| s.humidity),
            4 => p.percent(&lane, Filter::Cloud, "Clouds", |s| s.cloud),
            5 => p.uv(&lane),
            _ => p.pressure(&lane),
        }
        y += tall[index].1 + LANE_GAP + 14.0;
    }
    let x = p.x(now_slot);
    p.out.now_x = (x >= 0.0 && x <= input.width).then_some(x);
    p.out
}

/// The temperature of all the band as a thin line, for the overview.
pub fn overview(slots: &[Slot], width: f32, height: f32) -> String {
    let (low, high) = slots
        .iter()
        .filter_map(|s| s.temperature)
        .fold((f32::MAX, f32::MIN), |m, v| (m.0.min(v), m.1.max(v)));
    if low > high || slots.len() < 2 {
        return String::new();
    }
    let span = (high - low).max(1.0);
    let step = width / (slots.len() - 1) as f32;
    polyline(slots.iter().enumerate().map(|(i, s)| {
        s.temperature
            .map(|t| (i as f32 * step, height - 3.0 - (t - low) / span * (height - 6.0)))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_values_stay_few_at_any_range() {
        assert_eq!(round_values(2.3, 7.6), [3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(round_values(-3.0, 12.0), [0.0, 5.0, 10.0]);
        assert!(round_values(-40.0, 45.0).len() <= 7);
    }

    #[test]
    fn fahrenheit_lines_are_round_in_fahrenheit() {
        assert_eq!(shown(100.0, TempUnit::Fahrenheit), 212.0);
        assert_eq!(from_shown(212.0, TempUnit::Fahrenheit), 100.0);
    }

    #[test]
    fn dashes_keep_the_line_going() {
        assert_eq!(dashed(&[(0.0, 0.0), (10.0, 0.0)], 4.0, 2.0).matches('M').count(), 2);
        assert!(dashed(&[], 4.0, 2.0).is_empty());
    }

    #[test]
    fn a_gap_splits_a_line() {
        let line = polyline([Some((0.0, 0.0)), Some((1.0, 1.0)), None, Some((2.0, 2.0))].into_iter());
        assert_eq!(line.matches('M').count(), 2);
    }
}
