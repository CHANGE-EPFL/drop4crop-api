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

/// Parse a tile coordinate from a string, handling both integers and floats.
/// Floats are truncated toward zero. Negative values are rejected.
fn parse_tile_coord(s: &str) -> Result<u32, StatusCode> {
    // First try parsing as u32 directly (fastest path for normal integer requests)
    if let Ok(v) = s.parse::<u32>() {
        return Ok(v);
    }
    // If that fails, try parsing as f64 and truncate
    // This handles browsers that send float coordinates like "3.7"
    let f = s.parse::<f64>().map_err(|_| StatusCode::BAD_REQUEST)?;

    // Reject negative values - tile coordinates must be non-negative
    if f < 0.0 {
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(f.trunc() as u32)
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
        ("z" = String, description = "Zoom level"),
        ("x" = String, description = "Tile x coordinate"),
        ("y" = String, description = "Tile y coordinate"),
        ("layer" = String, Query, description = "Layer name")
    ),
    summary = "Get tile image",
    description = "Returns a tile image for the specified layer and coordinates."
)]
#[axum::debug_handler]
pub async fn tile_handler(
    Query(params): Query<Params>,
    Path((z_str, x_str, y_str)): Path<(String, String, String)>,
    State(db): State<DatabaseConnection>,
) -> Result<impl IntoResponse, StatusCode> {
    // Parse coordinates, handling both integers and floats (truncating floats)
    let z = parse_tile_coord(&z_str)?;
    let x = parse_tile_coord(&x_str)?;
    let y = parse_tile_coord(&y_str)?;

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

    // Attempt to extract the style and interpolation_type from the first related record.
    let (dbstyle, interpolation_type): (Option<JsonValue>, Option<String>) = related_styles
        .into_iter()
        .next()
        .map(|s| (s.style, Some(s.interpolation_type)))
        .unwrap_or((None, None));

    // Apply the style to the image.
    let png_data = super::styling::style_layer(img, dbstyle, interpolation_type.as_deref()).map_err(|e| {
        error!(error = %e, "Error applying style");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let response = ([(header::CONTENT_TYPE, "image/png")], png_data);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tile_coord_integer() {
        assert_eq!(parse_tile_coord("0").unwrap(), 0);
        assert_eq!(parse_tile_coord("1").unwrap(), 1);
        assert_eq!(parse_tile_coord("123").unwrap(), 123);
        assert_eq!(parse_tile_coord("4294967295").unwrap(), u32::MAX); // max u32
    }

    #[test]
    fn test_parse_tile_coord_float_truncation() {
        // Floats should be truncated (not rounded)
        assert_eq!(parse_tile_coord("3.7").unwrap(), 3);
        assert_eq!(parse_tile_coord("4.2").unwrap(), 4);
        assert_eq!(parse_tile_coord("5.9").unwrap(), 5);
        assert_eq!(parse_tile_coord("0.0").unwrap(), 0);
        assert_eq!(parse_tile_coord("0.999").unwrap(), 0);
        assert_eq!(parse_tile_coord("10.5").unwrap(), 10);
    }

    #[test]
    fn test_parse_tile_coord_invalid() {
        assert_eq!(parse_tile_coord("abc").unwrap_err(), StatusCode::BAD_REQUEST);
        assert_eq!(parse_tile_coord("").unwrap_err(), StatusCode::BAD_REQUEST);
        assert_eq!(parse_tile_coord("12abc").unwrap_err(), StatusCode::BAD_REQUEST);
        assert_eq!(parse_tile_coord("abc12").unwrap_err(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_parse_tile_coord_negative() {
        // All negative values should be rejected - tile coordinates must be non-negative
        assert_eq!(parse_tile_coord("-1").unwrap_err(), StatusCode::BAD_REQUEST);
        assert_eq!(parse_tile_coord("-0.5").unwrap_err(), StatusCode::BAD_REQUEST);
        assert_eq!(parse_tile_coord("-0.001").unwrap_err(), StatusCode::BAD_REQUEST);
        assert_eq!(parse_tile_coord("-100").unwrap_err(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_parse_tile_coord_zero_is_valid() {
        // Zero is a valid tile coordinate (e.g., at zoom 0 the only tile is 0/0/0)
        assert_eq!(parse_tile_coord("0").unwrap(), 0);
        assert_eq!(parse_tile_coord("0.0").unwrap(), 0);
        assert_eq!(parse_tile_coord("0.9").unwrap(), 0); // truncates to 0
    }
}
