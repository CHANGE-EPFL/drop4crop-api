use crate::entity::{layer, style};
use crate::tiles::XYZTile;
use anyhow::Result;
use axum::{
    extract::{Path, Query},
    http::{header, StatusCode},
    response::IntoResponse,
};
use image::ImageBuffer;
use sea_orm::{
    entity::prelude::*, ColumnTrait, Database, DatabaseConnection, EntityTrait, JsonValue,
};
use serde::Deserialize;
use tokio_retry::{strategy::FixedInterval, RetryIf};
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
    let img: ImageBuffer<image::Luma<u8>, Vec<u8>> = RetryIf::spawn(
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

    // Apply the style to the image.
    let png_data = crate::styling::style_layer(img, dbstyle).map_err(|e| {
        println!("[tile_handler] Error applying style: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let response = ([(header::CONTENT_TYPE, "image/png")], png_data);
    Ok(response)
}
