use crate::config::Config;
use crate::exif_geo::{Coordinates, ExifSummary, extract_image_location};
use crate::geocoder::Geocoder;
use crate::links::{MapLinks, decimal};
use crate::telegram::{Message, TelegramClient, Update};
use crate::wikimedia::{NearbyWikimedia, WikimediaClient};
use anyhow::Context;
use lambda_http::http::{Method, StatusCode};
use lambda_http::{Body, Request, Response};
use std::time::Duration;
use tracing::{error, warn};

const SEND_AS_FILE_TEXT: &str = "Please send the image as a file/document, not as a Telegram photo. Telegram photos are compressed and often have EXIF GPS metadata removed.";

/// Lambda HTTP application.
#[derive(Clone)]
pub struct App {
    config: Config,
    telegram: TelegramClient,
    geocoder: Option<Geocoder>,
    wikimedia: Option<WikimediaClient>,
}

impl App {
    /// Creates a Lambda app with shared HTTP clients.
    pub fn new(config: Config) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("reqwest client config must be valid");
        let telegram = TelegramClient::new(http_client.clone(), config.telegram_bot_token.clone());
        let geocoder = config.enable_reverse_geocoding.then(|| {
            Geocoder::new(
                http_client,
                config.nominatim_base_url.clone(),
                config.nominatim_user_agent.clone(),
                config.nominatim_email.clone(),
                config.nominatim_accept_language.clone(),
            )
        });
        let wikimedia = config.enable_wikimedia_lookup.then(|| {
            WikimediaClient::new(
                reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .build()
                    .expect("reqwest client config must be valid"),
                config.wikimedia_user_agent.clone(),
                config.wikipedia_languages.clone(),
                config.wikipedia_api_url.clone(),
                config.wikidata_sparql_url.clone(),
                config.wikimedia_radius_meters,
                config.wikimedia_limit,
            )
        });

        Self {
            config,
            telegram,
            geocoder,
            wikimedia,
        }
    }

    /// Handles Lambda Function URL requests.
    pub async fn handle_http(
        &self,
        request: Request,
    ) -> Result<Response<Body>, lambda_http::Error> {
        if request.method() == Method::GET {
            return response(StatusCode::OK, "ok");
        }
        if request.method() != Method::POST {
            return response(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
        }
        if !self.valid_secret(&request) {
            return response(StatusCode::UNAUTHORIZED, "unauthorized");
        }

        let update = serde_json::from_slice::<Update>(body_bytes(request.body()))
            .context("failed to parse Telegram webhook body")?;
        if let Err(error) = self.handle_update(update).await {
            error!(?error, "failed to handle Telegram update");
        }

        response(StatusCode::OK, "ok")
    }

    async fn handle_update(&self, update: Update) -> anyhow::Result<()> {
        let Some(message) = update.message else {
            return Ok(());
        };

        self.send_typing_action(&message).await;

        if message.document.is_some() {
            return self.handle_document(message).await;
        }

        if message.location.is_some() {
            return self.handle_location(message).await;
        }

        if !message.photo.is_empty() {
            return self.reply(&message, SEND_AS_FILE_TEXT).await;
        }

        if message
            .text
            .as_deref()
            .is_some_and(|text| text == "/start" || text == "/help")
        {
            return self
                .reply(
                    &message,
                    &help_text(&self.config.project_name, &self.config.github_url),
                )
                .await;
        }
        self.reply(
			&message,
			"Send me an original image as a file/document or share a Telegram location, and I will return coordinates, map links, and a place name when available. For images, I will also send a native Telegram map pin.",
		)
		.await
    }

    async fn handle_document(&self, message: Message) -> anyhow::Result<()> {
        let document = message
            .document
            .as_ref()
            .expect("checked document presence");
        if !document.looks_like_image() {
            return self
				.reply(
					&message,
					"This document does not look like an image. Please send an original JPEG/TIFF/DNG/HEIC image file.",
				)
				.await;
        }
        if document
            .file_size
            .or(Some(0))
            .is_some_and(|size| size > self.config.max_file_bytes)
        {
            return self
                .reply(&message, "This file is too large for this bot to download.")
                .await;
        }

        let telegram_file = self.telegram.get_file(&document.file_id).await?;
        if telegram_file
            .file_size
            .or(Some(0))
            .is_some_and(|size| size > self.config.max_file_bytes)
        {
            return self
                .reply(&message, "This file is too large for this bot to download.")
                .await;
        }

        let bytes = self
            .telegram
            .download_file(&telegram_file.file_path)
            .await?;
        let Some(image_location) = extract_image_location(&bytes) else {
            return self
				.reply(
					&message,
					"I could not find GPS coordinates in the image EXIF metadata. Make sure geotagging was enabled and send the original image as a file/document.",
				)
				.await;
        };

        self.reply_for_coordinates(
            &message,
            image_location.coordinates,
            document.file_name.as_deref(),
            "Location found in EXIF metadata.",
            Some(&image_location.exif),
            true,
        )
        .await
    }

    async fn handle_location(&self, message: Message) -> anyhow::Result<()> {
        let location = message.location.expect("checked location presence");
        let Some(coordinates) = Coordinates::new(location.latitude, location.longitude) else {
            return self
                .reply(
                    &message,
                    "Telegram sent invalid coordinates in this location.",
                )
                .await;
        };

        self.reply_for_coordinates(
            &message,
            coordinates,
            None,
            "Location received from Telegram.",
            None,
            false,
        )
        .await
    }

    async fn reply_for_coordinates(
        &self,
        message: &Message,
        coordinates: Coordinates,
        file_name: Option<&str>,
        intro: &str,
        exif: Option<&ExifSummary>,
        send_native_pin: bool,
    ) -> anyhow::Result<()> {
        let city = self.reverse_city(coordinates).await;
        let nearby = self.nearby_wikimedia(coordinates).await;
        let links = MapLinks::for_coordinates(coordinates, file_name);
        let reply = location_reply_text(
            intro,
            coordinates,
            city.as_deref(),
            exif,
            &links,
            nearby.as_ref(),
        );
        self.reply(message, &reply).await?;

        if send_native_pin
            && let Err(error) = self
                .telegram
                .send_location(
                    message.chat.id,
                    message.message_id,
                    coordinates.latitude,
                    coordinates.longitude,
                )
                .await
        {
            warn!(?error, "failed to send native Telegram location");
        }

        Ok(())
    }

    async fn reverse_city(&self, coordinates: Coordinates) -> Option<String> {
        let Some(geocoder) = &self.geocoder else {
            return None;
        };
        match geocoder.reverse_city(coordinates).await {
            Ok(city) => city,
            Err(error) => {
                warn!(?error, "reverse geocoding failed");
                None
            }
        }
    }

    async fn nearby_wikimedia(&self, coordinates: Coordinates) -> Option<NearbyWikimedia> {
        let Some(wikimedia) = &self.wikimedia else {
            return None;
        };
        let nearby = wikimedia.nearby(coordinates).await;
        (!nearby.is_empty()).then_some(nearby)
    }

    async fn reply(&self, message: &Message, text: &str) -> anyhow::Result<()> {
        self.telegram
            .send_message(message.chat.id, message.message_id, text)
            .await
    }

    async fn send_typing_action(&self, message: &Message) {
        if let Err(error) = self.telegram.send_typing_action(message.chat.id).await {
            warn!(?error, "failed to send Telegram typing action");
        }
    }

    fn valid_secret(&self, request: &Request) -> bool {
        let Some(secret) = &self.config.telegram_webhook_secret else {
            return true;
        };
        request
            .headers()
            .get("x-telegram-bot-api-secret-token")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value == secret)
    }
}

/// Formats the user-facing location reply.
pub fn location_reply_text(
    intro: &str,
    coordinates: Coordinates,
    city: Option<&str>,
    exif: Option<&ExifSummary>,
    links: &MapLinks,
    nearby: Option<&NearbyWikimedia>,
) -> String {
    let mut lines = vec![
        intro.to_string(),
        format!(
            "Coordinates: {}, {}",
            decimal(coordinates.latitude),
            decimal(coordinates.longitude)
        ),
    ];

    if let Some(city) = city {
        lines.push(format!("Place: {city}"));
        lines.push("Place lookup: OpenStreetMap Nominatim".to_string());
    }

    if let Some(exif) = exif.filter(|exif| !exif.is_empty()) {
        append_exif_summary(&mut lines, exif);
    }

    append_link_block(&mut lines, "Google Maps", &links.google_maps);
    append_link_block(&mut lines, "Apple Maps", &links.apple_maps);
    append_link_block(&mut lines, "Bing Maps", &links.bing_maps);
    append_link_block(&mut lines, "Google Earth", &links.google_earth);
    append_link_block(&mut lines, "Yandex Maps", &links.yandex_maps);
    append_link_block(&mut lines, "2GIS", &links.two_gis);
    append_link_block(&mut lines, "Mapillary", &links.mapillary);
    append_link_block(&mut lines, "OpenStreetMap", &links.openstreetmap);
    append_link_block(&mut lines, "Wikimapia", &links.wikimapia);
    append_link_block(&mut lines, "GeoHack", &links.geohack);
    append_link_block(&mut lines, "Wikimap Toolforge", &links.wikimap_toolforge);

    if let Some(nearby) = nearby {
        if !nearby.wikidata_items.is_empty() {
            lines.push(String::new());
            lines.push("Nearest Wikidata items (from this point):".to_string());
            for item in &nearby.wikidata_items {
                lines.push(format!(
                    "- {} ({}, {}): {}",
                    item.label,
                    item.id,
                    distance_km(item.distance_km, item.direction.as_deref()),
                    item.url
                ));
            }
        }
        if !nearby.wikipedia_articles.is_empty() {
            lines.push(String::new());
            lines.push("Nearest Wikipedia articles (from this point):".to_string());
            for article in &nearby.wikipedia_articles {
                lines.push(format!(
                    "- {} [{}] ({}) {}",
                    article.title,
                    article.language,
                    distance_meters(article.distance_meters, article.direction.as_deref()),
                    article.url
                ));
            }
        }
    }

    lines.join("\n")
}

fn append_link_block(lines: &mut Vec<String>, provider: &str, url: &str) {
    lines.push(String::new());
    lines.push(provider.to_string());
    lines.push(url.to_string());
}

fn help_text(project_name: &str, github_url: &str) -> String {
    format!(
        "{project_name}\n\nSend an original geotagged image as a file/document or share a Telegram location. If you send an image as a normal Telegram photo, Telegram may remove the EXIF GPS metadata.\n\nRepo: {github_url}\nSupport: @vitaly_zdanevich\n\nPrivacy: this bot runs on stateless AWS Lambda. It processes each image in memory, extracts EXIF GPS metadata, replies, and does not store photos. Telegram, AWS Lambda logs, and external lookup services may still receive request metadata.\n\nCommands: /help"
    )
}

fn append_exif_summary(lines: &mut Vec<String>, exif: &ExifSummary) {
    lines.push(String::new());
    lines.push("EXIF:".to_string());

    if exif.camera_make.is_some() || exif.camera_model.is_some() {
        let camera = [exif.camera_make.as_deref(), exif.camera_model.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(url) = &exif.camera_wikipedia_url {
            lines.push(format!("Camera: {camera}"));
            lines.push(url.to_string());
        } else {
            lines.push(format!("Camera: {camera}"));
        }
    }
    if let Some(value) = &exif.captured_at {
        lines.push(format!("Captured: {value}"));
    }
    if let Some(value) = &exif.lens_model {
        lines.push(format!("Lens: {value}"));
    }
    if let Some(value) = exif.focal_length_mm {
        lines.push(format!("Focal length: {} mm", short_number(value)));
    }
    if let Some(value) = exif.focal_length_35mm {
        lines.push(format!("Focal length 35mm: {value} mm"));
    }
    if let Some(value) = exif.f_number {
        lines.push(format!("Aperture: f/{}", short_number(value)));
    }
    if let Some(value) = &exif.exposure_time {
        lines.push(format!("Exposure: {value}"));
    }
    if let Some(value) = exif.iso {
        lines.push(format!("ISO: {value}"));
    }
    if let Some(value) = exif.altitude_meters {
        lines.push(format!("Altitude: {} m", short_number(value)));
    }
    if let Some(value) = exif.image_direction_degrees {
        lines.push(format!("Image direction: {} deg", short_number(value)));
    }
    if let Some(value) = &exif.gps_speed {
        lines.push(format!("GPS speed: {value}"));
    }
    if let Some(value) = &exif.software {
        lines.push(format!("Software: {value}"));
    }
}

fn distance_km(distance: Option<f64>, direction: Option<&str>) -> String {
    match (distance, direction) {
        (Some(value), Some(direction)) => format!("{value:.2} km {direction}"),
        (Some(value), None) => format!("{value:.2} km"),
        (None, Some(direction)) => format!("distance unknown, {direction}"),
        (None, None) => "distance unknown".to_string(),
    }
}

fn distance_meters(distance: Option<f64>, direction: Option<&str>) -> String {
    match (distance, direction) {
        (Some(value), Some(direction)) => format!("{value:.0} m {direction}"),
        (Some(value), None) => format!("{value:.0} m"),
        (None, Some(direction)) => format!("distance unknown, {direction}"),
        (None, None) => "distance unknown".to_string(),
    }
}

fn short_number(value: f64) -> String {
    let formatted = format!("{value:.2}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn body_bytes(body: &Body) -> &[u8] {
    match body {
        Body::Empty => &[],
        Body::Text(text) => text.as_bytes(),
        Body::Binary(bytes) => bytes,
        _ => &[],
    }
}

fn response(status: StatusCode, text: &str) -> Result<Response<Body>, lambda_http::Error> {
    Ok(Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(Body::Text(text.to_string()))?)
}

#[cfg(test)]
mod tests {
    use super::{help_text, location_reply_text};
    use crate::exif_geo::{Coordinates, ExifSummary};
    use crate::links::MapLinks;
    use crate::wikimedia::{NearbyWikidataItem, NearbyWikimedia, NearbyWikipediaArticle};

    #[test]
    fn formats_reply_with_city_and_all_links() {
        let coordinates = Coordinates {
            latitude: 41.715137,
            longitude: 44.827096,
        };
        let links = MapLinks::for_coordinates(coordinates, Some("tbilisi.jpg"));
        let reply = location_reply_text(
            "Location found in EXIF metadata.",
            coordinates,
            Some("Tbilisi, Georgia"),
            None,
            &links,
            None,
        );

        assert!(reply.contains("Location found in EXIF metadata."));
        assert!(reply.contains("Place: Tbilisi, Georgia"));
        assert!(reply.contains("\n\nGoogle Maps\nhttps://www.google.com/maps/search/"));
        assert!(reply.contains("\n\nApple Maps\nhttps://maps.apple.com/"));
        assert!(reply.contains("\n\nBing Maps\nhttps://bing.com/maps/"));
        assert!(reply.contains("\n\nGoogle Earth\nhttps://earth.google.com/web/search/"));
        assert!(reply.contains("\n\nYandex Maps\nhttps://yandex.com/maps/"));
        assert!(reply.contains("\n\n2GIS\nhttps://2gis.com/geo/"));
        assert!(reply.contains("\n\nMapillary\nhttps://www.mapillary.com/app/"));
        assert!(reply.contains("\n\nOpenStreetMap\nhttps://www.openstreetmap.org/"));
        assert!(reply.contains("\n\nWikimapia\nhttps://wikimapia.org/"));
        assert!(reply.contains("\n\nGeoHack\nhttps://geohack.toolforge.org/"));
        assert!(reply.contains("\n\nWikimap Toolforge\nhttps://wikimap.toolforge.org/"));
        assert!(!reply.contains("Google Maps:"));
    }

    #[test]
    fn formats_nearby_results_with_distance_and_direction() {
        let coordinates = Coordinates {
            latitude: 41.715137,
            longitude: 44.827096,
        };
        let links = MapLinks::for_coordinates(coordinates, None);
        let nearby = NearbyWikimedia {
            wikidata_items: vec![NearbyWikidataItem {
                id: "Q1".to_string(),
                label: "Nearby item".to_string(),
                url: "https://www.wikidata.org/wiki/Q1".to_string(),
                distance_km: Some(0.42),
                direction: Some("NE".to_string()),
            }],
            wikipedia_articles: vec![NearbyWikipediaArticle {
                title: "Nearby article".to_string(),
                language: "en".to_string(),
                url: "https://en.wikipedia.org/wiki/Nearby_article".to_string(),
                distance_meters: Some(180.0),
                direction: Some("W".to_string()),
            }],
        };
        let reply = location_reply_text(
            "Location received from Telegram.",
            coordinates,
            None,
            None,
            &links,
            Some(&nearby),
        );

        assert!(reply.contains("Nearby item (Q1, 0.42 km NE)"));
        assert!(reply.contains("Nearby article [en] (180 m W)"));
    }

    #[test]
    fn formats_exif_summary_with_camera_wikipedia_link() {
        let coordinates = Coordinates {
            latitude: 41.715137,
            longitude: 44.827096,
        };
        let links = MapLinks::for_coordinates(coordinates, None);
        let exif = ExifSummary {
            camera_make: Some("Apple".to_string()),
            camera_model: Some("iPhone 15 Pro".to_string()),
            camera_wikipedia_url: Some(
                "https://en.wikipedia.org/wiki/Special:Search?search=Apple%20iPhone%2015%20Pro"
                    .to_string(),
            ),
            f_number: Some(1.78),
            iso: Some(80),
            ..ExifSummary::default()
        };
        let reply = location_reply_text(
            "Location found in EXIF metadata.",
            coordinates,
            None,
            Some(&exif),
            &links,
            None,
        );

        assert!(reply.contains("EXIF:"));
        assert!(reply.contains("Camera: Apple iPhone 15 Pro\nhttps://en.wikipedia.org/wiki/Special:Search?search=Apple%20iPhone%2015%20Pro"));
        assert!(reply.contains("Aperture: f/1.78"));
        assert!(reply.contains("ISO: 80"));
    }

    #[test]
    fn formats_help_with_repo_support_and_privacy() {
        let help = help_text(
            "Telegram Photo Geolocator",
            "https://github.com/vitaly-zdanevich/bot_telegram_geolocation_from_photo",
        );

        assert!(help.contains(
            "Repo: https://github.com/vitaly-zdanevich/bot_telegram_geolocation_from_photo"
        ));
        assert!(help.contains("Support: @vitaly_zdanevich"));
        assert!(help.contains("stateless AWS Lambda"));
        assert!(help.contains("does not store photos"));
        assert!(help.contains("Commands: /help"));
        assert!(!help.contains("/privacy"));
    }
}
