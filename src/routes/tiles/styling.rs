use anyhow::Result;
use image::{ImageBuffer, ImageEncoder, Rgba, RgbaImage, codecs::png::PngEncoder};
use sea_orm::{FromQueryResult, JsonValue};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use utoipa::ToSchema;

// Representation of the JSON style
#[derive(Deserialize, Clone, ToSchema, Serialize, FromQueryResult)]
pub struct ColorStop {
    value: f32,
    red: u8,
    green: u8,
    blue: u8,
    opacity: u8,
}

/// Returns an interpolated colour based on a value and a set of colour stops.
pub fn get_color(value: f32, color_stops: &[(f32, Rgba<u8>)]) -> Rgba<u8> {
    for window in color_stops.windows(2) {
        let (v1, c1) = window[0];
        let (v2, c2) = window[1];

        if value <= v1 {
            return c1;
        }
        if value <= v2 {
            let t = (value - v1) / (v2 - v1);
            return Rgba([
                (c1.0[0] as f32 * (1.0 - t) + c2.0[0] as f32 * t) as u8,
                (c1.0[1] as f32 * (1.0 - t) + c2.0[1] as f32 * t) as u8,
                (c1.0[2] as f32 * (1.0 - t) + c2.0[2] as f32 * t) as u8,
                (c1.0[3] as f32 * (1.0 - t) + c2.0[3] as f32 * t) as u8,
            ]);
        }
    }
    color_stops
        .last()
        .map(|(_, c)| *c)
        .unwrap_or(Rgba([0, 0, 0, 255]))
}
pub fn style_layer(
    img: ImageBuffer<image::Luma<u8>, Vec<u8>>,
    style: Option<JsonValue>,
) -> Result<Vec<u8>> {
    // Convert the raw JSON into a vector of ColorStop.
    let stops: Vec<ColorStop> = match style {
        // If the column is stored as a JSON array, we can deserialize it directly.
        Some(JsonValue::Array(arr)) => serde_json::from_value(JsonValue::Array(arr.clone()))
            .unwrap_or_else(|e| {
                println!("[tile_handler] Failed to deserialize style array: {:?}", e);
                vec![]
            }),
        // If the column is stored as a non-empty JSON string, parse it.
        Some(JsonValue::String(ref s)) if !s.trim().is_empty() => serde_json::from_str(s)
            .unwrap_or_else(|e| {
                println!("[tile_handler] Failed to parse JSON style string: {:?}", e);
                vec![]
            }),
        // No valid style provided.
        _ => {
            println!("[tile_handler] No valid style found, using default grayscale.",);
            vec![]
        }
    };
    // Build our color stops (value, Rgba) for interpolation.
    let color_stops: Vec<(f32, Rgba<u8>)> = if stops.is_empty() {
        // Default grayscale mapping.
        vec![
            (0.0, Rgba([0, 0, 0, 255])),
            (255.0, Rgba([255, 255, 255, 255])),
        ]
    } else {
        let mut stops = stops
            .into_iter()
            .map(|cs| (cs.value, Rgba([cs.red, cs.green, cs.blue, cs.opacity])))
            .collect::<Vec<_>>();
        stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
        stops
    };

    let (width, height) = img.dimensions();
    let img_rgba: RgbaImage = ImageBuffer::from_fn(width, height, |x, y| {
        let p = img.get_pixel(x, y)[0] as f32;
        if p == 0.0 {
            Rgba([0, 0, 0, 0])
        } else {
            super::styling::get_color(p, &color_stops)
        }
    });

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
