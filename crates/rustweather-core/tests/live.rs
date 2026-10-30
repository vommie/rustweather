//! Checks against the real services. Run on purpose only: `cargo test -p rustweather-core --test live -- --ignored`.

use rustweather_core::http::Http;
use rustweather_core::radar::dwd::Projector;
use rustweather_core::settings::Place;
use rustweather_core::sources::{brightsky, geocode, openmeteo};
use serde_json::Value;

fn place() -> Place {
    Place::default()
}

#[tokio::test]
#[ignore = "needs the network"]
async fn projection_agrees_with_bright_sky() {
    let http = Http::new().unwrap();
    let projector = Projector::new().unwrap();
    for (lat, lon) in [(52.52, 13.405), (48.14, 11.58), (53.55, 10.0), (50.94, 6.96)] {
        let url = format!("https://api.brightsky.dev/radar?lat={lat}&lon={lon}&distance=2000&format=plain");
        let answer: Value = http.json(&url).await.unwrap();
        let bbox = &answer["bbox"];
        let bx = bbox[1].as_f64().unwrap() + answer["latlon_position"]["x"].as_f64().unwrap();
        let by = bbox[0].as_f64().unwrap() + answer["latlon_position"]["y"].as_f64().unwrap();
        let (x, y) = projector.project(lon, lat).unwrap();
        println!("{lat},{lon}: bright sky {bx:.2},{by:.2} ours {x:.2},{y:.2}");
        assert!(
            (f64::from(x) - bx).abs() < 1.5 && (f64::from(y) - by).abs() < 1.5,
            "{bx},{by} vs {x},{y}"
        );
    }
}

#[tokio::test]
#[ignore = "needs the network"]
async fn wind_grid_answers() {
    let http = Http::new().unwrap();
    let view = rustweather_core::map::View {
        lon: 10.0,
        lat: 51.0,
        zoom: 5.6,
        width: 1000,
        height: 700,
    };
    let field = rustweather_core::radar::wind::fetch(&http, &view).await.unwrap();
    assert!(field.covers(&view));
    println!("wind at Berlin: {:?}", field.at(13.4, 52.5));
    assert!(field.at(13.4, 52.5).is_some());
}

#[tokio::test]
#[ignore = "needs the network"]
async fn sources_answer() {
    let http = Http::new().unwrap();
    let current = brightsky::current(&http, &place()).await.unwrap();
    assert!(current.temperature.is_some());
    println!("{current:?}");
    let forecast = openmeteo::forecast(&http, &place()).await.unwrap();
    assert!(forecast.hours.len() > 100 && forecast.days.len() >= 7);
    println!(
        "feels {:?} uv {:?} aqi {:?} pollen {:?} trend {:?}",
        forecast.feels_like,
        forecast.uv,
        forecast.air_quality,
        forecast.pollen,
        forecast.trend(9.0)
    );
    println!("{:?}", brightsky::alerts(&http, &place()).await.unwrap());
    let series = rustweather_core::radar::outlook::series(&http, &place()).await.unwrap();
    assert!(!series.is_empty());
    println!(
        "{:?}",
        rustweather_core::radar::outlook::read(&series, chrono::Utc::now())
    );
    assert!(!geocode::search(&http, "Berlin").await.unwrap().is_empty());
}
