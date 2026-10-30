//! The wetterdienst.de station page, read as a fallback when the DWD data is out. Only this module knows the page's layout.

use regex::Regex;
use scraper::{Html, Selector};

use crate::error::{Error, Result};
use crate::http::Http;
use crate::model::Current;

const SERVICE: &str = "wetterdienst.de";

fn number(text: &str, label: &str) -> Option<f32> {
    let re = Regex::new(&format!(r"{label}:\s*(-?[0-9]+(?:[.,][0-9]+)?)")).ok()?;
    re.captures(text)?.get(1)?.as_str().replace(',', ".").parse().ok()
}

fn word(text: &str, label: &str) -> Option<String> {
    let re = Regex::new(&format!(r"{label}:\s*([^\r\n]+)")).ok()?;
    Some(re.captures(text)?.get(1)?.as_str().trim().to_owned())
}

pub fn parse(html: &str) -> Result<Current> {
    let doc = Html::parse_document(html);
    let select = |css: &str| Selector::parse(css).map_err(|e| Error::format(SERVICE, e));
    let row_css = select("table#msgs tbody tr, table.weather-table tbody tr")?;
    let cell = select("td")?;
    let row = doc
        .select(&row_css)
        .find(|r| r.select(&cell).count() >= 6)
        .ok_or_else(|| Error::format(SERVICE, "the page has no measurement row"))?;
    let cells: Vec<String> = row
        .select(&cell)
        .map(|c| c.text().collect::<Vec<_>>().join("\n"))
        .collect();
    let get = |i: usize| cells.get(i).map_or("", String::as_str);
    let station = select("div.station_info h1")?;
    let hpa = Regex::new(r"(\d+)\s*hPa").map_err(|e| Error::format(SERVICE, e))?;
    let kmh = Regex::new(r"(\d+)\s*km/h").map_err(|e| Error::format(SERVICE, e))?;
    Ok(Current {
        station: doc
            .select(&station)
            .next()
            .map(|h| h.text().collect::<String>().trim().to_owned())
            .unwrap_or_default(),
        source: SERVICE.into(),
        measured_at: None,
        temperature: number(get(1), "Temperatur"),
        dew_point: number(get(1), "Taupunkt"),
        humidity: number(get(1), "Feuchte"),
        condition: word(get(2), "Wetterzustand").unwrap_or_default(),
        visibility_km: number(get(2), "Sichtweite"),
        pressure: hpa.captures(get(4)).and_then(|c| c.get(1)?.as_str().parse().ok()),
        wind_kmh: kmh.captures(get(5)).and_then(|c| c.get(1)?.as_str().parse().ok()),
        ..Current::default()
    })
}

pub async fn current(http: &Http, url: &str) -> Result<Current> {
    parse(&http.text(url).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_measurement_row() {
        let html = r#"<div class="station_info"><h1>Berlin-Tempelhof</h1></div><table id="msgs"><tbody><tr>
            <td>14:00</td><td><div>Temperatur: 9,5 °C
Taupunkt: 7,7 °C
Feuchte: 89 %</div></td><td><div>Wetterzustand: bedeckt
Sichtweite: 19 km</div></td><td>0</td><td>1032 hPa</td><td>7 km/h NW</td></tr></tbody></table>"#;
        let c = parse(html).unwrap();
        assert_eq!(c.station, "Berlin-Tempelhof");
        assert_eq!(c.temperature, Some(9.5));
        assert_eq!(c.humidity, Some(89.0));
        assert_eq!(c.condition, "bedeckt");
        assert_eq!(c.pressure, Some(1032.0));
        assert_eq!(c.wind_kmh, Some(7.0));
    }

    #[test]
    fn a_page_without_table_is_an_error() {
        assert!(parse("<html></html>").is_err());
    }
}
