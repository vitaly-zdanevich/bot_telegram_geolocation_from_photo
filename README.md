# Telegram Photo Geolocator Bot

Telegram bot for AWS Lambda. Send an original geotagged image as a file/document, or share a Telegram location. The bot returns coordinates, place name when available, selected EXIF details, clickable map links, nearby Wikidata and Wikipedia links ordered by distance, and a native Telegram map pin for EXIF image results.

Normal Telegram photos are compressed and often lose EXIF metadata. If a user sends a Telegram photo, the bot asks them to resend it as a file/document.

## Features

- Extracts EXIF GPS coordinates from original image files.
- Prints a compact EXIF summary: camera make/model with a Wikipedia search link, capture time, lens, focal length, aperture, exposure, ISO, altitude, image direction, GPS speed, and software when present.
- Supports common image containers handled by `nom-exif` and `kamadak-exif`: JPEG/JPG, TIFF/TIF, DNG, HEIC/HEIF, AVIF, PNG EXIF, WebP EXIF, Canon CR3, Fujifilm RAF, and Phase One IIQ.
- Handles user-shared Telegram locations and returns the same link set without sending a duplicate native pin.
- Returns links for Google Maps, Apple Maps, Bing Maps, Google Earth, Yandex Maps, 2GIS, Mapillary, OpenStreetMap, Wikimapia, GeoHack, and WikiMap Toolforge.
- Uses OpenStreetMap Nominatim reverse geocoding for place names: city, town, village, hamlet, municipality, county, state, country.
- Returns nearby Wikidata items and Wikipedia articles with language, distance, and compass direction from the source point.
- Deploys as an ARM64 AWS Lambda built for Neoverse with `RUST_TARGET_CPU=neoverse-n1`.
- Terraform default Lambda sizing: 3008 MB memory, 900 second timeout, ARM64, `provided.al2023`.

## Deploy

Create `infra/terraform.tfvars`:

```hcl
telegram_bot_token      = "123456:replace-me"
telegram_webhook_secret = "replace-with-a-long-random-secret"

# Optional but recommended for Nominatim operator contact:
nominatim_email = "you@example.com"
```

Then:

```bash
./scripts/deploy.sh
```

The deploy script:

1. Builds `build/lambda.zip` for `aarch64-unknown-linux-gnu` with `RUST_TARGET_CPU=neoverse-n1`.
2. Runs `terraform init` and `terraform apply`.
3. Sets the Telegram webhook to the Lambda Function URL with `allowed_updates=["message"]`.

Terraform state contains the Telegram bot token. Use an encrypted or otherwise protected backend if this leaves your machine.

## Operations

Read recent Lambda logs:

```bash
./scripts/show-logs.sh
```

Useful environment overrides:

```bash
LOG_MINUTES=120 ./scripts/show-logs.sh
AWS_REGION=us-east-1 ./scripts/show-logs.sh
```

Build only:

```bash
./scripts/build-lambda.sh
```

Set webhook only:

```bash
./scripts/set-webhook.sh
```

## Configuration

Main Terraform variables:

- `lambda_memory_size`: default `3008`.
- `lambda_timeout_seconds`: default `900`.
- `max_file_mb`: default `20`.
- `reserved_concurrent_executions`: default `-1`, meaning no function-level reservation.
- `enable_reverse_geocoding`: default `true`.
- `enable_wikimedia_lookup`: default `true`.
- `wikipedia_languages`: default fallback order is `en`, `ru`, `be`; Wikidata sitelinks are used as a fallback for other Wikipedia languages.
- `wikipedia_language`: legacy comma-separated fallback list, default `en,ru,be`, used only when `wikipedia_languages` is empty.
- `wikimedia_radius_meters`: default `10000`.
- `wikimedia_limit`: default `5`.

Runtime environment variables are produced by Terraform and read in `src/config.rs`.

## Bot Help

`/help` shows the repository URL, support username, usage summary, and privacy note. There is no separate `/privacy` command.

## Development

```bash
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
terraform fmt -check -recursive infra
```

CI runs the same checks. The workflow uses `actions/checkout@v7` and `hashicorp/setup-terraform@v4.0.1`, avoiding the Node 20 action runtime.

## EXIF Scope

The bot does not dump every raw EXIF tag. Full EXIF can contain bulky maker notes and private identifiers such as serial numbers. The reply includes the useful fields near geolocation and omits raw maker notes by default.

## Privacy

The bot runs on stateless AWS Lambda. It downloads each Telegram file into Lambda memory, extracts metadata, replies, and does not store photos. Telegram, AWS Lambda logs, Nominatim, Wikimedia, and linked map services may receive request metadata. Disable reverse geocoding and Wikimedia lookups if you want fewer outbound requests.

Support: [@vitaly_zdanevich](https://t.me/vitaly_zdanevich)

## My Other Telegram Bots

GitHub:

- [bot_telegram_wikimedia_commons_uploader](https://github.com/vitaly-zdanevich/bot_telegram_wikimedia_commons_uploader) - uploads images/media to Wikimedia Commons under each user's own account.
- [bot_telegram_rutracker](https://github.com/vitaly-zdanevich/bot_telegram_rutracker) - searches and downloads from RuTracker.
- [bot_telegram_wikipedia](https://github.com/vitaly-zdanevich/bot_telegram_wikipedia) - searches Wikipedia on AWS Lambda.
- [bot_telegram_wikimedia_commons](https://github.com/vitaly-zdanevich/bot_telegram_wikimedia_commons) - searches Wikimedia Commons media.

GitLab:

- [bot_telegram_youtube](https://gitlab.com/vitaly-zdanevich/bot_telegram_youtube) - searches YouTube and returns audio in Ogg/Opus.
- [bot_telegram_evernote](https://gitlab.com/vitaly-zdanevich/bot_telegram_evernote) - sends notes to Evernote on AWS Lambda.

## Links

- Telegram Bot API: [Document](https://core.telegram.org/bots/api#document), [PhotoSize](https://core.telegram.org/bots/api#photosize), [Location](https://core.telegram.org/bots/api#location), [getFile](https://core.telegram.org/bots/api#getfile), [sendLocation](https://core.telegram.org/bots/api#sendlocation), [setWebhook](https://core.telegram.org/bots/api#setwebhook)
- AWS Lambda Rust runtime: https://docs.aws.amazon.com/lambda/latest/dg/lambda-rust.html
- Cargo Lambda: https://www.cargo-lambda.info/
- Terraform AWS Lambda function: https://registry.terraform.io/providers/hashicorp/aws/latest/docs/resources/lambda_function
- OpenStreetMap Nominatim usage policy: https://operations.osmfoundation.org/policies/nominatim/
- Nominatim reverse API: https://nominatim.org/release-docs/latest/api/Reverse/
- Wikimedia User-Agent policy: https://foundation.wikimedia.org/wiki/Policy:User-Agent_policy
- MediaWiki Geosearch API: https://www.mediawiki.org/wiki/API:Geosearch
- Wikidata SPARQL query service examples: https://www.wikidata.org/wiki/Wikidata:SPARQL_query_service/queries/examples
- Google Maps URLs: https://developers.google.com/maps/documentation/urls/get-started
- Bing Maps custom map URLs: https://learn.microsoft.com/en-us/bingmaps/articles/create-a-custom-map-url
- Apple Map Links: https://developer.apple.com/library/archive/featuredarticles/iPhoneURLScheme_Reference/MapLinks/MapLinks.html
- OpenStreetMap marker URLs: https://wiki.openstreetmap.org/wiki/Browsing#Adding_a_Marker
- GeoHack: https://www.mediawiki.org/wiki/GeoHack
- WikiMap Toolforge: https://wikimap.toolforge.org/
- Mapillary: https://www.mapillary.com/
