//! The maps to choose from. All but CARTO work without a key. Maps without place names get them as a layer of their own.

use super::tiles::{Layer, Post};
use crate::settings::Basemap;

const ESRI: &str = "https://server.arcgisonline.com/ArcGIS/rest/services";
const ESRI_CREDIT: &str = "Esri and data partners";
const OSM_CREDIT: &str = "© OpenStreetMap contributors";
const PLACES: &str = "Reference/World_Boundaries_and_Places";

fn layer(id: String, url: String, max_zoom: i32) -> Layer {
    Layer {
        post: Post::None,
        id,
        url,
        tile_size: 256,
        max_zoom,
    }
}

fn esri(service: &str, max_zoom: i32) -> Layer {
    layer(
        format!("esri-{}", service.replace('/', "-").to_lowercase()),
        format!("{ESRI}/{service}/MapServer/tile/{{z}}/{{y}}/{{x}}"),
        max_zoom,
    )
}

fn plain(id: &str, url: &str, max_zoom: i32) -> Layer {
    layer(id.to_owned(), url.to_owned(), max_zoom)
}

/// The provider's URL for a map layer; the key stays out of `id`, which names cache folders.
fn carto(style: &str, key: &str) -> Layer {
    layer(
        format!("carto-{style}"),
        format!("https://basemaps.cartocdn.com/rastertiles/{style}/{{z}}/{{x}}/{{y}}.png?key={key}"),
        19,
    )
}

impl Basemap {
    pub const ALL: [Self; 12] = [
        Self::Own,
        Self::Osm,
        Self::Satellite,
        Self::Sentinel,
        Self::Topographic,
        Self::Terrain,
        Self::Relief,
        Self::TopPlus,
        Self::LightGray,
        Self::DarkGray,
        Self::CartoDark,
        Self::CartoVoyager,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Own => "Standard map",
            Self::Osm => "OpenStreetMap",
            Self::Satellite => "Satellite",
            Self::Sentinel => "Satellite without clouds",
            Self::Topographic => "Topographic",
            Self::Terrain => "Streets and terrain",
            Self::Relief => "Shaded relief",
            Self::TopPlus => "TopPlus",
            Self::LightGray => "Light gray",
            Self::DarkGray => "Dark gray",
            Self::CartoDark => "CARTO dark",
            Self::CartoVoyager => "CARTO Voyager",
        }
    }

    /// What the map's license asks to show next to it.
    pub fn credit(self) -> &'static str {
        match self {
            Self::Own => "",
            Self::Osm => OSM_CREDIT,
            Self::Satellite | Self::Terrain | Self::Relief | Self::LightGray | Self::DarkGray => ESRI_CREDIT,
            Self::Sentinel => "Sentinel-2 cloudless 2023 by EOX, Copernicus data, labels Esri",
            Self::Topographic => "© OpenStreetMap contributors, SRTM, © OpenTopoMap (CC-BY-SA)",
            Self::TopPlus => "TopPlusOpen © BKG",
            Self::CartoDark | Self::CartoVoyager => "© CARTO, © OpenStreetMap contributors",
        }
    }

    pub fn needs_key(self) -> bool {
        matches!(self, Self::CartoDark | Self::CartoVoyager)
    }

    /// Base layer, and a layer of place names to lay over the rain where the map has one. No base layer means the own map.
    pub fn layers(self, carto_key: Option<&str>, split: bool) -> (Option<Layer>, Option<Layer>) {
        let esri_gray = |shade: &str| {
            (
                Some(esri(&format!("Canvas/World_{shade}_Gray_Base"), 16)),
                Some(esri(&format!("Canvas/World_{shade}_Gray_Reference"), 16)),
            )
        };
        match (self, carto_key) {
            (Self::Own, _) | (Self::CartoDark | Self::CartoVoyager, None) => (None, None),
            (Self::Osm, _) => (
                Some(plain("osm", "https://tile.openstreetmap.org/{z}/{x}/{y}.png", 19)),
                None,
            ),
            (Self::Satellite, _) => (Some(esri("World_Imagery", 18)), Some(esri(PLACES, 15))),
            (Self::Sentinel, _) => (
                Some(plain(
                    "eox-s2cloudless-2023",
                    "https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless-2023_3857/default/g/{z}/{y}/{x}.jpg",
                    15,
                )),
                Some(esri(PLACES, 15)),
            ),
            (Self::Topographic, _) => (
                Some(plain("opentopomap", "https://tile.opentopomap.org/{z}/{x}/{y}.png", 17)),
                None,
            ),
            (Self::Terrain, _) => (Some(esri("World_Topo_Map", 18)), None),
            (Self::Relief, _) => (Some(esri("World_Shaded_Relief", 13)), Some(esri(PLACES, 15))),
            (Self::TopPlus, _) => (
                Some(plain(
                    "topplus",
                    "https://sgx.geodatenzentrum.de/wmts_topplus_open/tile/1.0.0/web/default/WEBMERCATOR/{z}/{y}/{x}.png",
                    18,
                )),
                None,
            ),
            (Self::LightGray, _) => esri_gray("Light"),
            (Self::DarkGray, _) => esri_gray("Dark"),
            (Self::CartoDark, Some(key)) if split => {
                (Some(carto("dark_nolabels", key)), Some(carto("dark_only_labels", key)))
            }
            (Self::CartoDark, Some(key)) => (Some(carto("dark_all", key)), None),
            (Self::CartoVoyager, Some(key)) if split => (
                Some(carto("voyager_nolabels", key)),
                Some(carto("voyager_only_labels", key)),
            ),
            (Self::CartoVoyager, Some(key)) => (Some(carto("voyager", key)), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_map_but_the_own_one_has_a_layer_and_a_credit() {
        for map in Basemap::ALL {
            let (base, _) = map.layers(Some("k"), true);
            assert_eq!(base.is_none(), map == Basemap::Own, "{}", map.name());
            assert_eq!(map.credit().is_empty(), map == Basemap::Own, "{}", map.name());
        }
    }

    #[test]
    fn carto_stands_back_without_a_key() {
        assert_eq!(Basemap::CartoDark.layers(None, true), (None, None));
    }

    #[test]
    fn a_key_never_lands_in_a_cache_folder_name() {
        for map in Basemap::ALL {
            let (base, labels) = map.layers(Some("secret"), true);
            for layer in [base, labels].into_iter().flatten() {
                assert!(!layer.id.contains("secret"));
            }
        }
    }

    #[test]
    fn cache_folders_are_all_different() {
        let mut ids: Vec<String> = Basemap::ALL
            .into_iter()
            .filter_map(|map| map.layers(Some("k"), true).0.map(|layer| layer.id))
            .collect();
        ids.sort();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}
