use crate::exif_geo::Coordinates;

/// Clickable map links for a coordinate pair.
#[derive(Clone, Debug, PartialEq)]
pub struct MapLinks {
    /// Google Maps search URL with a pin.
    pub google_maps: String,
    /// Bing Maps URL centered on the point with a pin.
    pub bing_maps: String,
    /// Apple Maps URL centered on the point with a label.
    pub apple_maps: String,
    /// 2GIS web URL for the point.
    pub two_gis: String,
    /// Mapillary web app URL centered on nearby street-level imagery.
    pub mapillary: String,
    /// Yandex Maps URL centered on the point with a pin.
    pub yandex_maps: String,
    /// Wikimapia URL centered on the point.
    pub wikimapia: String,
    /// OpenStreetMap URL with a marker.
    pub openstreetmap: String,
    /// Google Earth web URL for the point.
    pub google_earth: String,
    /// GeoHack URL with camera-location parameters.
    pub geohack: String,
    /// Wikimedia Toolforge WikiMap URL centered on the point.
    pub wikimap_toolforge: String,
}

impl MapLinks {
    /// Builds map links using documented or commonly supported coordinate URL formats.
    pub fn for_coordinates(coordinates: Coordinates, file_name: Option<&str>) -> Self {
        let latitude = decimal(coordinates.latitude);
        let longitude = decimal(coordinates.longitude);
        let encoded_pair = format!("{latitude}%2C{longitude}");
        let bing_title = urlencoding::encode("Photo location");
        let geohack_file = file_name
            .map(clean_file_name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Photo.jpg".to_string());

        Self {
            google_maps: format!("https://www.google.com/maps/search/?api=1&query={encoded_pair}"),
            bing_maps: format!(
                "https://bing.com/maps/default.aspx?cp={latitude}~{longitude}&lvl=18&style=r&sp=point.{latitude}_{longitude}_{bing_title}"
            ),
            apple_maps: format!(
                "https://maps.apple.com/?ll={latitude},{longitude}&q=Photo%20location&z=18"
            ),
            two_gis: format!("https://2gis.com/geo/{longitude}%2C{latitude}"),
            mapillary: format!(
                "https://www.mapillary.com/app/?lat={latitude}&lng={longitude}&z=17"
            ),
            yandex_maps: format!(
                "https://yandex.com/maps/?ll={longitude}%2C{latitude}&z=18&pt={longitude}%2C{latitude},pm2rdm"
            ),
            wikimapia: format!(
                "https://wikimapia.org/#lat={latitude}&lon={longitude}&z=18&l=0&m=w"
            ),
            openstreetmap: format!(
                "https://www.openstreetmap.org/?mlat={latitude}&mlon={longitude}#map=18/{latitude}/{longitude}"
            ),
            google_earth: format!("https://earth.google.com/web/search/{latitude},{longitude}"),
            geohack: geohack_url(coordinates, &geohack_file),
            wikimap_toolforge: format!(
                "https://wikimap.toolforge.org/?lat={latitude}&lon={longitude}&zoom=18&lang=en&camera=true"
            ),
        }
    }
}

/// Formats coordinates compactly while preserving sub-meter-ish precision.
pub fn decimal(value: f64) -> String {
    let formatted = format!("{value:.7}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn clean_file_name(file_name: &str) -> String {
    file_name
        .chars()
        .filter(|character| !character.is_control() && *character != '/' && *character != '\\')
        .collect::<String>()
        .trim()
        .to_string()
}

fn geohack_url(coordinates: Coordinates, file_name: &str) -> String {
    let latitude_ref = if coordinates.latitude < 0.0 { "S" } else { "N" };
    let longitude_ref = if coordinates.longitude < 0.0 {
        "W"
    } else {
        "E"
    };
    let latitude = format!("{:010.6}", coordinates.latitude.abs());
    let longitude = format!("{:011.6}", coordinates.longitude.abs());
    let pagename_value = format!("File:{file_name}");
    let pagename = urlencoding::encode(&pagename_value);

    format!(
        "https://geohack.toolforge.org/geohack.php?pagename={pagename}&params={latitude}_{latitude_ref}_{longitude}_{longitude_ref}_globe:Earth_type:camera__&language=en"
    )
}

#[cfg(test)]
mod tests {
    use super::{MapLinks, decimal};
    use crate::exif_geo::Coordinates;

    #[test]
    fn trims_coordinate_decimals() {
        assert_eq!(decimal(41.7000000), "41.7");
        assert_eq!(decimal(-122.3316393), "-122.3316393");
    }

    #[test]
    fn builds_all_map_links() {
        let links = MapLinks::for_coordinates(
            Coordinates {
                latitude: 47.5951518,
                longitude: -122.3316393,
            },
            Some("Lumen Field.jpg"),
        );

        assert_eq!(
            links.google_maps,
            "https://www.google.com/maps/search/?api=1&query=47.5951518%2C-122.3316393"
        );
        assert!(links.bing_maps.contains("cp=47.5951518~-122.3316393"));
        assert!(
            links
                .bing_maps
                .contains("sp=point.47.5951518_-122.3316393_Photo%20location")
        );
        assert_eq!(
            links.apple_maps,
            "https://maps.apple.com/?ll=47.5951518,-122.3316393&q=Photo%20location&z=18"
        );
        assert_eq!(
            links.two_gis,
            "https://2gis.com/geo/-122.3316393%2C47.5951518"
        );
        assert_eq!(
            links.mapillary,
            "https://www.mapillary.com/app/?lat=47.5951518&lng=-122.3316393&z=17"
        );
        assert_eq!(
            links.yandex_maps,
            "https://yandex.com/maps/?ll=-122.3316393%2C47.5951518&z=18&pt=-122.3316393%2C47.5951518,pm2rdm"
        );
        assert_eq!(
            links.wikimapia,
            "https://wikimapia.org/#lat=47.5951518&lon=-122.3316393&z=18&l=0&m=w"
        );
        assert_eq!(
            links.openstreetmap,
            "https://www.openstreetmap.org/?mlat=47.5951518&mlon=-122.3316393#map=18/47.5951518/-122.3316393"
        );
        assert_eq!(
            links.google_earth,
            "https://earth.google.com/web/search/47.5951518,-122.3316393"
        );
        assert_eq!(
            links.geohack,
            "https://geohack.toolforge.org/geohack.php?pagename=File%3ALumen%20Field.jpg&params=047.595152_N_0122.331639_W_globe:Earth_type:camera__&language=en"
        );
        assert_eq!(
            links.wikimap_toolforge,
            "https://wikimap.toolforge.org/?lat=47.5951518&lon=-122.3316393&zoom=18&lang=en&camera=true"
        );
    }
}
