//! The warnings at a point of the map, from the same DWD layer the map draws, so a click always finds what is shown there. The DWD writes
//! these texts in German. Bright Sky is no help here: it answers per municipality, the map shows districts.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::Result;
use crate::http::Http;
use crate::map::{Extent, to_unit};
use crate::model::Alert;

/// Half the side of the small square that is asked about, in the unit square: about 3 km.
const REACH: f64 = 0.000_08;

#[derive(Deserialize, Default)]
struct Answer {
    #[serde(default)]
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    properties: Properties,
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

fn parse(answer: Answer) -> (Vec<Alert>, String) {
    let area = answer
        .features
        .first()
        .map(|f| f.properties.areadesc.clone())
        .unwrap_or_default();
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
    (alerts, area)
}

/// The warnings at a place, strongest first, and the name of the district.
pub async fn at(http: &Http, lon: f64, lat: f64) -> Result<(Vec<Alert>, String)> {
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
        let (alerts, area) = parse(serde_json::from_str(text).unwrap());
        assert_eq!(area, "Kreis Pfaffenhofen");
        assert_eq!(alerts[0].level, 3);
        assert_eq!(alerts[1].event, "FROST");
        assert!(alerts[1].onset.is_some());
    }

    #[test]
    fn no_warning_is_no_error() {
        let (alerts, area) = parse(serde_json::from_str(r#"{"features":[]}"#).unwrap());
        assert!(alerts.is_empty() && area.is_empty());
    }
}
