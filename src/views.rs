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
use sea_orm::{
    entity::prelude::*, ActiveModelTrait, ActiveValue, ColumnTrait, Condition, Database,
    DatabaseConnection, DbBackend, DbErr, EntityTrait, FromQueryResult, Order, QueryOrder,
    QuerySelect, Statement,
};
// use sea_orm::{DatabaseConnection};
use serde::Deserialize;
use std::cmp::Ordering;
use tokio_retry::strategy::FixedInterval;
use tokio_retry::RetryIf;

#[derive(Deserialize)]
pub struct Params {
    layer: String,
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
    // Get the filename from the layer name in the database
    let dbres = layer::Entity::find()
        .find_with_related(style::Entity)
        .filter(layer::Column::LayerName.eq(&params.layer))
        .all(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if dbres.is_empty() {
        println!("[tile_handler] No layer found for {}", &params.layer);
        return Err(StatusCode::NOT_FOUND);
    }
    let (dblayer, dbstyle_res) = dbres.first().unwrap();
    let dbstyle = dbstyle_res
        .clone()
        .pop()
        .map(|s| s.style)
        .unwrap_or_default();
    println!("Dbstyle: {:?}", dbstyle);
    // if dbstyle.is_none() {
    //     println!("[tile_handler] No style found for {}", &params.layer);
    // } else {
    //     println!(
    //         "[tile_handler] Found style for {}: {:?}",
    //         &params.layer,
    //         dbstyle.clone().unwrap()
    //     );
    // }
    // println!("[tile_handler] dbres: {:?}", dbstyle);
    // Unwrap the style. It is structured as a JSON: [{"value": 0.52352285385132, "red": 215, "green": 25, "blue": 28, "opacity": 255, "label": 0.5235}, {"value": 2.24634027481079, "red": 253, "green": 174, "blue": 97, "opacity": 255, "label": 2.2463}, {"value": 3.96915769577026, "red": 255, "green": 255, "blue": 191, "opacity": 255, "label": 3.9692}, {"value": 5.69197511672973, "red": 171, "green": 221, "blue": 164, "opacity": 255, "label": 5.692}, {"value": 7.4147925376892, "red": 43, "green": 131, "blue": 186, "opacity": 255, "label": 7.4148}]

    // let dbstyle = dbstyle.unwrap().style;
    // let style_vec: Vec<serde_json::Value> =
    // serde_json::from_str(&dbstyle).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // println!("[tile_handler] style_vec: {:?}", style_vec);
    // Parse JSON style
    let dbstyle_str = dbstyle.as_ref().ok_or_else(|| {
        println!("[tile_handler] dbstyle is None");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let style_vec: Vec<serde_json::Value> = serde_json::from_str(dbstyle_str.to_string().as_str())
        .map_err(|e| {
            println!("[tile_handler] Failed to parse JSON: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    println!("style_vec: {:?}", style_vec);
    // Convert JSON to a sorted vector of tuples (value, rgba)
    let mut color_stops: Vec<(f32, Rgba<u8>)> = style_vec
        .iter()
        .filter_map(|entry| {
            Some((
                entry.get("value")?.as_f64()? as f32,
                Rgba([
                    entry.get("red")?.as_u64()? as u8,
                    entry.get("green")?.as_u64()? as u8,
                    entry.get("blue")?.as_u64()? as u8,
                    entry.get("opacity")?.as_u64()? as u8,
                ]),
            ))
        })
        .collect();

    // Sort by value
    color_stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));

    // Convert the grayscale ImageBuffer to RGBA.
    // Convert grayscale ImageBuffer to RGBA using the style
    let (width, height) = img.dimensions();
    let img_rgba: RgbaImage = ImageBuffer::from_fn(width, height, |x, y| {
        let p = img.get_pixel(x, y)[0] as f32;
        if p == 0.0 {
            Rgba([0, 0, 0, 0])
        } else {
            get_color(p, &color_stops)
        }
    });

    // Encode the ImageBuffer to PNG
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

    // Build the response with a Content-Type header.
    let response = ([(header::CONTENT_TYPE, "image/png")], png_data);
    Ok(response)
}

// Function to find the closest colour for a given value
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
        .map(|(_, c)| c.clone())
        .unwrap_or(Rgba([0, 0, 0, 255]))
}
