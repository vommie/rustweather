//! Temperatures across the map, all from one grid of the DWD's model (`model.rs`). City labels and the heatmap only read it.

use chrono::{DateTime, Utc};

use super::cities;
use super::layers::MARGIN;
use super::model::Model;
use crate::error::Result;
use crate::http::Http;
use crate::map::{Extent, View, from_unit};

const COVERAGE: &str = "dwd__Icon_reg025_fd_sl_T2M";
/// The screen stretches the heatmap and smooths it, so one pixel in four is plenty.
const HEAT_DENSITY: f64 = 0.25;

/// A colored picture of the temperature and the stretch of the map it shows.
pub struct Heat {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub extent: Extent,
}

impl Model {
    /// The labels of a view: city, pixel position and temperature.
    pub fn labels(&self, view: &View) -> Vec<(&'static str, f32, f32, f32)> {
        cities::visible(view)
            .into_iter()
            .filter_map(|id| {
                let (lon, lat) = cities::place(id)?;
                let (name, x, y) = cities::on_view(id, view)?;
                Some((name, x as f32, y as f32, self.sample(0, lon, lat)?))
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
                if let Some(value) = self.sample(0, x * 360.0 - 180.0, lat) {
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
}

/// The temperature grid of an hour.
pub async fn load(http: &Http, file: &std::path::Path, hour: DateTime<Utc>) -> Result<Model> {
    Model::load(http, file, COVERAGE, hour).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> Model {
        let values = (0..360 * 181).map(|i| (i / 360) as f32 * 0.2 - 18.0).collect();
        crate::radar::model::uniform(360, 181, values)
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
