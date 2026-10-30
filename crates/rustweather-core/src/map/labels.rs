//! City labels of the app's own map, drawn by the UI at the positions computed here.

use super::View;

/// Name, longitude, latitude, the zoom from which it shows.
const CITIES: &[(&str, f64, f64, f64)] = &[
    ("Berlin", 13.40, 52.52, 0.0),
    ("Hamburg", 10.00, 53.55, 0.0),
    ("Munich", 11.58, 48.14, 0.0),
    ("Cologne", 6.96, 50.94, 0.0),
    ("Frankfurt", 8.68, 50.11, 0.0),
    ("Stuttgart", 9.18, 48.78, 5.0),
    ("Leipzig", 12.37, 51.34, 5.0),
    ("Dresden", 13.74, 51.05, 5.0),
    ("Hanover", 9.73, 52.37, 5.0),
    ("Bremen", 8.80, 53.08, 5.0),
    ("Nuremberg", 11.08, 49.45, 5.0),
    ("Rostock", 12.13, 54.09, 5.0),
    ("Kiel", 10.13, 54.32, 6.0),
    ("Erfurt", 11.03, 50.98, 6.0),
    ("Magdeburg", 11.63, 52.13, 6.0),
    ("Potsdam", 13.06, 52.40, 7.0),
    ("Vienna", 16.37, 48.21, 0.0),
    ("Zurich", 8.54, 47.38, 0.0),
    ("Prague", 14.42, 50.09, 0.0),
    ("Warsaw", 21.01, 52.23, 0.0),
    ("Amsterdam", 4.90, 52.37, 0.0),
    ("Brussels", 4.35, 50.85, 0.0),
    ("Paris", 2.35, 48.86, 0.0),
    ("London", -0.13, 51.51, 0.0),
    ("Copenhagen", 12.57, 55.68, 0.0),
    ("Stockholm", 18.07, 59.33, 0.0),
    ("Oslo", 10.75, 59.91, 0.0),
    ("Rome", 12.50, 41.90, 0.0),
    ("Madrid", -3.70, 40.42, 0.0),
    ("Lisbon", -9.14, 38.72, 0.0),
    ("Athens", 23.73, 37.98, 0.0),
    ("Budapest", 19.04, 47.50, 0.0),
];

pub fn visible(view: &View) -> Vec<(String, f32, f32)> {
    CITIES
        .iter()
        .filter(|c| view.zoom >= c.3)
        .filter_map(|&(name, lon, lat, _)| {
            let (x, y) = view.pixel_of(lon, lat);
            let inside =
                x > -40.0 && y > -10.0 && x < f64::from(view.width) + 40.0 && y < f64::from(view.height) + 10.0;
            inside.then(|| (name.to_owned(), x as f32, y as f32))
        })
        .collect()
}
