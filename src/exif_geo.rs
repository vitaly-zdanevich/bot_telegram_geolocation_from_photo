use exif::{Exif as KamadakExif, Field, In, Tag, Value};
use nom_exif::{EntryValue, Exif, ExifDateTime, ExifTag, GPSInfo, MediaParser, MediaSource};

/// Decimal-degree GPS coordinates from EXIF metadata.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coordinates {
    /// Latitude in WGS84 decimal degrees.
    pub latitude: f64,
    /// Longitude in WGS84 decimal degrees.
    pub longitude: f64,
}

impl Coordinates {
    /// Creates validated coordinates.
    pub fn new(latitude: f64, longitude: f64) -> Option<Self> {
        let valid_latitude = (-90.0..=90.0).contains(&latitude);
        let valid_longitude = (-180.0..=180.0).contains(&longitude);
        (valid_latitude && valid_longitude).then_some(Self {
            latitude,
            longitude,
        })
    }
}

/// Extracted location and selected image metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageLocation {
    /// Coordinates from image metadata.
    pub coordinates: Coordinates,
    /// Compact EXIF metadata summary.
    pub exif: ExifSummary,
}

/// User-facing EXIF fields that are useful near coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExifSummary {
    /// Camera manufacturer.
    pub camera_make: Option<String>,
    /// Camera model.
    pub camera_model: Option<String>,
    /// Wikipedia search URL for the camera make/model.
    pub camera_wikipedia_url: Option<String>,
    /// Original capture datetime, when present.
    pub captured_at: Option<String>,
    /// Lens model.
    pub lens_model: Option<String>,
    /// Focal length in millimeters.
    pub focal_length_mm: Option<f64>,
    /// 35mm-equivalent focal length.
    pub focal_length_35mm: Option<u32>,
    /// F-number.
    pub f_number: Option<f64>,
    /// Exposure time, formatted for display.
    pub exposure_time: Option<String>,
    /// ISO speed rating.
    pub iso: Option<u32>,
    /// GPS altitude in meters.
    pub altitude_meters: Option<f64>,
    /// Image direction in degrees from north.
    pub image_direction_degrees: Option<f64>,
    /// GPS speed value with unit.
    pub gps_speed: Option<String>,
    /// Software that wrote the image metadata.
    pub software: Option<String>,
}

impl ExifSummary {
    /// Returns true when no summary field is available.
    pub fn is_empty(&self) -> bool {
        self.camera_make.is_none()
            && self.camera_model.is_none()
            && self.captured_at.is_none()
            && self.lens_model.is_none()
            && self.focal_length_mm.is_none()
            && self.focal_length_35mm.is_none()
            && self.f_number.is_none()
            && self.exposure_time.is_none()
            && self.iso.is_none()
            && self.altitude_meters.is_none()
            && self.image_direction_degrees.is_none()
            && self.gps_speed.is_none()
            && self.software.is_none()
    }
}

/// Extracts GPS coordinates from EXIF metadata in an image container.
///
/// Telegram documents should preserve the original image bytes. Normal Telegram
/// photos are resized/re-encoded and usually have their metadata stripped.
pub fn extract_coordinates(bytes: &[u8]) -> Option<Coordinates> {
    extract_image_location(bytes).map(|location| location.coordinates)
}

/// Extracts coordinates and selected EXIF details from an image container.
pub fn extract_image_location(bytes: &[u8]) -> Option<ImageLocation> {
    extract_image_location_with_nom_exif(bytes)
        .or_else(|| extract_image_location_with_kamadak_exif(bytes))
}

fn extract_image_location_with_nom_exif(bytes: &[u8]) -> Option<ImageLocation> {
    let source = MediaSource::from_memory(bytes.to_vec()).ok()?;
    let mut parser = MediaParser::new();
    let exif_iter = parser.parse_exif(source).ok()?;
    let gps = exif_iter.parse_gps().ok().flatten()?;
    let exif: Exif = exif_iter.into();
    let coordinates = coordinates_from_nom_gps(&gps)?;

    Some(ImageLocation {
        coordinates,
        exif: exif_summary_from_nom_exif(&exif),
    })
}

fn coordinates_from_nom_gps(gps: &GPSInfo) -> Option<Coordinates> {
    Coordinates::new(gps.latitude_decimal()?, gps.longitude_decimal()?)
}

fn extract_image_location_with_kamadak_exif(bytes: &[u8]) -> Option<ImageLocation> {
    let exif = exif::Reader::new()
        .read_from_container(&mut std::io::Cursor::new(bytes))
        .ok()?;
    let coordinates = coordinates_from_kamadak_exif(&exif)?;

    Some(ImageLocation {
        coordinates,
        exif: exif_summary_from_kamadak_exif(&exif),
    })
}

fn coordinates_from_kamadak_exif(exif: &KamadakExif) -> Option<Coordinates> {
    Coordinates::new(
        gps_coordinate(exif, Tag::GPSLatitude, Tag::GPSLatitudeRef, 'S')?,
        gps_coordinate(exif, Tag::GPSLongitude, Tag::GPSLongitudeRef, 'W')?,
    )
}

fn gps_coordinate(
    exif: &KamadakExif,
    coord_tag: Tag,
    ref_tag: Tag,
    negative_ref: char,
) -> Option<f64> {
    let field = exif.get_field(coord_tag, In::PRIMARY)?;
    let degrees = rationals_to_degrees(&field.value)?;
    let hemisphere = exif
        .get_field(ref_tag, In::PRIMARY)
        .and_then(ascii_value)
        .and_then(|reference| reference.chars().next());
    let sign = if hemisphere == Some(negative_ref) {
        -1.0
    } else {
        1.0
    };
    Some(sign * degrees)
}

fn rationals_to_degrees(value: &Value) -> Option<f64> {
    let Value::Rational(parts) = value else {
        return None;
    };
    if parts.len() < 3 {
        return None;
    }
    Some(parts[0].to_f64() + parts[1].to_f64() / 60.0 + parts[2].to_f64() / 3600.0)
}

fn ascii_value(field: &Field) -> Option<String> {
    match &field.value {
        Value::Ascii(values) => values
            .first()
            .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string()),
        _ => None,
    }
}

fn exif_summary_from_nom_exif(exif: &Exif) -> ExifSummary {
    let camera_make = text_value(exif, ExifTag::Make);
    let camera_model = text_value(exif, ExifTag::Model);
    let camera_wikipedia_url =
        camera_wikipedia_url(camera_make.as_deref(), camera_model.as_deref());

    ExifSummary {
        camera_make,
        camera_model,
        camera_wikipedia_url,
        captured_at: datetime_value(exif, ExifTag::DateTimeOriginal)
            .or_else(|| datetime_value(exif, ExifTag::CreateDate))
            .or_else(|| datetime_value(exif, ExifTag::ModifyDate)),
        lens_model: text_value(exif, ExifTag::LensModel),
        focal_length_mm: float_value(exif, ExifTag::FocalLength),
        focal_length_35mm: integer_value(exif, ExifTag::FocalLengthIn35mmFilm),
        f_number: float_value(exif, ExifTag::FNumber),
        exposure_time: exposure_time_value(exif),
        iso: integer_value(exif, ExifTag::ISOSpeedRatings),
        altitude_meters: exif.gps_info().and_then(GPSInfo::altitude_meters),
        image_direction_degrees: float_value(exif, ExifTag::GPSImgDirection),
        gps_speed: gps_speed_value(exif),
        software: text_value(exif, ExifTag::Software),
    }
}

fn exif_summary_from_kamadak_exif(exif: &KamadakExif) -> ExifSummary {
    let camera_make = kamadak_text_value(exif, Tag::Make);
    let camera_model = kamadak_text_value(exif, Tag::Model);
    let camera_wikipedia_url =
        camera_wikipedia_url(camera_make.as_deref(), camera_model.as_deref());

    ExifSummary {
        camera_make,
        camera_model,
        camera_wikipedia_url,
        captured_at: kamadak_text_value(exif, Tag::DateTimeOriginal)
            .or_else(|| kamadak_text_value(exif, Tag::DateTime)),
        lens_model: kamadak_text_value(exif, Tag::LensModel),
        focal_length_mm: kamadak_rational_value(exif, Tag::FocalLength),
        focal_length_35mm: kamadak_u32_value(exif, Tag::FocalLengthIn35mmFilm),
        f_number: kamadak_rational_value(exif, Tag::FNumber),
        exposure_time: kamadak_exposure_time_value(exif),
        iso: kamadak_u32_value(exif, Tag::PhotographicSensitivity)
            .or_else(|| kamadak_u32_value(exif, Tag::ISOSpeed)),
        altitude_meters: kamadak_altitude_meters(exif),
        image_direction_degrees: kamadak_rational_value(exif, Tag::GPSImgDirection),
        gps_speed: kamadak_gps_speed_value(exif),
        software: kamadak_text_value(exif, Tag::Software),
    }
}

fn text_value(exif: &Exif, tag: ExifTag) -> Option<String> {
    exif.get(tag)
        .and_then(EntryValue::as_str)
        .map(clean_text)
        .filter(|value| !value.is_empty())
}

fn datetime_value(exif: &Exif, tag: ExifTag) -> Option<String> {
    let datetime = exif.get(tag)?.as_datetime()?;
    Some(match datetime {
        ExifDateTime::Aware(value) => value.to_rfc3339(),
        ExifDateTime::Naive(value) => value.to_string(),
    })
}

fn float_value(exif: &Exif, tag: ExifTag) -> Option<f64> {
    exif.get(tag).and_then(EntryValue::try_as_float)
}

fn integer_value(exif: &Exif, tag: ExifTag) -> Option<u32> {
    exif.get(tag)
        .and_then(EntryValue::try_as_integer)
        .and_then(|value| u32::try_from(value).ok())
}

fn exposure_time_value(exif: &Exif) -> Option<String> {
    let value = exif.get(ExifTag::ExposureTime)?;
    if let Some(rational) = value.as_urational() {
        return Some(format_rational_seconds(
            rational.numerator() as f64,
            rational.denominator() as f64,
        ));
    }
    value.try_as_float().map(format_seconds)
}

fn gps_speed_value(exif: &Exif) -> Option<String> {
    let value = float_value(exif, ExifTag::GPSSpeed)?;
    let unit = text_value(exif, ExifTag::GPSSpeedRef)
        .as_deref()
        .and_then(|value| value.chars().next())
        .map(gps_speed_unit)
        .unwrap_or("km/h");
    Some(format!("{} {unit}", trim_float(value, 2)))
}

fn kamadak_text_value(exif: &KamadakExif, tag: Tag) -> Option<String> {
    exif.get_field(tag, In::PRIMARY)
        .and_then(ascii_value)
        .map(clean_text)
        .filter(|value| !value.is_empty())
}

fn kamadak_rational_value(exif: &KamadakExif, tag: Tag) -> Option<f64> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    match &field.value {
        Value::Rational(values) => values.first().map(exif::Rational::to_f64),
        Value::SRational(values) => values.first().map(exif::SRational::to_f64),
        _ => None,
    }
}

fn kamadak_u32_value(exif: &KamadakExif, tag: Tag) -> Option<u32> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    match &field.value {
        Value::Byte(values) => values.first().map(|value| u32::from(*value)),
        Value::Short(values) => values.first().map(|value| u32::from(*value)),
        Value::Long(values) => values.first().copied(),
        _ => None,
    }
}

fn kamadak_exposure_time_value(exif: &KamadakExif) -> Option<String> {
    let field = exif.get_field(Tag::ExposureTime, In::PRIMARY)?;
    match &field.value {
        Value::Rational(values) => values
            .first()
            .map(|value| format_rational_seconds(value.num as f64, value.denom as f64)),
        _ => None,
    }
}

fn kamadak_altitude_meters(exif: &KamadakExif) -> Option<f64> {
    let altitude = kamadak_rational_value(exif, Tag::GPSAltitude)?;
    let below_sea_level = exif
        .get_field(Tag::GPSAltitudeRef, In::PRIMARY)
        .and_then(|field| match &field.value {
            Value::Byte(values) => values.first().copied(),
            _ => None,
        })
        == Some(1);
    Some(if below_sea_level { -altitude } else { altitude })
}

fn kamadak_gps_speed_value(exif: &KamadakExif) -> Option<String> {
    let value = kamadak_rational_value(exif, Tag::GPSSpeed)?;
    let unit = kamadak_text_value(exif, Tag::GPSSpeedRef)
        .as_deref()
        .and_then(|value| value.chars().next())
        .map(gps_speed_unit)
        .unwrap_or("km/h");
    Some(format!("{} {unit}", trim_float(value, 2)))
}

fn camera_wikipedia_url(make: Option<&str>, model: Option<&str>) -> Option<String> {
    let query = [make, model]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!query.is_empty()).then(|| {
        format!(
            "https://en.wikipedia.org/wiki/Special:Search?search={}",
            urlencoding::encode(&query)
        )
    })
}

fn format_rational_seconds(numerator: f64, denominator: f64) -> String {
    if numerator > 0.0 && denominator > numerator {
        let reciprocal = denominator / numerator;
        if (reciprocal - reciprocal.round()).abs() < 0.001 {
            return format!("1/{:.0} s", reciprocal.round());
        }
    }
    format_seconds(numerator / denominator)
}

fn format_seconds(value: f64) -> String {
    format!("{} s", trim_float(value, 4))
}

fn gps_speed_unit(value: char) -> &'static str {
    match value {
        'K' | 'k' => "km/h",
        'M' | 'm' => "mph",
        'N' | 'n' => "knots",
        _ => "km/h",
    }
}

fn clean_text(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .trim_matches(char::from(0))
        .trim()
        .to_string()
}

fn trim_float(value: f64, precision: usize) -> String {
    let formatted = format!("{value:.precision$}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        Coordinates, camera_wikipedia_url, coordinates_from_nom_gps, format_rational_seconds,
    };
    use nom_exif::{Altitude, GPSInfo, LatLng, LatRef, LonRef, URational};

    #[test]
    fn validates_coordinate_ranges() {
        assert_eq!(
            Coordinates::new(41.715137, 44.827096),
            Some(Coordinates {
                latitude: 41.715137,
                longitude: 44.827096
            })
        );
        assert_eq!(Coordinates::new(91.0, 44.0), None);
        assert_eq!(Coordinates::new(41.0, 181.0), None);
    }

    #[test]
    fn converts_nom_exif_gps_coordinates() {
        let gps = GPSInfo {
            latitude_ref: LatRef::South,
            latitude: LatLng::new(
                URational::new(33, 1),
                URational::new(51, 1),
                URational::new(312, 10),
            ),
            longitude_ref: LonRef::East,
            longitude: LatLng::new(
                URational::new(151, 1),
                URational::new(12, 1),
                URational::new(306, 10),
            ),
            altitude: Altitude::Unknown,
            speed: None,
        };

        let coordinates = coordinates_from_nom_gps(&gps).unwrap();
        assert!((coordinates.latitude + 33.858667).abs() < 0.000001);
        assert!((coordinates.longitude - 151.2085).abs() < 0.000001);
    }

    #[test]
    fn formats_exif_display_values() {
        assert_eq!(format_rational_seconds(1.0, 125.0), "1/125 s");
        assert_eq!(format_rational_seconds(1.0, 2.0), "1/2 s");
        assert_eq!(
            camera_wikipedia_url(Some("Apple"), Some("iPhone 15 Pro")).as_deref(),
            Some("https://en.wikipedia.org/wiki/Special:Search?search=Apple%20iPhone%2015%20Pro")
        );
    }
}
