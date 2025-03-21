use crate::entity::{layer, style};
use crate::tiles::XYZTile;
use anyhow::Result;
use axum::{
    extract::{Path, Query},
    http::{header, StatusCode},
    response::IntoResponse,
};
use image::codecs::png::PngEncoder;
use image::ImageEncoder;
use image::{ImageBuffer, Rgba, RgbaImage};
use sea_orm::JsonValue;
use sea_orm::{entity::prelude::*, ColumnTrait, Database, DatabaseConnection, EntityTrait};
use serde::Deserialize;
use std::cmp::Ordering;
use tokio_retry::strategy::FixedInterval;
use tokio_retry::RetryIf;
#[derive(Deserialize)]
pub struct Params {
    layer: String,
}

// Representation of the JSON style
#[derive(Deserialize)]
struct ColorStop {
    value: f32,
    red: u8,
    green: u8,
    blue: u8,
    opacity: u8,
}

pub async fn tile_handler(
    Query(params): Query<Params>,
    Path((z, x, y)): Path<(u32, u32, u32)>,
) -> Result<impl IntoResponse, StatusCode> {
    let xyz_tile = XYZTile { x, y, z };
    let retry_strategy = FixedInterval::from_millis(200).take(5);
    let img = RetryIf::spawn(
        retry_strategy,
        || xyz_tile.get_one(&params.layer),
        |_: &anyhow::Error| {
            println!("[tile_handler] Error: x: {}, y: {}, z: {}", x, y, z);
            true
        },
    )
    .await
    .map_err(|e| {
        println!("[tile_handler] Failed after 5 attempts: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let config = crate::config::Config::from_env();
    let db: DatabaseConnection = Database::connect(config.db_url.as_ref().unwrap())
        .await
        .unwrap();

    // Find the layer record by layer name.
    let layer_record = match layer::Entity::find()
        .filter(layer::Column::LayerName.eq(&params.layer))
        .one(&db)
        .await
        .map_err(|e| {
            println!("[tile_handler] Database query error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })? {
        Some(rec) => rec,
        None => {
            println!("[tile_handler] No layer found for {}", &params.layer);
            return Err(StatusCode::NOT_FOUND);
        }
    };

    // Load the related style record(s).
    let related_styles = layer_record
        .find_related(style::Entity)
        .all(&db)
        .await
        .map_err(|e| {
            println!("[tile_handler] Database query error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Attempt to extract the style from the first related record.
    let dbstyle: Option<JsonValue> = related_styles.into_iter().next().and_then(|s| s.style);

    // Convert the raw JSON into a vector of ColorStop.
    let stops: Vec<ColorStop> = match dbstyle {
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
            println!(
                "[tile_handler] No valid style found for {}, using default grayscale.",
                &params.layer
            );
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
            get_color(p, &color_stops)
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
            .map_err(|e| {
                println!("[tile_handler] PNG encoding error: {:?}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }

    let response = ([(header::CONTENT_TYPE, "image/png")], png_data);
    Ok(response)
}

/// Returns an interpolated colour based on a value and a set of colour stops.
fn get_color(value: f32, color_stops: &[(f32, Rgba<u8>)]) -> Rgba<u8> {
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
