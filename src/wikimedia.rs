use crate::exif_geo::Coordinates;
use crate::links::decimal;
use anyhow::Context;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;
use tracing::warn;

/// Nearby Wikimedia results for a coordinate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NearbyWikimedia {
    /// Nearby Wikidata items with labels.
    pub wikidata_items: Vec<NearbyWikidataItem>,
    /// Nearby Wikipedia articles.
    pub wikipedia_articles: Vec<NearbyWikipediaArticle>,
}

impl NearbyWikimedia {
    /// Returns true when no nearby Wikimedia data was found.
    pub fn is_empty(&self) -> bool {
        self.wikidata_items.is_empty() && self.wikipedia_articles.is_empty()
    }
}

/// Nearby Wikidata item.
#[derive(Clone, Debug, PartialEq)]
pub struct NearbyWikidataItem {
    /// QID, for example `Q9188`.
    pub id: String,
    /// Best available label.
    pub label: String,
    /// Wikidata item URL.
    pub url: String,
    /// Distance from the point in kilometers.
    pub distance_km: Option<f64>,
    /// Compass direction from the source point to the item.
    pub direction: Option<String>,
}

/// Nearby Wikipedia article.
#[derive(Clone, Debug, PartialEq)]
pub struct NearbyWikipediaArticle {
    /// Article title.
    pub title: String,
    /// Wikipedia language edition, for example `en` or `ru`.
    pub language: String,
    /// Article URL.
    pub url: String,
    /// Distance from the point in meters.
    pub distance_meters: Option<f64>,
    /// Compass direction from the source point to the article coordinate.
    pub direction: Option<String>,
}

/// Wikimedia API client for nearby Wikidata and Wikipedia lookup.
#[derive(Clone)]
pub struct WikimediaClient {
    client: reqwest::Client,
    user_agent: String,
    wikipedia_endpoints: Vec<WikipediaEndpoint>,
    wikipedia_languages: Vec<String>,
    wikidata_sparql_url: String,
    radius_meters: u32,
    limit: u32,
}

#[derive(Clone, Debug)]
struct WikipediaEndpoint {
    language: String,
    api_url: String,
}

impl WikimediaClient {
    /// Creates a Wikimedia client.
    pub fn new(
        client: reqwest::Client,
        user_agent: String,
        wikipedia_languages: Vec<String>,
        wikipedia_api_url: Option<String>,
        wikidata_sparql_url: String,
        radius_meters: u32,
        limit: u32,
    ) -> Self {
        let wikipedia_languages = if wikipedia_languages.is_empty() {
            vec!["en".to_string()]
        } else {
            wikipedia_languages
        };
        let wikipedia_endpoints = wikipedia_endpoints(&wikipedia_languages, wikipedia_api_url);

        Self {
            client,
            user_agent,
            wikipedia_endpoints,
            wikipedia_languages,
            wikidata_sparql_url,
            radius_meters,
            limit,
        }
    }

    /// Fetches nearby Wikidata items and Wikipedia articles.
    pub async fn nearby(&self, coordinates: Coordinates) -> NearbyWikimedia {
        let (wikidata_items, wikipedia_articles) = tokio::join!(
            self.nearby_wikidata_items(coordinates),
            self.nearby_wikipedia_articles(coordinates),
        );

        let wikidata_items = wikidata_items.unwrap_or_else(|error| {
            warn!(?error, "nearby Wikidata lookup failed");
            Vec::new()
        });
        let wikipedia_articles = wikipedia_articles.unwrap_or_else(|error| {
            warn!(?error, "nearby Wikipedia lookup failed");
            Vec::new()
        });

        NearbyWikimedia {
            wikidata_items,
            wikipedia_articles,
        }
    }

    async fn nearby_wikidata_items(
        &self,
        coordinates: Coordinates,
    ) -> anyhow::Result<Vec<NearbyWikidataItem>> {
        let query = self.wikidata_query(coordinates);
        let response = self
            .client
            .get(&self.wikidata_sparql_url)
            .header(USER_AGENT, &self.user_agent)
            .header("Api-User-Agent", &self.user_agent)
            .header(ACCEPT, "application/sparql-results+json")
            .query(&[("query", query.as_str()), ("format", "json")])
            .send()
            .await
            .context("Wikidata SPARQL request failed")?
            .error_for_status()
            .context("Wikidata SPARQL returned an error status")?
            .json::<SparqlResponse>()
            .await
            .context("failed to parse Wikidata SPARQL response")?;

        Ok(response.into_items(self.limit, coordinates))
    }

    async fn nearby_wikipedia_articles(
        &self,
        coordinates: Coordinates,
    ) -> anyhow::Result<Vec<NearbyWikipediaArticle>> {
        let mut articles = Vec::new();
        for endpoint in &self.wikipedia_endpoints {
            match self
                .nearby_wikipedia_articles_for_endpoint(endpoint, coordinates)
                .await
            {
                Ok(mut endpoint_articles) => articles.append(&mut endpoint_articles),
                Err(error) => {
                    warn!(
                        ?error,
                        language = %endpoint.language,
                        "nearby Wikipedia language lookup failed"
                    );
                }
            }
        }

        let mut fallback_articles = self
            .nearby_wikipedia_articles_from_wikidata(coordinates)
            .await
            .unwrap_or_else(|error| {
                warn!(?error, "nearby Wikipedia Wikidata fallback failed");
                Vec::new()
            });
        articles.append(&mut fallback_articles);
        deduplicate_articles(&mut articles);
        articles.sort_by(|left, right| article_order(left, right, &self.wikipedia_languages));
        articles.truncate(self.limit as usize);

        Ok(articles)
    }

    async fn nearby_wikipedia_articles_for_endpoint(
        &self,
        endpoint: &WikipediaEndpoint,
        coordinates: Coordinates,
    ) -> anyhow::Result<Vec<NearbyWikipediaArticle>> {
        let gscoord = format!(
            "{}|{}",
            decimal(coordinates.latitude),
            decimal(coordinates.longitude)
        );
        let radius = self.radius_meters.to_string();
        let limit = self.limit.to_string();
        let response = self
            .client
            .get(&endpoint.api_url)
            .header(USER_AGENT, &self.user_agent)
            .header("Api-User-Agent", &self.user_agent)
            .query(&[
                ("action", "query"),
                ("format", "json"),
                ("list", "geosearch"),
                ("gscoord", gscoord.as_str()),
                ("gsradius", radius.as_str()),
                ("gslimit", limit.as_str()),
                ("gsnamespace", "0"),
            ])
            .send()
            .await
            .context("Wikipedia geosearch request failed")?
            .error_for_status()
            .context("Wikipedia geosearch returned an error status")?
            .json::<WikipediaGeosearchResponse>()
            .await
            .context("failed to parse Wikipedia geosearch response")?;

        Ok(response.into_articles(&endpoint.language, coordinates, self.limit))
    }

    async fn nearby_wikipedia_articles_from_wikidata(
        &self,
        coordinates: Coordinates,
    ) -> anyhow::Result<Vec<NearbyWikipediaArticle>> {
        let query = self.wikidata_article_query(coordinates);
        let response = self
            .client
            .get(&self.wikidata_sparql_url)
            .header(USER_AGENT, &self.user_agent)
            .header("Api-User-Agent", &self.user_agent)
            .header(ACCEPT, "application/sparql-results+json")
            .query(&[("query", query.as_str()), ("format", "json")])
            .send()
            .await
            .context("Wikidata Wikipedia sitelink SPARQL request failed")?
            .error_for_status()
            .context("Wikidata Wikipedia sitelink SPARQL returned an error status")?
            .json::<SparqlResponse>()
            .await
            .context("failed to parse Wikidata Wikipedia sitelink SPARQL response")?;

        Ok(response.into_articles(self.limit, coordinates, &self.wikipedia_languages))
    }

    fn wikidata_query(&self, coordinates: Coordinates) -> String {
        let longitude = decimal(coordinates.longitude);
        let latitude = decimal(coordinates.latitude);
        let radius_km = decimal(self.radius_meters as f64 / 1000.0);
        let label_languages = wikidata_label_languages(&self.wikipedia_languages);

        format!(
            r#"SELECT ?item ?itemLabel ?location ?distance WHERE {{
	SERVICE wikibase:around {{
		?item wdt:P625 ?location .
		bd:serviceParam wikibase:center "Point({longitude} {latitude})"^^geo:wktLiteral .
		bd:serviceParam wikibase:radius "{radius_km}" .
		bd:serviceParam wikibase:distance ?distance .
	}}
	SERVICE wikibase:label {{ bd:serviceParam wikibase:language "{label_languages}" . }}
}}
ORDER BY ASC(?distance)
LIMIT {}"#,
            self.limit
        )
    }

    fn wikidata_article_query(&self, coordinates: Coordinates) -> String {
        let longitude = decimal(coordinates.longitude);
        let latitude = decimal(coordinates.latitude);
        let radius_km = decimal(self.radius_meters as f64 / 1000.0);
        let item_limit = self.limit.saturating_mul(4).max(self.limit);

        format!(
            r#"SELECT ?item ?article ?location ?distance WHERE {{
	{{
		SELECT ?item ?location ?distance WHERE {{
			SERVICE wikibase:around {{
				?item wdt:P625 ?location .
				bd:serviceParam wikibase:center "Point({longitude} {latitude})"^^geo:wktLiteral .
				bd:serviceParam wikibase:radius "{radius_km}" .
				bd:serviceParam wikibase:distance ?distance .
			}}
		}}
		ORDER BY ASC(?distance)
		LIMIT {item_limit}
	}}
	?article schema:about ?item ;
		schema:isPartOf ?wiki .
	?wiki wikibase:wikiGroup "wikipedia" .
}}
ORDER BY ASC(?distance)"#
        )
    }
}

#[derive(Debug, Deserialize)]
struct SparqlResponse {
    results: SparqlResults,
}

impl SparqlResponse {
    fn into_items(self, limit: u32, origin: Coordinates) -> Vec<NearbyWikidataItem> {
        let mut items = self
            .results
            .bindings
            .into_iter()
            .filter_map(|binding| binding.into_item(origin))
            .collect::<Vec<_>>();
        items.sort_by(|left, right| optional_distance_order(left.distance_km, right.distance_km));
        items.truncate(limit as usize);
        items
    }

    fn into_articles(
        self,
        limit: u32,
        origin: Coordinates,
        preferred_languages: &[String],
    ) -> Vec<NearbyWikipediaArticle> {
        let mut candidates = self
            .results
            .bindings
            .into_iter()
            .filter_map(|binding| binding.into_article_candidate(origin, preferred_languages))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            optional_distance_order(left.article.distance_meters, right.article.distance_meters)
                .then_with(|| left.language_rank.cmp(&right.language_rank))
        });

        let mut seen_items = Vec::new();
        let mut articles = Vec::new();
        for candidate in candidates {
            if seen_items.contains(&candidate.item_id) {
                continue;
            }
            seen_items.push(candidate.item_id);
            articles.push(candidate.article);
            if articles.len() >= limit as usize {
                break;
            }
        }

        articles
    }
}

#[derive(Debug, Deserialize)]
struct SparqlResults {
    bindings: Vec<SparqlBinding>,
}

#[derive(Debug, Deserialize)]
struct SparqlBinding {
    item: SparqlValue,
    #[serde(rename = "itemLabel")]
    item_label: Option<SparqlValue>,
    location: Option<SparqlValue>,
    distance: Option<SparqlValue>,
    article: Option<SparqlValue>,
}

impl SparqlBinding {
    fn into_item(self, origin: Coordinates) -> Option<NearbyWikidataItem> {
        let url = self.item.value;
        let id = wikidata_id_from_url(&url)?;
        let label = self
            .item_label
            .map(|value| value.value)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| id.clone());
        let distance_km = self
            .distance
            .and_then(|value| value.value.parse::<f64>().ok());
        let coordinates = self
            .location
            .as_ref()
            .and_then(|value| parse_wkt_point(&value.value));
        let distance_km =
            distance_km.or_else(|| coordinates.map(|target| distance_km_between(origin, target)));
        let direction = coordinates
            .and_then(|target| compass_direction(origin, target))
            .map(str::to_string);

        Some(NearbyWikidataItem {
            id,
            label,
            url,
            distance_km,
            direction,
        })
    }

    fn into_article_candidate(
        self,
        origin: Coordinates,
        preferred_languages: &[String],
    ) -> Option<WikidataArticleCandidate> {
        let item_id = wikidata_id_from_url(&self.item.value)?;
        let article_url = self.article?.value;
        let language = wikipedia_language_from_url(&article_url)?;
        let title = wikipedia_title_from_url(&article_url)?;
        let distance_km = self
            .distance
            .and_then(|value| value.value.parse::<f64>().ok());
        let coordinates = self
            .location
            .as_ref()
            .and_then(|value| parse_wkt_point(&value.value));
        let distance_meters = distance_km
            .map(|distance| distance * 1000.0)
            .or_else(|| coordinates.map(|target| distance_km_between(origin, target) * 1000.0));
        let direction = coordinates
            .and_then(|target| compass_direction(origin, target))
            .map(str::to_string);
        let rank = language_rank(&language, preferred_languages);

        Some(WikidataArticleCandidate {
            item_id,
            language_rank: rank,
            article: NearbyWikipediaArticle {
                title,
                language,
                url: article_url,
                distance_meters,
                direction,
            },
        })
    }
}

#[derive(Debug)]
struct WikidataArticleCandidate {
    item_id: String,
    language_rank: usize,
    article: NearbyWikipediaArticle,
}

#[derive(Debug, Deserialize)]
struct SparqlValue {
    value: String,
}

#[derive(Debug, Deserialize)]
struct WikipediaGeosearchResponse {
    query: Option<WikipediaQuery>,
}

impl WikipediaGeosearchResponse {
    fn into_articles(
        self,
        language: &str,
        origin: Coordinates,
        limit: u32,
    ) -> Vec<NearbyWikipediaArticle> {
        let mut articles = self
            .query
            .map(|query| query.geosearch)
            .unwrap_or_default()
            .into_iter()
            .map(|page| page.into_article(language, origin))
            .collect::<Vec<_>>();
        articles.sort_by(|left, right| {
            optional_distance_order(left.distance_meters, right.distance_meters)
        });
        articles.truncate(limit as usize);
        articles
    }
}

#[derive(Debug, Deserialize)]
struct WikipediaQuery {
    geosearch: Vec<WikipediaGeoPage>,
}

#[derive(Debug, Deserialize)]
struct WikipediaGeoPage {
    title: String,
    dist: Option<f64>,
    lat: Option<f64>,
    lon: Option<f64>,
}

impl WikipediaGeoPage {
    fn into_article(self, language: &str, origin: Coordinates) -> NearbyWikipediaArticle {
        let url_title = self.title.replace(' ', "_");
        let coordinates = self
            .lat
            .zip(self.lon)
            .and_then(|(latitude, longitude)| Coordinates::new(latitude, longitude));
        let distance_meters = self
            .dist
            .or_else(|| coordinates.map(|target| distance_km_between(origin, target) * 1000.0));
        let direction = coordinates
            .and_then(|target| compass_direction(origin, target))
            .map(str::to_string);

        NearbyWikipediaArticle {
            title: self.title,
            language: language.to_string(),
            url: format!(
                "https://{language}.wikipedia.org/wiki/{}",
                urlencoding::encode(&url_title)
            ),
            distance_meters,
            direction,
        }
    }
}

fn wikipedia_endpoints(languages: &[String], api_url: Option<String>) -> Vec<WikipediaEndpoint> {
    if let Some(api_url) = api_url {
        return vec![WikipediaEndpoint {
            language: languages
                .first()
                .cloned()
                .unwrap_or_else(|| "en".to_string()),
            api_url,
        }];
    }

    languages
        .iter()
        .map(|language| WikipediaEndpoint {
            language: language.clone(),
            api_url: format!("https://{language}.wikipedia.org/w/api.php"),
        })
        .collect()
}

fn wikidata_label_languages(preferred_languages: &[String]) -> String {
    let mut languages = preferred_languages.to_vec();
    for fallback in ["mul", "en"] {
        if !languages.iter().any(|language| language == fallback) {
            languages.push(fallback.to_string());
        }
    }
    languages.join(",")
}

fn deduplicate_articles(articles: &mut Vec<NearbyWikipediaArticle>) {
    let mut seen_urls = Vec::new();
    articles.retain(|article| {
        if seen_urls.contains(&article.url) {
            false
        } else {
            seen_urls.push(article.url.clone());
            true
        }
    });
}

fn article_order(
    left: &NearbyWikipediaArticle,
    right: &NearbyWikipediaArticle,
    preferred_languages: &[String],
) -> std::cmp::Ordering {
    optional_distance_order(left.distance_meters, right.distance_meters).then_with(|| {
        language_rank(&left.language, preferred_languages)
            .cmp(&language_rank(&right.language, preferred_languages))
    })
}

fn language_rank(language: &str, preferred_languages: &[String]) -> usize {
    preferred_languages
        .iter()
        .position(|preferred| preferred == language)
        .unwrap_or(preferred_languages.len())
}

fn wikipedia_language_from_url(url: &str) -> Option<String> {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = without_scheme.split('/').next()?;
    host.strip_suffix(".wikipedia.org")
        .filter(|language| !language.is_empty())
        .map(str::to_string)
}

fn wikipedia_title_from_url(url: &str) -> Option<String> {
    let raw_title = url.split("/wiki/").nth(1)?;
    let decoded = urlencoding::decode(raw_title).ok()?;
    Some(decoded.replace('_', " "))
}

fn optional_distance_order(left: Option<f64>, right: Option<f64>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.total_cmp(&right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn parse_wkt_point(value: &str) -> Option<Coordinates> {
    let inner = value.strip_prefix("Point(")?.strip_suffix(')')?;
    let mut parts = inner.split_whitespace();
    let longitude = parts.next()?.parse::<f64>().ok()?;
    let latitude = parts.next()?.parse::<f64>().ok()?;
    Coordinates::new(latitude, longitude)
}

fn distance_km_between(origin: Coordinates, target: Coordinates) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6371.0088;

    let lat1 = origin.latitude.to_radians();
    let lat2 = target.latitude.to_radians();
    let delta_lat = (target.latitude - origin.latitude).to_radians();
    let delta_lon = (target.longitude - origin.longitude).to_radians();
    let a =
        (delta_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (delta_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

    EARTH_RADIUS_KM * c
}

fn compass_direction(origin: Coordinates, target: Coordinates) -> Option<&'static str> {
    let same_latitude = (origin.latitude - target.latitude).abs() < f64::EPSILON;
    let same_longitude = (origin.longitude - target.longitude).abs() < f64::EPSILON;
    if same_latitude && same_longitude {
        return None;
    }

    let lat1 = origin.latitude.to_radians();
    let lat2 = target.latitude.to_radians();
    let delta_lon = (target.longitude - origin.longitude).to_radians();
    let y = delta_lon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * delta_lon.cos();
    let bearing = (y.atan2(x).to_degrees() + 360.0) % 360.0;
    let index = ((bearing + 11.25) / 22.5).floor() as usize % DIRECTIONS.len();

    Some(DIRECTIONS[index])
}

const DIRECTIONS: [&str; 16] = [
    "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW",
    "NNW",
];

fn wikidata_id_from_url(url: &str) -> Option<String> {
    url.rsplit('/').next().map(str::to_string).filter(|id| {
        id.len() > 1 && id.starts_with('Q') && id[1..].bytes().all(|byte| byte.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::{SparqlResponse, WikipediaGeosearchResponse, parse_wkt_point};
    use crate::exif_geo::Coordinates;

    #[test]
    fn parses_wikidata_sparql_response() {
        let json = r#"{
			"results": {
				"bindings": [{
					"item": {"type": "uri", "value": "http://www.wikidata.org/entity/Q9188"},
					"itemLabel": {"xml:lang": "en", "type": "literal", "value": "Empire State Building"},
					"location": {"datatype": "http://www.opengis.net/ont/geosparql#wktLiteral", "type": "literal", "value": "Point(-73.985656 40.748433)"},
					"distance": {"datatype": "http://www.w3.org/2001/XMLSchema#double", "type": "literal", "value": "0.012"}
				}]
			}
		}"#;
        let response = serde_json::from_str::<SparqlResponse>(json).unwrap();
        let items = response.into_items(
            5,
            Coordinates {
                latitude: 40.7484,
                longitude: -73.9857,
            },
        );

        assert_eq!(items[0].id, "Q9188");
        assert_eq!(items[0].label, "Empire State Building");
        assert_eq!(items[0].distance_km, Some(0.012));
        assert!(items[0].direction.is_some());
    }

    #[test]
    fn parses_wikipedia_geosearch_response() {
        let json = r#"{
			"query": {
				"geosearch": [{
					"pageid": 18618509,
					"ns": 0,
					"title": "Wikimedia Foundation",
					"lat": 37.7891838,
					"lon": -122.4033522,
					"dist": 0
				}]
			}
		}"#;
        let response = serde_json::from_str::<WikipediaGeosearchResponse>(json).unwrap();
        let articles = response.into_articles(
            "en",
            Coordinates {
                latitude: 37.789,
                longitude: -122.403,
            },
            5,
        );

        assert_eq!(articles[0].title, "Wikimedia Foundation");
        assert_eq!(articles[0].language, "en");
        assert_eq!(
            articles[0].url,
            "https://en.wikipedia.org/wiki/Wikimedia_Foundation"
        );
        assert_eq!(articles[0].distance_meters, Some(0.0));
        assert!(articles[0].direction.is_some());
    }

    #[test]
    fn parses_wikidata_wkt_point() {
        assert_eq!(
            parse_wkt_point("Point(44.827096 41.715137)"),
            Some(Coordinates {
                latitude: 41.715137,
                longitude: 44.827096,
            })
        );
    }

    #[test]
    fn sorts_results_by_distance() {
        let wikidata_json = r#"{
			"results": {
				"bindings": [
					{
						"item": {"type": "uri", "value": "http://www.wikidata.org/entity/Q2"},
						"itemLabel": {"type": "literal", "value": "Far"},
						"distance": {"type": "literal", "value": "5"}
					},
					{
						"item": {"type": "uri", "value": "http://www.wikidata.org/entity/Q1"},
						"itemLabel": {"type": "literal", "value": "Near"},
						"distance": {"type": "literal", "value": "1"}
					}
				]
			}
		}"#;
        let origin = Coordinates {
            latitude: 0.0,
            longitude: 0.0,
        };
        let response = serde_json::from_str::<SparqlResponse>(wikidata_json).unwrap();
        let items = response.into_items(5, origin);
        assert_eq!(items[0].id, "Q1");

        let wikipedia_json = r#"{
			"query": {
				"geosearch": [
					{"title": "Far", "dist": 5000},
					{"title": "Near", "dist": 1000}
				]
			}
		}"#;
        let response = serde_json::from_str::<WikipediaGeosearchResponse>(wikipedia_json).unwrap();
        let articles = response.into_articles("en", origin, 5);
        assert_eq!(articles[0].title, "Near");
    }

    #[test]
    fn picks_preferred_language_from_wikidata_sitelinks() {
        let json = r#"{
			"results": {
				"bindings": [
					{
						"item": {"type": "uri", "value": "http://www.wikidata.org/entity/Q1"},
						"article": {"type": "uri", "value": "https://fr.wikipedia.org/wiki/French_title"},
						"distance": {"type": "literal", "value": "0.1"}
					},
					{
						"item": {"type": "uri", "value": "http://www.wikidata.org/entity/Q1"},
						"article": {"type": "uri", "value": "https://ru.wikipedia.org/wiki/Russian_title"},
						"distance": {"type": "literal", "value": "0.1"}
					},
					{
						"item": {"type": "uri", "value": "http://www.wikidata.org/entity/Q2"},
						"article": {"type": "uri", "value": "https://be.wikipedia.org/wiki/Belarusian_title"},
						"distance": {"type": "literal", "value": "0.2"}
					}
				]
			}
		}"#;
        let origin = Coordinates {
            latitude: 0.0,
            longitude: 0.0,
        };
        let preferred_languages = vec!["en".to_string(), "ru".to_string(), "be".to_string()];
        let response = serde_json::from_str::<SparqlResponse>(json).unwrap();
        let articles = response.into_articles(5, origin, &preferred_languages);

        assert_eq!(articles[0].title, "Russian title");
        assert_eq!(articles[0].language, "ru");
        assert_eq!(articles[1].title, "Belarusian title");
        assert_eq!(articles[1].language, "be");
    }
}
