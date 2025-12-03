use anyhow::Result;
use image::{ImageBuffer, ImageEncoder, Rgba, RgbaImage, codecs::png::PngEncoder};
use sea_orm::{FromQueryResult, JsonValue};
use serde::{Deserialize, Deserializer, Serialize};
use std::cmp::Ordering;
use tracing::{debug, warn};
use utoipa::ToSchema;

/// Custom deserializer that accepts both strings and numbers for the label field
fn deserialize_label<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;

    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;

    match value {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s)),
        Some(serde_json::Value::Number(n)) => Ok(Some(n.to_string())),
        Some(serde_json::Value::Bool(b)) => Ok(Some(b.to_string())),
        Some(other) => Err(D::Error::custom(format!(
            "unexpected type for label: {:?}",
            other
        ))),
    }
}

// Representation of the JSON style.
#[derive(Deserialize, Clone, ToSchema, Serialize, FromQueryResult, Debug)]
pub struct ColorStop {
    pub value: f32,
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub opacity: u8,
    /// Optional label for discrete legends (e.g., "0.1 - 0.2", "<= 0.1", "> 10")
    #[serde(default, deserialize_with = "deserialize_label")]
    pub label: Option<String>,
}

/// Returns a discrete (non-interpolated) color based on a value and a set of color stops.
/// Each color stop represents the upper bound of its bucket.
/// Values outside the range are clamped to the nearest bucket's color.
pub fn get_color_discrete(value: f32, color_stops: &[(f32, Rgba<u8>)]) -> Rgba<u8> {
    if color_stops.is_empty() {
        return Rgba([0, 0, 0, 0]);
    }

    // Find the first bucket where value <= threshold
    for &(threshold, color) in color_stops {
        if value <= threshold {
            return color;
        }
    }

    // Value exceeds all thresholds - clamp to the highest bucket's color
    color_stops.last().map(|(_, c)| *c).unwrap_or(Rgba([0, 0, 0, 0]))
}

/// Returns an interpolated color based on a value and a set of color stops.
/// Values outside the range are clamped to the nearest stop's color.
pub fn get_color(value: f32, color_stops: &[(f32, Rgba<u8>)]) -> Rgba<u8> {
    if color_stops.is_empty() {
        return Rgba([0, 0, 0, 0]);
    }

    // Clamp to min color if below range
    if let Some(&(min_val, min_color)) = color_stops.first() {
        if value <= min_val {
            return min_color;
        }
    }

    // Clamp to max color if above range
    if let Some(&(max_val, max_color)) = color_stops.last() {
        if value >= max_val {
            return max_color;
        }
    }

    // Interpolate between stops
    for window in color_stops.windows(2) {
        let (v1, c1) = window[0];
        let (v2, c2) = window[1];
        if value == v1 {
            return c1;
        }
        if value < v2 {
            let t = (value - v1) / (v2 - v1);
            return Rgba([
                (c1.0[0] as f32 * (1.0 - t) + c2.0[0] as f32 * t) as u8,
                (c1.0[1] as f32 * (1.0 - t) + c2.0[1] as f32 * t) as u8,
                (c1.0[2] as f32 * (1.0 - t) + c2.0[2] as f32 * t) as u8,
                (c1.0[3] as f32 * (1.0 - t) + c2.0[3] as f32 * t) as u8,
            ]);
        }
    }
    *color_stops
        .last()
        .map(|(_, c)| c)
        .unwrap_or(&Rgba([0, 0, 0, 0]))
}

/// Applies a style to a grayscale image based on a provided style.
/// In this version, we assume that the input image is an ImageBuffer with u16 pixel values
/// (i.e. ImageBuffer<Luma<u16>, Vec<u16>>), where each pixel's value is the data value.
/// If the data value is outside the color stops range, a transparent pixel is returned.
///
/// The `interpolation_type` parameter determines how colors are applied:
/// - "linear" (default): Smooth gradient interpolation between color stops
/// - "discrete": Each value falls into a bucket and gets that bucket's color
pub fn style_layer(
    img: ImageBuffer<image::Luma<u16>, Vec<u16>>,
    style: Option<JsonValue>,
    interpolation_type: Option<&str>,
) -> Result<Vec<u8>> {
    let is_discrete = interpolation_type == Some("discrete");

    // Deserialize the style stops.
    let stops: Vec<ColorStop> = match style {
        Some(JsonValue::Array(arr)) => serde_json::from_value(JsonValue::Array(arr.clone()))
            .unwrap_or_else(|e| {
                warn!(error = %e, "Failed to deserialize style array");
                vec![]
            }),
        Some(JsonValue::String(ref s)) if !s.trim().is_empty() => serde_json::from_str(s)
            .unwrap_or_else(|e| {
                warn!(error = %e, "Failed to parse JSON style string");
                vec![]
            }),
        _ => {
            debug!("No valid style found, using default grayscale");
            vec![]
        }
    };

    // Determine the data range from the style stops.
    // If no stops are provided, we default to 0–255.
    let (_data_min, _data_max) = if stops.is_empty() {
        (0.0, 255.0)
    } else {
        let mut stops_sorted = stops.clone();
        stops_sorted.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(Ordering::Equal));
        (
            stops_sorted.first().unwrap().value,
            stops_sorted.last().unwrap().value,
        )
    };

    // Build color stops for interpolation.
    let color_stops: Vec<(f32, Rgba<u8>)> = if stops.is_empty() {
        vec![
            (0.0, Rgba([0, 0, 0, 255])),
            (255.0, Rgba([255, 255, 255, 255])),
        ]
    } else {
        let mut cs = stops
            .into_iter()
            .map(|cs| (cs.value, Rgba([cs.red, cs.green, cs.blue, cs.opacity])))
            .collect::<Vec<_>>();
        cs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
        cs
    };

    let (width, height) = img.dimensions();
    let img_rgba: RgbaImage = ImageBuffer::from_fn(width, height, |x, y| {
        // Read the u16 raw value and convert it to f32.
        let data_value = img.get_pixel(x, y)[0] as f32;
        // Optionally, if 0 represents no data, return transparent.
        if data_value == 0.0 {
            return Rgba([0, 0, 0, 0]);
        }

        if is_discrete {
            // For discrete mode, find the bucket and return its color
            get_color_discrete(data_value, &color_stops)
        } else {
            // For linear mode, interpolate (clamping is handled in get_color)
            get_color(data_value, &color_stops)
        }
    });

    // Encode the final RGBA image as a PNG.
    let mut png_data = Vec::new();
    {
        let encoder = PngEncoder::new(&mut png_data);
        encoder
            .write_image(
                img_rgba.as_raw(),
                img_rgba.width(),
                img_rgba.height(),
                image::ColorType::Rgba8.into(),
            )
            .map_err(|e| anyhow::anyhow!("[tile_handler] PNG encoding error: {:?}", e))?;
    }
    Ok(png_data)
}
