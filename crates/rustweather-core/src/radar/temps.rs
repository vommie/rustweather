//! Temperatures across the map, all from one grid: the DWD's ICON model for the whole world as numbers, in a single request per hour. Asking a
//! service for every city or point would use up its daily limit within a few pans. City labels and the heatmap only read this grid.

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, DurationRound as _, TimeDelta, Utc};

use super::cities;
use super::layers::MARGIN;
use crate::error::{Error, Result};
use crate::http::Http;
use crate::map::{Extent, View, from_unit};
use crate::store;

const COVERAGE: &str = "https://maps.dwd.de/geoserver/dwd/wcs?service=WCS&version=2.0.1&request=GetCoverage\
    &coverageId=dwd__Icon_reg025_fd_sl_T2M&format=text/plain&subset=Lat(-90,90)&subset=Long(-180,180)";
const TIMEOUT: Duration = Duration::from_secs(90);
/// The screen stretches the heatmap and smooths it, so one pixel in four is plenty.
const HEAT_DENSITY: f64 = 0.25;
/// Values the map server uses for "nothing here" are far outside of any weather.
const LARGEST: f32 = 80.0;
/// Six numbers in front of the values: hour, west, north, step, columns, rows.
const HEADER: usize = 6 * 8;

/// The temperature of one hour on a regular grid of longitude and latitude.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub hour: DateTime<Utc>,
    west: f64,
    north: f64,
    step: f64,
    columns: usize,
    rows: usize,
    /// Row by row from the north.
    values: Vec<f32>,
}

/// A colored picture of the temperature and the stretch of the map it shows.
pub struct Heat {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub extent: Extent,
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

/// The server's text: a header that says how the numbers lie on the world, then the numbers.
fn parse(text: &str, hour: DateTime<Utc>) -> Option<Grid> {
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
    let values: Vec<f32> = after(text, "Band 0:")?
        .split_ascii_whitespace()
        .map(|v| v.parse::<f32>().ok().filter(|v| v.abs() < LARGEST).unwrap_or(f32::NAN))
        .collect();
    (step > 0.0 && (step + down).abs() < 1e-9 && values.len() == columns * rows).then_some(Grid {
        hour,
        west: left + step * x0 as f64,
        north: top + down * y0 as f64,
        step,
        columns,
        rows,
        values,
    })
}

impl Grid {
    /// The temperature at a place, between the four grid points around it.
    pub fn value(&self, lon: f64, lat: f64) -> Option<f32> {
        let fx = ((lon - self.west) / self.step).rem_euclid(self.columns as f64);
        let fy = ((self.north - lat) / self.step).clamp(0.0, (self.rows - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1) % self.columns, (y0 + 1).min(self.rows - 1));
        let (tx, ty) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let at = |x: usize, y: usize| self.values.get(y * self.columns + x).copied().unwrap_or(f32::NAN);
        let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
        let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
        let value = top * (1.0 - ty) + bottom * ty;
        (!value.is_nan()).then_some(value)
    }

    /// The labels of a view: city, pixel position and temperature.
    pub fn labels(&self, view: &View) -> Vec<(&'static str, f32, f32, f32)> {
        cities::visible(view)
            .into_iter()
            .filter_map(|id| {
                let (lon, lat) = cities::place(id)?;
                let (name, x, y) = cities::on_view(id, view)?;
                Some((name, x as f32, y as f32, self.value(lon, lat)?))
            })
            .collect()
    }

    /// The picture for a view, with room around it, so a short pan needs no new one.
    pub fn heat(&self, view: &View) -> Heat {
        let mut extent = view.extent(MARGIN);
        extent.y0 = extent.y0.max(0.0);
        extent.y1 = extent.y1.min(1.0);
        let density = view.scale() * HEAT_DENSITY;
        let width = (((extent.x1 - extent.x0) * density).ceil() as usize).max(2);
        let height = (((extent.y1 - extent.y0) * density).ceil() as usize).max(2);
        let mut rgba = vec![0u8; width * height * 4];
        for (row, line) in rgba.chunks_exact_mut(width * 4).enumerate() {
            let y = extent.y0 + (row as f64 + 0.5) / height as f64 * (extent.y1 - extent.y0);
            let (_, lat) = from_unit(0.5, y);
            for (column, pixel) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let x = extent.x0 + (column as f64 + 0.5) / width as f64 * (extent.x1 - extent.x0);
                if let Some(value) = self.value(x * 360.0 - 180.0, lat) {
                    let color = crate::color::tile(value);
                    *pixel = [color.0, color.1, color.2, 255];
                }
            }
        }
        Heat {
            width: width as u32,
            height: height as u32,
            rgba,
            extent,
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + self.values.len() * 4);
        out.extend(self.hour.timestamp().to_le_bytes());
        out.extend(self.west.to_le_bytes());
        out.extend(self.north.to_le_bytes());
        out.extend(self.step.to_le_bytes());
        out.extend((self.columns as u64).to_le_bytes());
        out.extend((self.rows as u64).to_le_bytes());
        out.extend(self.values.iter().flat_map(|v| v.to_le_bytes()));
        out
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let (head, body) = bytes.split_at_checked(HEADER)?;
        let (fields, _) = head.as_chunks::<8>();
        let float = |index: usize| fields.get(index).map(|b| f64::from_le_bytes(*b));
        let count = |index: usize| usize::try_from(u64::from_le_bytes(*fields.get(index)?)).ok();
        let (columns, rows) = (count(4)?, count(5)?);
        let (cells, _) = body.as_chunks::<4>();
        (cells.len() == columns.checked_mul(rows)? && rows > 0 && columns > 0).then(|| Self {
            hour: DateTime::from_timestamp(i64::from_le_bytes(fields[0]), 0).unwrap_or_default(),
            west: float(1).unwrap_or_default(),
            north: float(2).unwrap_or_default(),
            step: float(3).unwrap_or_default(),
            columns,
            rows,
            values: cells.iter().map(|c| f32::from_le_bytes(*c)).collect(),
        })
    }
}

/// The grid of an hour: from the disk when it is there, otherwise from the server.
pub async fn load(http: &Http, file: &Path, hour: DateTime<Utc>) -> Result<Grid> {
    if let Some(grid) = std::fs::read(file).ok().and_then(|bytes| Grid::from_bytes(&bytes))
        && grid.hour == hour
    {
        return Ok(grid);
    }
    let url = format!(
        "{COVERAGE}&subset=time(%22{}%22)",
        hour.format("%Y-%m-%dT%H:%M:%S.000Z")
    );
    let bytes = http.bytes_within(&url, TIMEOUT).await?;
    let grid = tokio::task::spawn_blocking(move || parse(&String::from_utf8_lossy(&bytes), hour))
        .await
        .ok()
        .flatten()
        .ok_or_else(|| Error::format("DWD", "no temperature grid in the answer"))?;
    if let Err(error) = store::write_atomic(file, &grid.to_bytes()) {
        tracing::debug!(%error, "[Radar] Could not cache the temperature grid");
    }
    Ok(grid)
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

    fn grid() -> Grid {
        parse(SAMPLE, DateTime::UNIX_EPOCH).unwrap()
    }

    #[test]
    fn reads_the_server_text_and_finds_each_value_on_the_map() {
        let grid = grid();
        assert_eq!((grid.columns, grid.rows), (4, 3));
        assert_eq!(grid.value(9.25, 46.75), Some(1.0));
        assert_eq!(grid.value(10.0, 46.5), Some(8.0));
    }

    #[test]
    fn blends_between_grid_points() {
        assert_eq!(grid().value(9.375, 46.75), Some(1.5));
        assert_eq!(grid().value(9.25, 46.625), Some(3.0));
    }

    #[test]
    fn a_gap_in_the_data_stays_a_gap() {
        assert_eq!(grid().value(9.75, 46.25), None);
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
        let grid = grid();
        let back = Grid::from_bytes(&grid.to_bytes()).unwrap();
        assert_eq!(back.values[..4], grid.values[..4]);
        assert_eq!(
            (back.west, back.north, back.columns),
            (grid.west, grid.north, grid.columns)
        );
        assert!(Grid::from_bytes(&[1, 2, 3]).is_none());
    }

    fn world() -> Grid {
        Grid {
            hour: DateTime::UNIX_EPOCH,
            west: -180.0,
            north: 90.0,
            step: 1.0,
            columns: 360,
            rows: 181,
            values: (0..360 * 181).map(|i| (i / 360) as f32 * 0.2 - 18.0).collect(),
        }
    }

    fn view() -> View {
        View {
            lon: 10.0,
            lat: 51.0,
            zoom: 6.0,
            width: 1000,
            height: 700,
        }
    }

    #[test]
    fn the_heatmap_fills_more_than_the_view() {
        let v = view();
        let heat = world().heat(&v);
        let e = v.extent(0.0);
        assert!(heat.extent.x0 < e.x0 && heat.extent.x1 > e.x1 && heat.extent.y0 < e.y0 && heat.extent.y1 > e.y1);
        assert_eq!(heat.rgba.len(), (heat.width * heat.height * 4) as usize);
        assert!(heat.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    }

    #[test]
    fn every_city_of_a_view_gets_its_value() {
        let v = view();
        let labels = world().labels(&v);
        assert_eq!(labels.len(), cities::visible(&v).len());
        assert!(labels.iter().any(|l| l.0 == "Berlin"));
    }

    #[test]
    fn zooming_changes_which_cities_show() {
        let (far, near) = (view(), View { zoom: 8.0, ..view() });
        let names = |v: &View| world().labels(v).iter().map(|l| l.0).collect::<Vec<_>>();
        assert_ne!(names(&far), names(&near));
    }
}
