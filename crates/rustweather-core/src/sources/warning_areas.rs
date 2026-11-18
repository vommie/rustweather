//! All current warnings of Germany as shapes, from the DWD's feature service in one request: the app draws them itself at any zoom, and a click
//! finds its warning without asking a server. The map server took seconds for each picture and each click. The texts are the DWD's German ones.

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::warnings::{Found, Outline, level};
use crate::color::Rgb;
use crate::error::{Error, Result};
use crate::http::Http;
use crate::map::{Extent, to_unit};
use crate::model::Alert;
use crate::store;

const URL: &str = "https://maps.dwd.de/geoserver/dwd/ows?service=WFS&version=2.0.0&request=GetFeature\
    &typeNames=dwd:Warnungen_Landkreise&outputFormat=application/json\
    &propertyName=IDENTIFIER,SEVERITY,EVENT,HEADLINE,DESCRIPTION,INSTRUCTION,AREADESC,ONSET,EXPIRES,EC_AREA_COLOR,THE_GEOM";
const TIMEOUT: Duration = Duration::from_secs(90);
/// The longer side of the drawing space of all shapes.
const DRAWING_SIZE: f64 = 100_000.0;

#[derive(Deserialize, Default)]
struct Answer {
    #[serde(default)]
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    #[serde(default)]
    properties: Properties,
    geometry: Option<Geometry>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "UPPERCASE", default)]
struct Properties {
    identifier: String,
    severity: String,
    event: String,
    headline: String,
    description: String,
    instruction: Option<String>,
    areadesc: String,
    onset: Option<DateTime<Utc>>,
    expires: Option<DateTime<Utc>>,
    ec_area_color: String,
}

#[derive(Deserialize)]
struct Geometry {
    #[serde(rename = "type")]
    kind: String,
    coordinates: Value,
}

/// One warning for one district.
#[derive(Debug, Clone)]
pub struct Area {
    pub alert: Alert,
    pub name: String,
    pub color: Rgb,
    pub outline: Outline,
}

/// The warnings of a moment.
#[derive(Debug, Clone, Default)]
pub struct WarningMap {
    /// Weakest first, so the strongest are drawn on top.
    pub areas: Vec<Area>,
    pub extent: Option<Extent>,
}

/// One color of the drawing: its shapes as SVG commands in the drawing space of `WarningMap::extent`.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub color: Rgb,
    pub commands: String,
}

fn rings(geometry: &Geometry) -> Vec<Vec<(f64, f64)>> {
    let ring = |points: &Value| -> Vec<(f64, f64)> {
        points
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Some(to_unit(p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
            .collect()
    };
    let list = |value: &Value| value.as_array().cloned().unwrap_or_default();
    match geometry.kind.as_str() {
        "Polygon" => list(&geometry.coordinates).iter().map(ring).collect(),
        "MultiPolygon" => list(&geometry.coordinates)
            .iter()
            .flat_map(list)
            .map(|r| ring(&r))
            .collect(),
        _ => Vec::new(),
    }
}

fn color(text: &str) -> Rgb {
    let mut parts = text.split_whitespace().filter_map(|v| v.parse::<u8>().ok());
    match (parts.next(), parts.next(), parts.next()) {
        (Some(r), Some(g), Some(b)) => Rgb(r, g, b),
        _ => Rgb(0xf9, 0xe2, 0xaf),
    }
}

fn parse(bytes: &[u8]) -> Option<WarningMap> {
    let answer: Answer = serde_json::from_slice(bytes).ok()?;
    let mut areas: Vec<Area> = answer
        .features
        .into_iter()
        .filter_map(|f| {
            let outline = Outline::from_rings(rings(f.geometry.as_ref()?))?;
            let p = f.properties;
            Some(Area {
                color: color(&p.ec_area_color),
                name: p.areadesc,
                alert: Alert {
                    id: p.identifier,
                    level: level(&p.severity),
                    event: p.event,
                    headline: p.headline,
                    description: p.description,
                    instruction: p.instruction.unwrap_or_default(),
                    onset: p.onset,
                    expires: p.expires,
                },
                outline,
            })
        })
        .collect();
    areas.sort_by_key(|a| a.alert.level);
    let extent = areas.iter().map(|a| a.outline.extent).reduce(|a, b| Extent {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    });
    Some(WarningMap { areas, extent })
}

impl WarningMap {
    /// What holds at a point of the unit square: the warnings of the district under it, strongest first.
    pub fn at(&self, x: f64, y: f64) -> Found {
        let hits: Vec<&Area> = self.areas.iter().filter(|a| a.outline.contains(x, y)).collect();
        let mut alerts: Vec<Alert> = hits.iter().map(|a| a.alert.clone()).collect();
        alerts.sort_by_key(|a| std::cmp::Reverse(a.level));
        Found {
            alerts,
            area: hits.first().map(|a| a.name.clone()).unwrap_or_default(),
            outline: hits.last().map(|a| a.outline.clone()),
        }
    }

    /// The width and height of the drawing space, whose corner is the top left of `extent`.
    pub fn drawing_size(&self) -> (f32, f32) {
        self.extent.map_or((0.0, 0.0), |e| {
            let unit = (e.x1 - e.x0).max(e.y1 - e.y0).max(f64::MIN_POSITIVE) / DRAWING_SIZE;
            (((e.x1 - e.x0) / unit) as f32, ((e.y1 - e.y0) / unit) as f32)
        })
    }

    /// All shapes, one layer per color, weakest first.
    pub fn layers(&self) -> Vec<Layer> {
        let Some(e) = self.extent else {
            return Vec::new();
        };
        let unit = (e.x1 - e.x0).max(e.y1 - e.y0).max(f64::MIN_POSITIVE) / DRAWING_SIZE;
        let mut out: Vec<Layer> = Vec::new();
        for area in &self.areas {
            let commands = area.outline.commands_in(e.x0, e.y0, unit);
            match out.last_mut() {
                Some(layer) if layer.color == area.color => layer.commands += &commands,
                _ => out.push(Layer {
                    color: area.color,
                    commands,
                }),
            }
        }
        out
    }
}

/// The warnings from the disk, when they are not older than `max_age`.
pub fn load(file: &Path, max_age: Duration) -> Option<(WarningMap, DateTime<Utc>)> {
    let modified: DateTime<Utc> = std::fs::metadata(file).ok()?.modified().ok()?.into();
    let age = (Utc::now() - modified).to_std().unwrap_or_default();
    if age > max_age {
        return None;
    }
    Some((parse(&std::fs::read(file).ok()?)?, modified))
}

pub async fn fetch(http: &Http, file: &Path) -> Result<WarningMap> {
    let bytes = http.bytes_within(URL, TIMEOUT).await?;
    let map = parse(&bytes).ok_or_else(|| Error::format("DWD", "no warning shapes in the answer"))?;
    if let Err(error) = store::write_atomic(file, &bytes) {
        tracing::debug!(%error, "[Warnings] Could not cache the shapes");
    }
    Ok(map)
}

/// A small map for tests elsewhere.
#[cfg(test)]
pub(crate) fn sample_map() -> WarningMap {
    tests::map()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"features":[
        {"properties":{"IDENTIFIER":"a","SEVERITY":"Moderate","EVENT":"STURMBÖEN","HEADLINE":"Sturm","AREADESC":"Kreis A","EC_AREA_COLOR":"251 140 0"},
         "geometry":{"type":"Polygon","coordinates":[[[6,50],[8,50],[8,52],[6,52],[6,50]]]}},
        {"properties":{"IDENTIFIER":"b","SEVERITY":"Severe","EVENT":"ORKAN","HEADLINE":"Orkan","AREADESC":"Kreis A","EC_AREA_COLOR":"229 57 53"},
         "geometry":{"type":"Polygon","coordinates":[[[6,50],[8,50],[8,52],[6,52],[6,50]]]}},
        {"properties":{"IDENTIFIER":"c","SEVERITY":"Minor","EVENT":"FROST","AREADESC":"Kreis B","EC_AREA_COLOR":"255 235 59"},
         "geometry":{"type":"MultiPolygon","coordinates":[[[[10,48],[11,48],[11,49],[10,48]]],[[[12,48],[13,48],[13,49],[12,48]]]]}}]}"#;

    pub(super) fn map() -> WarningMap {
        parse(SAMPLE.as_bytes()).unwrap()
    }

    #[test]
    fn a_click_finds_all_warnings_of_its_district_strongest_first() {
        let (x, y) = to_unit(7.0, 51.0);
        let found = map().at(x, y);
        assert_eq!(found.area, "Kreis A");
        assert_eq!(
            found.alerts.iter().map(|a| a.event.as_str()).collect::<Vec<_>>(),
            ["ORKAN", "STURMBÖEN"]
        );
        assert!(found.outline.is_some());
        let (x, y) = to_unit(0.0, 40.0);
        assert!(map().at(x, y).alerts.is_empty());
    }

    #[test]
    fn colors_come_from_the_dwd_and_the_strongest_lie_on_top() {
        let layers = map().layers();
        assert_eq!(layers.first().map(|l| l.color), Some(Rgb(255, 235, 59)));
        assert_eq!(layers.last().map(|l| l.color), Some(Rgb(229, 57, 53)));
        assert_eq!(
            layers[0].commands.matches('Z').count(),
            2,
            "both parts of the multipolygon"
        );
    }

    #[test]
    fn the_drawing_space_keeps_the_shape_of_the_extent() {
        let map = map();
        let (w, h) = map.drawing_size();
        let e = map.extent.unwrap();
        assert!(((w / h) as f64 - (e.x1 - e.x0) / (e.y1 - e.y0)).abs() < 1e-3);
    }

    #[test]
    fn an_error_page_is_no_map() {
        assert!(parse(b"<ows:ExceptionReport/>").is_none());
        assert!(parse(br#"{"features":[]}"#).is_some_and(|m| m.areas.is_empty() && m.extent.is_none()));
    }
}
