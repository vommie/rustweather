//! Numbers of the DWD's ICON model for the whole world, as one grid per hour. One request serves every place on earth, so moving the map costs
//! nothing. Asking a service for each point would use up its daily limit within a few pans.

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, DurationRound as _, TimeDelta, Utc};

use crate::error::{Error, Result};
use crate::http::Http;
use crate::store;

const SERVICE: &str = "https://maps.dwd.de/geoserver/dwd/wcs?service=WCS&version=2.0.1&request=GetCoverage&format=text/plain\
    &subset=Lat(-90,90)&subset=Long(-180,180)";
const TIMEOUT: Duration = Duration::from_secs(90);
const TRIES: usize = 4;
const BETWEEN_TRIES: Duration = Duration::from_millis(700);
/// Values the map server uses for "nothing here" are far outside of any weather.
const LARGEST: f32 = 200.0;
/// Seven numbers in front of the values: hour, west, north, step, columns, rows, bands.
const HEADER: usize = 7 * 8;

/// One or more values per point of a regular grid of longitude and latitude.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub hour: DateTime<Utc>,
    west: f64,
    north: f64,
    step: f64,
    columns: usize,
    rows: usize,
    bands: usize,
    /// Band by band, each row by row from the north.
    values: Vec<f32>,
}

/// The hour a grid has to be for.
pub fn hour(now: DateTime<Utc>) -> DateTime<Utc> {
    now.duration_trunc(TimeDelta::hours(1)).unwrap_or(now)
}

fn after<'a>(text: &'a str, mark: &str) -> Option<&'a str> {
    Some(&text[text.find(mark)? + mark.len()..])
}

fn number(text: &str, name: &str) -> Option<f64> {
    let rest = after(text, &format!("\"{name}\","))?;
    rest[..rest.find(']')?].trim().parse().ok()
}

/// The server's text: a header that says how the numbers lie on the world, then the numbers of each band.
fn parse(text: &str, hour: DateTime<Utc>) -> Option<Model> {
    let range = after(text, "GridEnvelope2D[")?;
    let range = &range[..range.find(']')?];
    let (x, y) = range.split_once(',')?;
    let span = |part: &str| -> Option<(i64, i64)> {
        let (low, high) = part.trim().split_once("..")?;
        Some((low.parse().ok()?, high.parse().ok()?))
    };
    let ((x0, x1), (y0, y1)) = (span(x)?, span(y)?);
    let (columns, rows) = (usize::try_from(x1 - x0 + 1).ok()?, usize::try_from(y1 - y0 + 1).ok()?);
    let (step, left) = (number(text, "elt_0_0")?, number(text, "elt_0_2")?);
    let (down, top) = (number(text, "elt_1_1")?, number(text, "elt_1_2")?);
    let body = after(text, "Band 0:")?;
    let values: Vec<f32> = body
        .split_ascii_whitespace()
        .filter(|word| *word != "Band" && !word.ends_with(':'))
        .map(|v| v.parse::<f32>().ok().filter(|v| v.abs() < LARGEST).unwrap_or(f32::NAN))
        .collect();
    let bands = body.matches("Band ").count() + 1;
    (step > 0.0 && (step + down).abs() < 1e-9 && values.len() == columns * rows * bands).then_some(Model {
        hour,
        west: left + step * x0 as f64,
        north: top + down * y0 as f64,
        step,
        columns,
        rows,
        bands,
        values,
    })
}

impl Model {
    /// A band's value at a place, between the four grid points around it.
    pub fn sample(&self, band: usize, lon: f64, lat: f64) -> Option<f32> {
        let fx = ((lon - self.west) / self.step).rem_euclid(self.columns as f64);
        let fy = ((self.north - lat) / self.step).clamp(0.0, (self.rows - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1) % self.columns, (y0 + 1).min(self.rows - 1));
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let start = band * self.columns * self.rows;
        let at = |x: usize, y: usize| {
            self.values
                .get(start + y * self.columns + x)
                .copied()
                .unwrap_or(f32::NAN)
        };
        let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
        let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
        let value = top * (1.0 - ty) + bottom * ty;
        (!value.is_nan()).then_some(value)
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + self.values.len() * 4);
        out.extend(self.hour.timestamp().to_le_bytes());
        out.extend(self.west.to_le_bytes());
        out.extend(self.north.to_le_bytes());
        out.extend(self.step.to_le_bytes());
        for count in [self.columns, self.rows, self.bands] {
            out.extend((count as u64).to_le_bytes());
        }
        out.extend(self.values.iter().flat_map(|v| v.to_le_bytes()));
        out
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let (head, body) = bytes.split_at_checked(HEADER)?;
        let (fields, _) = head.as_chunks::<8>();
        let float = |index: usize| fields.get(index).map(|b| f64::from_le_bytes(*b));
        let count = |index: usize| usize::try_from(u64::from_le_bytes(*fields.get(index)?)).ok();
        let (columns, rows, bands) = (count(4)?, count(5)?, count(6)?);
        let (cells, _) = body.as_chunks::<4>();
        let fits = cells.len() == columns.checked_mul(rows)?.checked_mul(bands)? && rows > 0 && columns > 0;
        fits.then(|| Self {
            hour: DateTime::from_timestamp(i64::from_le_bytes(fields[0]), 0).unwrap_or_default(),
            west: float(1).unwrap_or_default(),
            north: float(2).unwrap_or_default(),
            step: float(3).unwrap_or_default(),
            columns,
            rows,
            bands,
            values: cells.iter().map(|c| f32::from_le_bytes(*c)).collect(),
        })
    }

    /// The grid of an hour: from the disk when it is there, otherwise from the server.
    pub async fn load(http: &Http, file: &Path, coverage: &str, hour: DateTime<Utc>) -> Result<Self> {
        if let Some(model) = std::fs::read(file).ok().and_then(|bytes| Self::from_bytes(&bytes))
            && model.hour == hour
        {
            return Ok(model);
        }
        let url = format!(
            "{SERVICE}&coverageId={coverage}&subset=time(%22{}%22)",
            hour.format("%Y-%m-%dT%H:%M:%S.000Z")
        );
        // Now and then the server answers with a short error page and HTTP 200, and with the grid a moment later.
        for _ in 0..TRIES {
            let bytes = http.bytes_within(&url, TIMEOUT).await?;
            let size = bytes.len();
            let parsed = tokio::task::spawn_blocking(move || parse(&String::from_utf8_lossy(&bytes), hour));
            if let Ok(Some(model)) = parsed.await {
                if let Err(error) = store::write_atomic(file, &model.to_bytes()) {
                    tracing::debug!(%error, "[Radar] Could not cache a model grid");
                }
                return Ok(model);
            }
            tracing::debug!(coverage, size, "[Radar] No model grid in the answer");
            tokio::time::sleep(BETWEEN_TRIES).await;
        }
        Err(Error::format("DWD", "no model grid in the answer"))
    }
}

#[cfg(test)]
pub fn uniform(columns: usize, rows: usize, values: Vec<f32>) -> Model {
    Model {
        hour: DateTime::UNIX_EPOCH,
        west: -180.0,
        north: 90.0,
        step: 1.0,
        columns,
        rows,
        bands: values.len() / (columns * rows),
        values,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"Grid bounds: GeneralBounds[(9.125, 44.875), (10.125, 45.875)]
Grid range: GridEnvelope2D[757..760, 173..175]
Grid to world: PARAM_MT["Affine",
  PARAMETER["num_row", 3],
  PARAMETER["elt_0_0", 0.25],
  PARAMETER["elt_0_2", -180.0],
  PARAMETER["elt_1_1", -0.25],
  PARAMETER["elt_1_2", 90.0]]
Contents:
Band 0:
1.0 2.0 3.0 4.0
5.0 6.0 7.0 8.0
9.0 10.0 -9999.0 12.0
"#;

    fn model() -> Model {
        parse(SAMPLE, DateTime::UNIX_EPOCH).unwrap()
    }

    #[test]
    fn reads_the_server_text_and_finds_each_value_on_the_map() {
        let model = model();
        assert_eq!((model.columns, model.rows, model.bands), (4, 3, 1));
        assert_eq!(model.sample(0, 9.25, 46.75), Some(1.0));
        assert_eq!(model.sample(0, 10.0, 46.5), Some(8.0));
    }

    #[test]
    fn reads_two_bands() {
        let text = format!("{SAMPLE}Band 1:\n10 20 30 40\n50 60 70 80\n90 100 110 120\n");
        let model = parse(&text, DateTime::UNIX_EPOCH).unwrap();
        assert_eq!(model.bands, 2);
        assert_eq!(model.sample(1, 9.25, 46.75), Some(10.0));
        assert_eq!(model.sample(0, 9.25, 46.75), Some(1.0));
    }

    #[test]
    fn blends_between_grid_points() {
        assert_eq!(model().sample(0, 9.375, 46.75), Some(1.5));
        assert_eq!(model().sample(0, 9.25, 46.625), Some(3.0));
    }

    #[test]
    fn a_gap_in_the_data_stays_a_gap() {
        assert_eq!(model().sample(0, 9.75, 46.25), None);
    }

    #[test]
    fn an_error_page_is_no_grid() {
        assert!(
            parse(
                "<ows:ExceptionReport>nothing</ows:ExceptionReport>",
                DateTime::UNIX_EPOCH
            )
            .is_none()
        );
    }

    #[test]
    fn the_disk_gives_back_the_same_grid() {
        let model = model();
        let back = Model::from_bytes(&model.to_bytes()).unwrap();
        assert_eq!(back.values[..4], model.values[..4]);
        assert_eq!(
            (back.west, back.north, back.columns, back.bands),
            (model.west, model.north, 4, 1)
        );
        assert!(Model::from_bytes(&[1, 2, 3]).is_none());
    }
}
