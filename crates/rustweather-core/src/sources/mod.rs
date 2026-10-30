//! The weather services. Each module turns one service's answers into `crate::model`.

pub mod brightsky;
pub mod geocode;
pub mod openmeteo;
pub mod wetterdienst;

use crate::error::Result;
use crate::http::Http;
use crate::model::Current;
use crate::settings::Place;

/// Current weather from the DWD via Bright Sky, with the wetterdienst.de page as the fallback.
pub async fn current(http: &Http, place: &Place, station_url: &str) -> Result<Current> {
    match brightsky::current(http, place).await {
        Ok(current) => Ok(current),
        Err(primary) if !station_url.is_empty() => {
            tracing::warn!(error = %primary, "[Sources] Bright Sky failed, trying wetterdienst.de");
            wetterdienst::current(http, station_url).await.map_err(|_| primary)
        }
        Err(primary) => Err(primary),
    }
}
