use crate::exif_geo::Coordinates;
use crate::links::decimal;
use anyhow::Context;
use reqwest::header::{ACCEPT_LANGUAGE, USER_AGENT};
use serde::Deserialize;

/// Optional reverse geocoder backed by Nominatim-compatible API responses.
#[derive(Clone)]
pub struct Geocoder {
    client: reqwest::Client,
    base_url: String,
    user_agent: String,
    email: Option<String>,
    accept_language: Option<String>,
}

impl Geocoder {
    /// Creates a new reverse geocoder client.
    pub fn new(
        client: reqwest::Client,
        base_url: String,
        user_agent: String,
        email: Option<String>,
        accept_language: Option<String>,
    ) -> Self {
        Self {
            client,
            base_url,
            user_agent,
            email,
            accept_language,
        }
    }

    /// Returns a human-readable place label for coordinates.
    pub async fn reverse_city(&self, coordinates: Coordinates) -> anyhow::Result<Option<String>> {
        let latitude = decimal(coordinates.latitude);
        let longitude = decimal(coordinates.longitude);
        let mut query = vec![
            ("format", "jsonv2".to_string()),
            ("addressdetails", "1".to_string()),
            ("zoom", "10".to_string()),
            ("layer", "address".to_string()),
            ("lat", latitude),
            ("lon", longitude),
        ];
        if let Some(email) = &self.email {
            query.push(("email", email.clone()));
        }

        let mut request = self
            .client
            .get(&self.base_url)
            .header(USER_AGENT, &self.user_agent);
        if let Some(accept_language) = &self.accept_language {
            request = request.header(ACCEPT_LANGUAGE, accept_language);
        }

        let response = request
            .query(&query)
            .send()
            .await
            .context("Nominatim reverse geocoding request failed")?
            .error_for_status()
            .context("Nominatim reverse geocoding returned an error status")?
            .json::<NominatimReverseResponse>()
            .await
            .context("failed to parse Nominatim reverse geocoding response")?;

        Ok(response.city_label())
    }
}

#[derive(Debug, Deserialize)]
struct NominatimReverseResponse {
    address: Option<NominatimAddress>,
}

impl NominatimReverseResponse {
    fn city_label(&self) -> Option<String> {
        self.address.as_ref().and_then(NominatimAddress::city_label)
    }
}

#[derive(Debug, Default, Deserialize)]
struct NominatimAddress {
    city: Option<String>,
    town: Option<String>,
    village: Option<String>,
    hamlet: Option<String>,
    municipality: Option<String>,
    county: Option<String>,
    state: Option<String>,
    country: Option<String>,
}

impl NominatimAddress {
    fn city_label(&self) -> Option<String> {
        let place = [
            self.city.as_deref(),
            self.town.as_deref(),
            self.village.as_deref(),
            self.hamlet.as_deref(),
            self.municipality.as_deref(),
            self.county.as_deref(),
            self.state.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())?;

        Some(match self.country.as_deref() {
            Some(country) if !country.trim().is_empty() && country != place => {
                format!("{place}, {country}")
            }
            _ => place.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{NominatimAddress, NominatimReverseResponse};

    #[test]
    fn formats_city_with_country() {
        let response = NominatimReverseResponse {
            address: Some(NominatimAddress {
                city: Some("Tbilisi".to_string()),
                country: Some("Georgia".to_string()),
                ..NominatimAddress::default()
            }),
        };

        assert_eq!(response.city_label().as_deref(), Some("Tbilisi, Georgia"));
    }

    #[test]
    fn falls_back_to_town_or_state() {
        let response = NominatimReverseResponse {
            address: Some(NominatimAddress {
                town: Some("Sutton Coldfield".to_string()),
                country: Some("United Kingdom".to_string()),
                ..NominatimAddress::default()
            }),
        };
        assert_eq!(
            response.city_label().as_deref(),
            Some("Sutton Coldfield, United Kingdom")
        );

        let response = NominatimReverseResponse {
            address: Some(NominatimAddress {
                state: Some("Nevada".to_string()),
                country: Some("United States".to_string()),
                ..NominatimAddress::default()
            }),
        };
        assert_eq!(
            response.city_label().as_deref(),
            Some("Nevada, United States")
        );
    }

    #[test]
    fn falls_back_to_village_or_hamlet() {
        let response = NominatimReverseResponse {
            address: Some(NominatimAddress {
                village: Some("Porozovo".to_string()),
                country: Some("Belarus".to_string()),
                ..NominatimAddress::default()
            }),
        };
        assert_eq!(response.city_label().as_deref(), Some("Porozovo, Belarus"));

        let response = NominatimReverseResponse {
            address: Some(NominatimAddress {
                hamlet: Some("Small Place".to_string()),
                country: Some("Ireland".to_string()),
                ..NominatimAddress::default()
            }),
        };
        assert_eq!(
            response.city_label().as_deref(),
            Some("Small Place, Ireland")
        );
    }
}
