use anyhow::{Context, anyhow};
use std::env;

/// Runtime configuration read from Lambda environment variables.
#[derive(Clone, Debug)]
pub struct Config {
    /// Telegram bot token from BotFather.
    pub telegram_bot_token: String,
    /// Optional webhook secret checked against Telegram's secret-token header.
    pub telegram_webhook_secret: Option<String>,
    /// Maximum Telegram file size to download.
    pub max_file_bytes: u64,
    /// Project name used in replies and User-Agent strings.
    pub project_name: String,
    /// Public project URL used in documentation and User-Agent strings.
    pub github_url: String,
    /// Enables Nominatim reverse geocoding for city names.
    pub enable_reverse_geocoding: bool,
    /// Reverse geocoder endpoint.
    pub nominatim_base_url: String,
    /// Optional email parameter for Nominatim operator contact.
    pub nominatim_email: Option<String>,
    /// Preferred response language for reverse geocoding.
    pub nominatim_accept_language: Option<String>,
    /// User-Agent sent to Nominatim.
    pub nominatim_user_agent: String,
    /// Enables nearby Wikidata and Wikipedia lookups.
    pub enable_wikimedia_lookup: bool,
    /// Wikimedia API User-Agent.
    pub wikimedia_user_agent: String,
    /// Wikipedia language editions for nearby article search.
    pub wikipedia_languages: Vec<String>,
    /// Optional MediaWiki Action API URL override for nearby article search.
    pub wikipedia_api_url: Option<String>,
    /// Wikidata SPARQL endpoint for nearby item search.
    pub wikidata_sparql_url: String,
    /// Nearby search radius in meters for Wikimedia lookups.
    pub wikimedia_radius_meters: u32,
    /// Maximum number of nearby Wikidata and Wikipedia results each.
    pub wikimedia_limit: u32,
}

impl Config {
    /// Loads configuration from environment variables.
    pub fn from_env() -> anyhow::Result<Self> {
        let project_name = env_string("PROJECT_NAME")
            .unwrap_or_else(|| "telegram-photo-geolocator-bot".to_string());
        let github_url = env_string("GITHUB_URL").unwrap_or_else(|| {
            "https://github.com/vitaly-zdanevich/bot_telegram_geolocation_from_photo".to_string()
        });
        let telegram_bot_token = env_string("TELEGRAM_BOT_TOKEN")
            .ok_or_else(|| anyhow!("TELEGRAM_BOT_TOKEN is required"))?;
        let max_file_mb = env_string("MAX_FILE_MB")
            .as_deref()
            .unwrap_or("20")
            .parse::<u64>()
            .context("MAX_FILE_MB must be an integer")?;
        let nominatim_user_agent = env_string("NOMINATIM_USER_AGENT")
            .unwrap_or_else(|| format!("{project_name}/0.1 ({github_url})"));
        let wikipedia_languages = wikipedia_languages_from_env();
        let wikipedia_api_url = env_string("WIKIPEDIA_API_URL");

        Ok(Self {
            telegram_bot_token,
            telegram_webhook_secret: env_string("TELEGRAM_WEBHOOK_SECRET"),
            max_file_bytes: max_file_mb * 1024 * 1024,
            project_name,
            github_url,
            enable_reverse_geocoding: env_bool("ENABLE_REVERSE_GEOCODING", true)?,
            nominatim_base_url: env_string("NOMINATIM_BASE_URL")
                .unwrap_or_else(|| "https://nominatim.openstreetmap.org/reverse".to_string()),
            nominatim_email: env_string("NOMINATIM_EMAIL"),
            nominatim_accept_language: env_string("NOMINATIM_ACCEPT_LANGUAGE"),
            nominatim_user_agent: nominatim_user_agent.clone(),
            enable_wikimedia_lookup: env_bool("ENABLE_WIKIMEDIA_LOOKUP", true)?,
            wikimedia_user_agent: env_string("WIKIMEDIA_USER_AGENT")
                .unwrap_or(nominatim_user_agent),
            wikipedia_languages,
            wikipedia_api_url,
            wikidata_sparql_url: env_string("WIKIDATA_SPARQL_URL")
                .unwrap_or_else(|| "https://query.wikidata.org/sparql".to_string()),
            wikimedia_radius_meters: env_string("WIKIMEDIA_RADIUS_METERS")
                .as_deref()
                .unwrap_or("10000")
                .parse::<u32>()
                .context("WIKIMEDIA_RADIUS_METERS must be an integer")?,
            wikimedia_limit: env_string("WIKIMEDIA_LIMIT")
                .as_deref()
                .unwrap_or("5")
                .parse::<u32>()
                .context("WIKIMEDIA_LIMIT must be an integer")?
                .clamp(1, 10),
        })
    }
}

fn env_string(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_bool(name: &str, default: bool) -> anyhow::Result<bool> {
    let Some(value) = env_string(name) else {
        return Ok(default);
    };
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(anyhow!("{name} must be a boolean")),
    }
}

fn wikipedia_languages_from_env() -> Vec<String> {
    env_string("WIKIPEDIA_LANGUAGES")
        .or_else(|| env_string("WIKIPEDIA_LANGUAGE"))
        .map(|value| parse_language_codes(&value))
        .filter(|languages| !languages.is_empty())
        .unwrap_or_else(default_wikipedia_languages)
}

fn parse_language_codes(value: &str) -> Vec<String> {
    let mut languages = Vec::new();
    for part in value.split(|character: char| character == ',' || character.is_whitespace()) {
        let Some(language) = clean_language_code(part) else {
            continue;
        };
        if !languages.contains(&language) {
            languages.push(language);
        }
    }
    languages
}

fn clean_language_code(value: &str) -> Option<String> {
    let cleaned: String = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_ascii_lowercase())
    }
}

fn default_wikipedia_languages() -> Vec<String> {
    ["en", "ru", "be"].into_iter().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::{env_bool, parse_language_codes};

    #[test]
    fn parses_boolean_values() {
        unsafe {
            std::env::set_var("TEST_BOOL", "yes");
        }
        assert!(env_bool("TEST_BOOL", false).unwrap());

        unsafe {
            std::env::set_var("TEST_BOOL", "0");
        }
        assert!(!env_bool("TEST_BOOL", true).unwrap());
    }

    #[test]
    fn parses_wikipedia_language_list() {
        assert_eq!(parse_language_codes("en, ru be,en"), vec!["en", "ru", "be"]);
    }
}
