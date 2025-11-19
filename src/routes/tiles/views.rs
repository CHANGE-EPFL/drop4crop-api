use crate::routes::layers::db as layer;
use crate::routes::styles::db as style;
use crate::routes::tiles::utils::XYZTile;
use anyhow::Result;
use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use image::ImageBuffer;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, JsonValue, entity::prelude::*};
use serde::Deserialize;
use tokio_retry::{RetryIf, strategy::FixedInterval};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use tracing::{debug, error};

#[derive(Deserialize, ToSchema)]
pub struct Params {
    layer: String,
}

/// XYZ tiles router (for /xyz endpoint under /layers)
pub fn xyz_router(db: &DatabaseConnection) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(tile_handler))
        .with_state(db.clone())
}

#[utoipa::path(
    get,
    path = "/{z}/{x}/{y}",
    responses(
        (status = 200, description = "Tile image found", body = [u8], content_type = "image/png"),
        (status = 404, description = "Layer not found"),
        (status = 500, description = "Internal server error")
    ),
    params(
        ("z" = u32, description = "Zoom level"),
        ("x" = u32, description = "Tile x coordinate"),
        ("y" = u32, description = "Tile y coordinate"),
        ("layer" = String, Query, description = "Layer name")
    ),
    summary = "Get tile image",
    description = "Returns a tile image for the specified layer and coordinates."
)]
#[axum::debug_handler]
pub async fn tile_handler(
    Query(params): Query<Params>,
    Path((z, x, y)): Path<(u32, u32, u32)>,
    State(db): State<DatabaseConnection>,
) -> Result<impl IntoResponse, StatusCode> {
    let config = crate::config::Config::from_env();
    let max_tiles = 1 << z;
    if x >= max_tiles || y >= max_tiles {
        // Invalid tile coordinate - this is expected for out-of-bounds requests
        return Err(StatusCode::NOT_FOUND);
    }
    let xyz_tile = XYZTile { x, y, z };
    let retry_strategy = FixedInterval::from_millis(200).take(5);
    let img: ImageBuffer<image::Luma<u16>, Vec<u16>> = RetryIf::spawn(
        retry_strategy,
        || xyz_tile.get_one(&config, &params.layer),
        |e: &anyhow::Error| {
            error!(
                layer = %params.layer,
                z, x, y,
                error = %e,
                "Tile generation failed"
            );
            true
        },
    )
    .await
    .map_err(|e| {
        error!(
            layer = %params.layer,
            z, x, y,
            error = %e,
            "Failed to generate tile after 5 retries"
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Find the layer record by layer name.
    let layer_record = match layer::Entity::find()
        .filter(layer::Column::LayerName.eq(&params.layer))
        .one(&db)
        .await
        .map_err(|e| {
            error!(error = %e, "Database query error");
            StatusCode::INTERNAL_SERVER_ERROR
        })? {
        Some(rec) => rec,
        None => {
            debug!(layer = %params.layer, "No layer found");
            return Err(StatusCode::NOT_FOUND);
        }
    };

    // Load the related style record(s).
    let related_styles = layer_record
        .find_related(style::Entity)
        .all(&db)
        .await
        .map_err(|e| {
            error!(error = %e, "Database query error");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Attempt to extract the style from the first related record.
    let dbstyle: Option<JsonValue> = related_styles.into_iter().next().and_then(|s| s.style);

    // Apply the style to the image.
    let png_data = super::styling::style_layer(img, dbstyle).map_err(|e| {
        error!(error = %e, "Error applying style");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let response = ([(header::CONTENT_TYPE, "image/png")], png_data);
    Ok(response)
}
