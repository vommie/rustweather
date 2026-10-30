//! The warnings at a point of the map, from the same DWD layer the map draws, so a click always finds what is shown there. The DWD writes
//! these texts in German. Bright Sky is no help here: it answers per municipality, the map shows districts.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::error::Result;
use crate::http::Http;
use crate::map::{Extent, to_unit};
use crate::model::Alert;

/// Half the side of the small square that is asked about, in the unit square: about 3 km.
const REACH: f64 = 0.000_08;
/// Half the width of the world in the server's Web Mercator meters.
const HALF_WORLD: f64 = 6_378_137.0 * std::f64::consts::PI;
/// The longer side of a shape's own drawing space.
const DRAWING_SIZE: f64 = 10_000.0;

/// What the map knows about a place.
#[derive(Debug, Default)]
pub struct Found {
    /// Strongest first.
    pub alerts: Vec<Alert>,
    pub area: String,
    pub outline: Option<Outline>,
}

/// The shape of a district in the unit square of the map.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    rings: Vec<Vec<(f64, f64)>>,
    pub extent: Extent,
}

impl Outline {
    fn new(rings: Vec<Vec<(f64, f64)>>) -> Option<Self> {
        let points = || rings.iter().flatten();
        let fold =
            |pick: fn(&(f64, f64)) -> f64, join: fn(f64, f64) -> f64, start: f64| points().map(pick).fold(start, join);
        points().next()?;
        let extent = Extent {
            x0: fold(|p| p.0, f64::min, f64::MAX),
            y0: fold(|p| p.1, f64::min, f64::MAX),
            x1: fold(|p| p.0, f64::max, f64::MIN),
            y1: fold(|p| p.1, f64::max, f64::MIN),
        };
        Some(Self { rings, extent })
    }

    /// Is a point of the unit square inside? Holes count as outside.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        let mut inside = false;
        for ring in &self.rings {
            for pair in ring.windows(2) {
                let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
                if (y0 > y) != (y1 > y) && x < x0 + (y - y0) / (y1 - y0) * (x1 - x0) {
                    inside = !inside;
                }
            }
        }
        inside
    }

    /// The shape as SVG path commands, and the width and height of the space they are drawn in.
    pub fn path(&self) -> (String, f32, f32) {
        let e = self.extent;
        let unit = (e.x1 - e.x0).max(e.y1 - e.y0).max(f64::MIN_POSITIVE) / DRAWING_SIZE;
        let mut commands = String::new();
        for ring in &self.rings {
            for (index, (x, y)) in ring.iter().enumerate() {
                let letter = if index == 0 { 'M' } else { 'L' };
                commands += &format!("{letter}{:.1} {:.1} ", (x - e.x0) / unit, (y - e.y0) / unit);
            }
            commands.push('Z');
        }
        (commands, ((e.x1 - e.x0) / unit) as f32, ((e.y1 - e.y0) / unit) as f32)
    }
}

#[derive(Deserialize, Default)]
struct Answer {
    #[serde(default)]
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    properties: Properties,
    geometry: Option<Geometry>,
}

#[derive(Deserialize)]
struct Geometry {
    #[serde(rename = "type")]
    kind: String,
    coordinates: Value,
}

/// The rings of a shape in the map's unit square.
fn rings(geometry: &Geometry) -> Vec<Vec<(f64, f64)>> {
    let ring = |points: &Value| -> Vec<(f64, f64)> {
        points
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let (x, y) = (p.get(0)?.as_f64()?, p.get(1)?.as_f64()?);
                Some((x / HALF_WORLD / 2.0 + 0.5, 0.5 - y / HALF_WORLD / 2.0))
            })
            .collect()
    };
    let list = |value: &Value| value.as_array().cloned().unwrap_or_default();
    match geometry.kind.as_str() {
        "Polygon" => list(&geometry.coordinates).iter().map(ring).collect(),
        "MultiPolygon" => list(&geometry.coordinates)
            .iter()
            .flat_map(list)
            .map(|points| ring(&points))
            .collect(),
        _ => Vec::new(),
    }
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
}

fn level(severity: &str) -> u8 {
    match severity.to_lowercase().as_str() {
        "extreme" => 4,
        "severe" => 3,
        "moderate" => 2,
        _ => 1,
    }
}

fn parse(answer: Answer) -> Found {
    let area = answer
        .features
        .first()
        .map(|f| f.properties.areadesc.clone())
        .unwrap_or_default();
    let outline = answer
        .features
        .iter()
        .find_map(|f| f.geometry.as_ref())
        .and_then(|g| Outline::new(rings(g)));
    let mut alerts: Vec<Alert> = answer
        .features
        .into_iter()
        .map(|f| {
            let p = f.properties;
            Alert {
                id: p.identifier,
                level: level(&p.severity),
                event: p.event,
                headline: p.headline,
                description: p.description,
                instruction: p.instruction.unwrap_or_default(),
                onset: p.onset,
                expires: p.expires,
            }
        })
        .collect();
    alerts.sort_by_key(|a| std::cmp::Reverse(a.level));
    Found { alerts, area, outline }
}

/// The warnings at a place and the district they are for.
pub async fn at(http: &Http, lon: f64, lat: f64) -> Result<Found> {
    let (x, y) = to_unit(lon, lat);
    let square = Extent {
        x0: x - REACH,
        y0: y - REACH,
        x1: x + REACH,
        y1: y + REACH,
    };
    let url = format!(
        "https://maps.dwd.de/geoserver/dwd/wms?service=WMS&version=1.3.0&request=GetFeatureInfo\
         &layers=dwd:Warnungen_Landkreise&query_layers=dwd:Warnungen_Landkreise&styles=&crs=EPSG:3857&bbox={}\
         &width=5&height=5&i=2&j=2&info_format=application/json&feature_count=20",
        square.bbox_3857()
    );
    Ok(parse(http.json(&url).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_map_servers_answer() {
        let text = r#"{"features":[
            {"properties":{"IDENTIFIER":"a","SEVERITY":"Minor","EVENT":"FROST","HEADLINE":"Amtliche WARNUNG vor FROST","DESCRIPTION":"Leichter Frost.",
             "INSTRUCTION":null,"AREADESC":"Kreis Pfaffenhofen","ONSET":"2026-10-30T22:00:00Z","EXPIRES":"2026-10-31T07:00:00Z","OTHER":1}},
            {"properties":{"IDENTIFIER":"b","SEVERITY":"Severe","EVENT":"STURM","HEADLINE":"Sturm","DESCRIPTION":"Boeen.","AREADESC":"Kreis Pfaffenhofen"}}]}"#;
        let found = parse(serde_json::from_str(text).unwrap());
        assert_eq!(found.area, "Kreis Pfaffenhofen");
        assert_eq!(found.alerts[0].level, 3);
        assert_eq!(found.alerts[1].event, "FROST");
        assert!(found.alerts[1].onset.is_some());
    }

    #[test]
    fn no_warning_is_no_error() {
        let found = parse(serde_json::from_str(r#"{"features":[]}"#).unwrap());
        assert!(found.alerts.is_empty() && found.area.is_empty() && found.outline.is_none());
    }

    fn square() -> Outline {
        // A square of 2 million meters around the middle of the map, with a hole of half its size.
        let text = r#"{"features":[{"properties":{"AREADESC":"A"},"geometry":{"type":"Polygon","coordinates":[
            [[-1000000,-1000000],[1000000,-1000000],[1000000,1000000],[-1000000,1000000],[-1000000,-1000000]],
            [[-500000,-500000],[500000,-500000],[500000,500000],[-500000,500000],[-500000,-500000]]]}}]}"#;
        parse(serde_json::from_str(text).unwrap()).outline.unwrap()
    }

    #[test]
    fn an_outline_knows_what_is_inside() {
        let outline = square();
        let near = 750_000.0 / HALF_WORLD / 2.0;
        assert!(outline.contains(0.5 + near, 0.5));
        assert!(!outline.contains(0.5, 0.5), "the hole");
        assert!(!outline.contains(0.2, 0.5));
    }

    #[test]
    fn an_outline_becomes_a_path_that_fits_its_space() {
        let (commands, width, height) = square().path();
        assert!(commands.starts_with("M0.0 10000.0 L10000.0 10000.0 "), "{commands}");
        assert_eq!(commands.matches('Z').count(), 2);
        assert!((width - 10_000.0).abs() < 0.1 && (height - 10_000.0).abs() < 0.1);
    }

    #[test]
    fn a_multipolygon_keeps_all_its_parts() {
        let text = r#"{"features":[{"properties":{},"geometry":{"type":"MultiPolygon","coordinates":[
            [[[0,0],[10,0],[10,10],[0,0]]],[[[20,20],[30,20],[30,30],[20,20]]]]}}]}"#;
        let outline = parse(serde_json::from_str(text).unwrap()).outline.unwrap();
        assert_eq!(outline.path().0.matches('Z').count(), 2);
    }
}
