use crate::routes::layers::models::DownloadQueryParams;
use crate::routes::layers::utils::crop_to_bbox;
use crate::routes::tiles::storage;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::{
    body::Body,
    http::{HeaderMap, header},
    response::Response,
};
use hyper::StatusCode;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

/// S3-compatible COG endpoint - serves GeoTIFF files with HTTP Range support
/// Path format: /api/layers/cog/{filename} (e.g., /api/layers/cog/barley_pcr-globwb_hadgem2-es_rcp26_vwc_2080.tif)
#[utoipa::path(
    get,
    path = "/{filename}",
    params(
        ("filename" = String, Path, description = "Full filename with .tif extension"),
        DownloadQueryParams
    ),
    responses(
        (status = 200, description = "TIFF file (full content)", content_type = "image/tiff"),
        (status = 206, description = "TIFF file (partial content for COG streaming)", content_type = "image/tiff"),
        (status = 404, description = "Layer not found"),
        (status = 500, description = "Internal server error")
    ),
    summary = "S3-compatible COG endpoint",
    description = "Serves Cloud Optimized GeoTIFF files with HTTP Range request support for streaming. Compatible with GDAL /vsicurl/ and QGIS."
)]
pub async fn get_cog_data(
    State(db): State<DatabaseConnection>,
    Path(filename): Path<String>,
    Query(params): Query<DownloadQueryParams>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    get_layer_data(db, filename, params, headers).await
}

/// Shared function for fetching layer data (used by both legacy /download and new /data endpoints)
async fn get_layer_data(
    db: DatabaseConnection,
    filename: String,
    params: DownloadQueryParams,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let config = crate::config::Config::from_env();

    // Verify layer exists in database
    use crate::routes::layers::db::{Column, Entity as LayerEntity};
    let layer = LayerEntity::find()
        .filter(Column::Filename.eq(&filename))
        .one(&db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "message": "Database error",
                    "error": e.to_string()
                })),
            )
        })?;

    if layer.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "message": "Layer not found"
            })),
        ));
    }

    // Check for Range header (HTTP Range Request for COG streaming)
    let range_header = headers.get(header::RANGE);

    // Fetch the file from S3
    let data = if let Some(range) = range_header {
        // Parse range header and fetch only requested bytes from S3
        storage::get_object_range(&config, &filename, range.to_str().unwrap_or(""))
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "message": "Failed to fetch file range from S3",
                        "error": e.to_string()
                    })),
                )
            })?
    } else {
        // Fetch entire file
        storage::get_object(&config, &filename).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "message": "Failed to fetch file from S3",
                    "error": e.to_string()
                })),
            )
        })?
    };

    let file_size = data.len();

    // If no cropping parameters provided, return the file (full or range)
    if params.minx.is_none()
        || params.miny.is_none()
        || params.maxx.is_none()
        || params.maxy.is_none()
    {
        let mut response_builder = Response::builder();

        if range_header.is_some() {
            // Return 206 Partial Content for range requests
            response_builder = response_builder
                .status(StatusCode::PARTIAL_CONTENT)
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes 0-{}/{}", file_size - 1, file_size),
                )
                .header(header::ACCEPT_RANGES, "bytes");
        } else {
            response_builder = response_builder.status(StatusCode::OK);
        }

        let response = response_builder
            .header(header::CONTENT_TYPE, "image/tiff")
            .header(header::CONTENT_LENGTH, file_size)
            .header(header::CACHE_CONTROL, "public, max-age=31536000")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(
                header::ACCESS_CONTROL_EXPOSE_HEADERS,
                "Content-Range, Accept-Ranges",
            )
            .header(
                header::CONTENT_DISPOSITION,
                format!("inline; filename=\"{}\"", filename),
            )
            .body(Body::from(data))
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "message": "Failed to create response",
                        "error": e.to_string()
                    })),
                )
            })?;

        return Ok(response);
    }

    // Crop the raster to the specified bounding box
    let minx = params.minx.unwrap();
    let miny = params.miny.unwrap();
    let maxx = params.maxx.unwrap();
    let maxy = params.maxy.unwrap();

    let cropped_data = crop_to_bbox(&data, minx, miny, maxx, maxy).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "message": "Failed to crop raster",
                "error": e
            })),
        )
    })?;

    // Extract layer name from filename (remove .tif extension)
    let layer_name = filename.trim_end_matches(".tif");
    let cropped_filename = format!("{}_cropped.tif", layer_name);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", cropped_filename),
        )
        .body(Body::from(cropped_data))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "message": "Failed to create response",
                    "error": e.to_string()
                })),
            )
        })?;

    Ok(response)
}
