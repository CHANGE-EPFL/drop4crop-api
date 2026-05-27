use crate::common::state::AppState;
use crate::config::Config;
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

fn parse_range(range_header: &str, total_size: i64) -> Option<(i64, i64)> {
    let s = range_header.strip_prefix("bytes=")?;
    let mut parts = s.splitn(2, '-');
    let start_str = parts.next()?.trim();
    let end_str = parts.next()?.trim();

    let start: i64 = if start_str.is_empty() {
        let suffix: i64 = end_str.parse().ok()?;
        (total_size - suffix).max(0)
    } else {
        start_str.parse().ok()?
    };

    let end: i64 = if end_str.is_empty() {
        total_size - 1
    } else {
        end_str.parse().ok()?
    };

    if start > end || start >= total_size {
        return None;
    }

    Some((start, end.min(total_size - 1)))
}

/// HEAD handler for COG files — returns Content-Length without body
#[utoipa::path(
    head,
    path = "/{filename}",
    params(
        ("filename" = String, Path, description = "Full filename with .tif extension"),
    ),
    responses(
        (status = 200, description = "File metadata (no body)"),
        (status = 404, description = "Layer not found"),
    ),
    summary = "COG file metadata (HEAD)",
    description = "Returns Content-Length and Accept-Ranges headers for GDAL /vsicurl/ compatibility."
)]
pub async fn head_cog_data(
    State(app_state): State<AppState>,
    Path(filename): Path<String>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    use crate::routes::layers::db::{Column, Entity as LayerEntity};

    let layer = LayerEntity::find()
        .filter(Column::Filename.eq(&filename))
        .one(&app_state.db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "Database error", "error": e.to_string()}))))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({"message": "Layer not found"}))))?;

    let file_size = if let Some(size) = layer.file_size {
        size
    } else {
        let data = storage::get_object(&app_state.config, layer.project_id, &filename).await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "S3 error", "error": e.to_string()}))))?;
        data.len() as i64
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/tiff")
        .header(header::CONTENT_LENGTH, file_size)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, "Content-Range, Accept-Ranges, Content-Length")
        .header(header::CONTENT_DISPOSITION, format!("inline; filename=\"{}\"", filename))
        .body(Body::empty())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "Response error", "error": e.to_string()}))))
}

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
    State(app_state): State<AppState>,
    Path(filename): Path<String>,
    Query(params): Query<DownloadQueryParams>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    get_layer_data(&app_state.db, &app_state.config, filename, params, headers).await
}

async fn get_layer_data(
    db: &DatabaseConnection,
    config: &Config,
    filename: String,
    params: DownloadQueryParams,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    use crate::routes::layers::db::{Column, Entity as LayerEntity};
    let layer = LayerEntity::find()
        .filter(Column::Filename.eq(&filename))
        .one(db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "Database error", "error": e.to_string()}))))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({"message": "Layer not found"}))))?;

    let project_id = layer.project_id;
    let range_header = headers.get(header::RANGE);

    if let Some(range) = range_header {
        let range_str = range.to_str().unwrap_or("");

        let total_size = if let Some(size) = layer.file_size {
            size
        } else {
            let full = storage::get_object(config, project_id, &filename).await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "S3 error", "error": e.to_string()}))))?;
            full.len() as i64
        };

        let (start, end) = parse_range(range_str, total_size)
            .ok_or_else(|| (StatusCode::RANGE_NOT_SATISFIABLE, Json(serde_json::json!({"message": "Invalid range"}))))?;

        let data = storage::get_object_range(config, project_id, &filename, range_str)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "S3 range error", "error": e.to_string()}))))?;

        let response = Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_TYPE, "image/tiff")
            .header(header::CONTENT_LENGTH, data.len())
            .header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", start, end, total_size))
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CACHE_CONTROL, "public, max-age=31536000")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, "Content-Range, Accept-Ranges, Content-Length")
            .header(header::CONTENT_DISPOSITION, format!("inline; filename=\"{}\"", filename))
            .body(Body::from(data))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "Response error", "error": e.to_string()}))))?;

        return Ok(response);
    }

    let data = storage::get_object(config, project_id, &filename).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "S3 error", "error": e.to_string()})))
    })?;

    let file_size = data.len();

    if params.minx.is_none() || params.miny.is_none() || params.maxx.is_none() || params.maxy.is_none() {
        let response = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "image/tiff")
            .header(header::CONTENT_LENGTH, file_size)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CACHE_CONTROL, "public, max-age=31536000")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, "Content-Range, Accept-Ranges, Content-Length")
            .header(header::CONTENT_DISPOSITION, format!("inline; filename=\"{}\"", filename))
            .body(Body::from(data))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "Response error", "error": e.to_string()}))))?;

        return Ok(response);
    }

    let minx = params.minx.unwrap();
    let miny = params.miny.unwrap();
    let maxx = params.maxx.unwrap();
    let maxy = params.maxy.unwrap();

    let cropped_data = crop_to_bbox(&data, minx, miny, maxx, maxy).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "Failed to crop raster", "error": e})))
    })?;

    let layer_name = filename.trim_end_matches(".tif");
    let cropped_filename = format!("{}_cropped.tif", layer_name);

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", cropped_filename))
        .body(Body::from(cropped_data))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "Response error", "error": e.to_string()}))))?;

    Ok(response)
}
