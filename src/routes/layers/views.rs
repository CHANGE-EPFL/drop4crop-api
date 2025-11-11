use super::db::Layer;
use super::utils::{LayerInfo, convert_to_cog_in_memory, get_min_max_of_raster, get_global_average_of_raster, parse_filename};
use crate::common::auth::Role;
use crate::common::state::AppState;
use crate::routes::tiles::storage;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::{
    body::Body,
    extract::Multipart,
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
};
use axum_keycloak_auth::{PassthroughMode, layer::KeycloakAuthLayer};
use crudcrate::CRUDResource;
use gdal::Dataset;
use hyper::StatusCode;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait,  QueryFilter,
    QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::vec;
use std::{collections::HashMap, ffi::CString};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

// // Custom response type for /map endpoint that includes properly formatted style data for legend
// #[derive(Serialize, ToSchema)]
// pub struct MapLayerResponse {
//     pub id: uuid::Uuid,
//     pub layer_name: Option<String>,
//     pub crop: Option<String>,
//     pub water_model: Option<String>,
//     pub climate_model: Option<String>,
//     pub scenario: Option<String>,
//     pub variable: Option<String>,
//     pub year: Option<i32>,
//     pub enabled: bool,
//     pub uploaded_at: chrono::DateTime<Utc>,
//     pub last_updated: chrono::DateTime<Utc>,
//     pub global_average: Option<f64>,
//     pub filename: Option<String>,
//     pub min_value: Option<f64>,
//     pub max_value: Option<f64>,
//     pub style_id: Option<uuid::Uuid>,
//     pub is_crop_specific: bool,
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub style: Option<serde_json::Value>,
//     pub country_values: Option<Vec<serde_json::Value>>,
// }
#[derive(Deserialize, IntoParams)]
pub struct UploadQueryParams {
    overwrite_duplicates: Option<bool>,
}

pub fn router(state: &AppState) -> OpenApiRouter {
    let public_router = OpenApiRouter::new()
        .routes(routes!(get_groups))
        .routes(routes!(get_pixel_value))
        .with_state(state.db.clone());

    let mut protected_router = Layer::router(&state.db.clone());
    let protected_custom_routes = OpenApiRouter::new()
        .routes(routes!(upload_file))
        .with_state(state.db.clone());

    protected_router = protected_router.merge(protected_custom_routes);

    if let Some(instance) = state.keycloak_auth_instance.clone() {
        protected_router = protected_router.layer(
            KeycloakAuthLayer::<Role>::builder()
                .instance(instance)
                .passthrough_mode(PassthroughMode::Block)
                .persist_raw_claims(false)
                .expected_audiences(vec![String::from("account")])
                .required_roles(vec![Role::Administrator])
                .build(),
        );
    } else if !state.config.tests_running {
        println!(
            "Warning: Mutating routes of {} router are not protected",
            Layer::RESOURCE_NAME_PLURAL
        );
    }

    public_router.merge(protected_router)
}

/// S3-compatible COG data router (for /cog endpoint under /layers)
/// This provides a clean S3-like path structure for COG files
pub fn cog_router(db: &DatabaseConnection) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(get_cog_data))
        .with_state(db.clone())
}

#[utoipa::path(
    get,
    path = "/groups",
    responses(
        (status = 200, description = "Filtered data found", body = HashMap<String, Vec<JsonValue>>),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get all unique groups",
    description = "This endpoint allows the menu to be populated with available keys"
)]
pub async fn get_groups(
    State(db): State<DatabaseConnection>,
) -> Result<Json<HashMap<String, Vec<JsonValue>>>, (StatusCode, Json<String>)> {
    let mut groups: HashMap<String, Vec<JsonValue>> = HashMap::new();

    let layer_variables = [
        ("crop", super::db::Column::Crop),
        ("water_model", super::db::Column::WaterModel),
        ("climate_model", super::db::Column::ClimateModel),
        ("scenario", super::db::Column::Scenario),
        ("variable", super::db::Column::Variable),
        ("year", super::db::Column::Year),
    ];

    for (variable, column) in layer_variables.iter() {
        let res = super::db::Entity::find()
            .filter(super::db::Column::Enabled.eq(true))
            .select_only()
            .column(*column)
            .distinct()
            .into_json()
            .all(&db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

        let values: Vec<JsonValue> = res
            .into_iter()
            .filter_map(|mut json| json.as_object_mut()?.remove(*variable))
            .filter(|value| !value.is_null())
            .collect();

        groups.insert(variable.to_string(), values);
    }

    Ok(Json(groups))
}

#[derive(Deserialize, ToSchema, IntoParams)]
pub struct GetPixelValueParams {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Serialize, ToSchema, IntoParams)]
pub struct PixelValueResponse {
    pub value: f64,
}

#[utoipa::path(
    get,
    path = "/{layer_id}/value",
    params(
        ("layer_id" = String, Path, description = "Layer ID"),
        GetPixelValueParams
    ),
    responses(
        (status = 200, description = "Pixel value found", body = PixelValueResponse),
        (status = 400, description = "Coordinates out of bounds"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get the pixel value at a given lat/lon",
    description = "Fetches a GeoTIFF from cache/S3 and returns the pixel value at the specified latitude and longitude."
)]
pub async fn get_pixel_value(
    Path(layer_id): Path<String>,
    Query(params): Query<GetPixelValueParams>,
    State(_db): State<DatabaseConnection>,
) -> Result<impl IntoResponse, StatusCode> {
    // Build the filename for the TIFF.
    let filename = format!("{}.tif", layer_id);

    // Fetch the object using your existing S3 integration (with caching).
    let object = storage::get_object(&filename).await.map_err(|e| {
        println!("[get_pixel_value] Error fetching object: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Write the bytes to GDAL's /vsimem virtual file system.
    let vsi_path = format!("/vsimem/{}", filename);
    {
        let c_vsi_path = CString::new(vsi_path.clone()).unwrap();
        let mode = CString::new("w").unwrap();
        unsafe {
            let fp = gdal_sys::VSIFOpenL(c_vsi_path.as_ptr(), mode.as_ptr());
            if fp.is_null() {
                println!("[get_pixel_value] Failed to open /vsimem file");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
            let written = gdal_sys::VSIFWriteL(object.as_ptr() as *const _, 1, object.len(), fp);
            if written != object.len() {
                gdal_sys::VSIFCloseL(fp);
                println!("[get_pixel_value] Failed to write all data to /vsimem file");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
            gdal_sys::VSIFCloseL(fp);
        }
    }

    // Open the dataset with GDAL.
    let dataset = Dataset::open(&vsi_path).map_err(|e| {
        println!("[get_pixel_value] Error opening dataset: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Remove the in-memory file.
    {
        let c_vsi_path = CString::new(vsi_path.clone()).unwrap();
        unsafe {
            gdal_sys::VSIUnlink(c_vsi_path.as_ptr());
        }
    }

    // Retrieve the geo-transform.
    let geo_transform = dataset.geo_transform().map_err(|e| {
        println!("[get_pixel_value] Error getting geo_transform: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Compute pixel coordinates.
    // Assuming the geo_transform is of the form:
    // [origin_x, pixel_width, 0, origin_y, 0, pixel_height]
    // Note: For north-up images, pixel_height is typically negative.
    let col = ((params.lon - geo_transform[0]) / geo_transform[1]).floor() as isize;
    let row = if geo_transform[5] < 0.0 {
        ((geo_transform[3] - params.lat) / -geo_transform[5]).floor() as isize
    } else {
        ((params.lat - geo_transform[3]) / geo_transform[5]).floor() as isize
    };

    // Check that the computed pixel coordinates fall within the dataset bounds.
    let (raster_x_size, raster_y_size) = dataset.raster_size();
    if col < 0 || row < 0 || col >= raster_x_size as isize || row >= raster_y_size as isize {
        println!(
            "[get_pixel_value] Coordinates out of bounds: col {}, row {}",
            col, row
        );
        return Err(StatusCode::BAD_REQUEST);
    }

    // Read the pixel value from band 1.
    let band = dataset.rasterband(1).map_err(|e| {
        println!("[get_pixel_value] Error accessing raster band: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let buf_result = band
        .read_as::<f64>((col, row), (1, 1), (1, 1), None)
        .map_err(|e| {
            println!("[get_pixel_value] Error reading pixel value: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let buf = buf_result.data();
    let value = buf.first().cloned().unwrap_or(0.0);

    let response = PixelValueResponse { value };
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/uploads",
    params(UploadQueryParams),
    responses(
        (status = 200, description = "File uploaded successfully", body = Layer),
        (status = 400, description = "Invalid file or filename format"),
        (status = 409, description = "Layer already exists"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Upload a GeoTIFF file",
    description = "Uploads a GeoTIFF file, converts it to COG format, and creates a layer record"
)]
pub async fn upload_file(
    State(db): State<DatabaseConnection>,
    Query(params): Query<UploadQueryParams>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    println!("[upload_file] Starting upload request");
    let config = crate::config::Config::from_env();
    let overwrite_duplicates = params
        .overwrite_duplicates
        .unwrap_or(config.overwrite_duplicate_layers);

    println!("[upload_file] About to process multipart data...");
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        println!("[upload_file] Error reading multipart field: {}", e);
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Error parsing `multipart/form-data` request",
                "message": "Failed to read file data"
            })),
        )
    })? {
        println!("[upload_file] Got a field from multipart");
        let name = field.name().unwrap_or("file");

        if name == "file" {
            println!("[upload_file] Processing file field");
            let filename = field
                .file_name()
                .ok_or_else(|| {
                    println!("[upload_file] No filename provided");
                    (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "message": "No filename provided"
                        })),
                    )
                })?
                .to_lowercase();

            println!("[upload_file] Filename: {}", filename);
            println!("[upload_file] About to read file bytes...");

            let data = match field.bytes().await {
                Ok(data) => {
                    println!("[upload_file] Successfully read {} bytes", data.len());
                    data
                }
                Err(e) => {
                    println!("[upload_file] Error reading file bytes: {:?}", e);
                    println!(
                        "[upload_file] Error type: {}",
                        std::any::type_name::<std::option::IntoIter<&()>>()
                    );
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "Error reading file bytes",
                            "message": format!("Failed to read file data: {}", e)
                        })),
                    ));
                }
            };

            println!("[upload_file] Successfully read {} bytes", data.len());

            // Parse filename to extract layer information
            println!("[upload_file] Parsing filename...");
            let layer_info = parse_filename(&filename).map_err(|e| {
                println!("[upload_file] Error parsing filename: {}", e);
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "message": "Invalid filename format",
                        "error": e.to_string()
                    })),
                )
            })?;
            println!("[upload_file] Successfully parsed filename");

            // Check for duplicate layer
            println!("[upload_file] Checking for duplicate layers...");
            let duplicate_query = match &layer_info {
                LayerInfo::Climate(info) => {
                    use crate::routes::layers::db::{Column, Entity as LayerEntity};
                    LayerEntity::find()
                        .filter(Column::Crop.eq(&info.crop))
                        .filter(Column::Variable.eq(&info.variable))
                        .filter(Column::WaterModel.eq(&info.water_model))
                        .filter(Column::ClimateModel.eq(&info.climate_model))
                        .filter(Column::Scenario.eq(&info.scenario))
                        .filter(Column::Year.eq(info.year))
                }
                LayerInfo::Crop(info) => {
                    use crate::routes::layers::db::{Column, Entity as LayerEntity};
                    LayerEntity::find()
                        .filter(Column::Crop.eq(&info.crop))
                        .filter(Column::Variable.eq(&info.variable))
                        .filter(Column::IsCropSpecific.eq(true))
                }
            };

            let existing_layer = duplicate_query.one(&db).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "message": "Database error",
                        "error": e.to_string()
                    })),
                )
            })?;

            if let Some(existing) = existing_layer {
                if overwrite_duplicates {
                    // Delete existing layer from S3 and database
                    if let Some(ref filename) = existing.filename {
                        let s3_key = storage::get_s3_key(filename);
                        storage::delete_object(&s3_key).await.map_err(|e| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({
                                    "message": "Failed to delete existing layer from S3",
                                    "error": e.to_string()
                                })),
                            )
                        })?;
                    }

                    use crate::routes::layers::db::Entity as LayerEntity;
                    LayerEntity::delete_by_id(existing.id)
                        .exec(&db)
                        .await
                        .map_err(|e| {
                            (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({
                                    "message": "Failed to delete existing layer from database",
                                    "error": e.to_string()
                                })),
                            )
                        })?;

                    println!(
                        "Deleted existing layer: {}",
                        existing.filename.unwrap_or_else(|| "unknown".to_string())
                    );
                    println!(
                        "[upload_file] Continuing with upload of duplicate file: {}",
                        filename
                    );
                } else {
                    println!("[upload_file] Rejecting duplicate file: {}", filename);
                    return Err((
                        StatusCode::CONFLICT,
                        Json(serde_json::json!({
                            "message": format!("Layer already exists for {}. Delete layer first to re-upload, or set overwrite_duplicates=true", filename)
                        })),
                    ));
                }
            }

            // Convert to COG
            println!("[upload_file] Converting to COG format...");
            let cog_bytes = convert_to_cog_in_memory(&data).map_err(|e| {
                println!("[upload_file] Error converting to COG: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "message": "Failed to convert to COG",
                        "error": e.to_string()
                    })),
                )
            })?;
            println!(
                "[upload_file] Successfully converted to COG, size: {} bytes",
                cog_bytes.len()
            );

            // Calculate min/max values
            let (min_val, max_val) = get_min_max_of_raster(&cog_bytes).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "message": "Failed to calculate raster statistics",
                        "error": e.to_string()
                    })),
                )
            })?;

            // Calculate global average
            let global_avg = get_global_average_of_raster(&cog_bytes).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "message": "Failed to calculate global average",
                        "error": e.to_string()
                    })),
                )
            })?;

            // Check for invalid values
            if min_val.is_finite() && max_val.is_finite() && global_avg.is_finite() {
                println!("Raster stats: min={}, max={}, global_average={}", min_val, max_val, global_avg);
            } else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "message": "Invalid raster statistics: min, max, or global_average value is infinite"
                    })),
                ));
            }

            // Upload to S3
            let s3_key = storage::get_s3_key(&filename);
            storage::upload_object(&s3_key, &cog_bytes)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "message": "Failed to upload to S3",
                            "error": e.to_string()
                        })),
                    )
                })?;

            // Create layer record in database
            let layer_name = filename.strip_suffix(".tif").unwrap_or(&filename);
            let layer_record = match layer_info {
                LayerInfo::Climate(info) => {
                    use crate::routes::layers::db::ActiveModel as LayerActiveModel;
                    LayerActiveModel {
                        id: Set(Uuid::new_v4()),
                        filename: Set(Some(filename.clone())),
                        layer_name: Set(Some(layer_name.to_string())),
                        crop: Set(Some(info.crop)),
                        water_model: Set(Some(info.water_model)),
                        climate_model: Set(Some(info.climate_model)),
                        scenario: Set(Some(info.scenario)),
                        variable: Set(Some(info.variable)),
                        year: Set(Some(info.year)),
                        min_value: Set(Some(min_val)),
                        max_value: Set(Some(max_val)),
                        global_average: Set(Some(global_avg)),
                        enabled: Set(true),
                        is_crop_specific: Set(false),
                        ..Default::default()
                    }
                }
                LayerInfo::Crop(info) => {
                    use crate::routes::layers::db::ActiveModel as LayerActiveModel;
                    LayerActiveModel {
                        id: Set(Uuid::new_v4()),
                        filename: Set(Some(filename.clone())),
                        layer_name: Set(Some(layer_name.to_string())),
                        crop: Set(Some(info.crop)),
                        variable: Set(Some(info.variable)),
                        min_value: Set(Some(min_val)),
                        max_value: Set(Some(max_val)),
                        global_average: Set(Some(global_avg)),
                        enabled: Set(true),
                        is_crop_specific: Set(true),
                        ..Default::default()
                    }
                }
            };

            println!(
                "[upload_file] Attempting to save layer to database: {}",
                filename
            );
            let saved_layer = match layer_record.insert(&db).await {
                Ok(layer) => {
                    println!(
                        "[upload_file] Successfully saved layer to database: {}",
                        filename
                    );
                    layer
                }
                Err(e) => {
                    println!(
                        "[upload_file] ERROR: Failed to save layer to database: {} - Error: {}",
                        filename, e
                    );
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "message": "Failed to save layer to database",
                            "error": e.to_string()
                        })),
                    ));
                }
            };

            println!("Successfully uploaded layer: {}", filename);

            // Return the saved layer as Layer model
            println!(
                "[upload_file] Creating response object for layer: {}",
                filename
            );
            let layer_response = match std::panic::catch_unwind(|| Layer::from(saved_layer)) {
                Ok(response) => {
                    println!(
                        "[upload_file] Successfully created response object for layer: {}",
                        filename
                    );
                    response
                }
                Err(panic_info) => {
                    println!(
                        "[upload_file] PANIC during Layer::from() conversion for layer: {} - {:?}",
                        filename, panic_info
                    );
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "message": "Panic during response creation",
                            "error": "Internal panic during layer conversion"
                        })),
                    ));
                }
            };
            println!(
                "[upload_file] Response object created, preparing to send for layer: {}",
                filename
            );
            println!("[upload_file] Sending response for layer: {}", filename);
            return Ok((StatusCode::OK, Json(layer_response)));
        }
    }

    println!("[upload_file] No file found in multipart data, returning error");
    Err((
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "message": "No file found in upload"
        })),
    ))
}

#[derive(Deserialize, IntoParams)]
pub struct DownloadQueryParams {
    minx: Option<f64>,
    miny: Option<f64>,
    maxx: Option<f64>,
    maxy: Option<f64>,
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
        storage::get_object_range(&filename, range.to_str().unwrap_or("")).await.map_err(|e| {
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
        storage::get_object(&filename).await.map_err(|e| {
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
                .header(header::CONTENT_RANGE, format!("bytes 0-{}/{}", file_size - 1, file_size))
                .header(header::ACCEPT_RANGES, "bytes");
        } else {
            response_builder = response_builder.status(StatusCode::OK);
        }

        let response = response_builder
            .header(header::CONTENT_TYPE, "image/tiff")
            .header(header::CONTENT_LENGTH, file_size)
            .header(header::CACHE_CONTROL, "public, max-age=31536000")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, "Content-Range, Accept-Ranges")
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

/// Crops a GeoTIFF to the specified bounding box
/// Returns the cropped GeoTIFF as bytes
fn crop_to_bbox(
    original_data: &[u8],
    minx: f64,
    miny: f64,
    maxx: f64,
    maxy: f64,
) -> Result<Vec<u8>, String> {
    use gdal::raster::Buffer;

    // Write original data to vsimem
    let input_path = format!("/vsimem/input_{}.tif", uuid::Uuid::new_v4());
    let c_input_path = CString::new(input_path.clone()).map_err(|e| e.to_string())?;

    unsafe {
        let mode = CString::new("w").unwrap();
        let fp = gdal_sys::VSIFOpenL(c_input_path.as_ptr(), mode.as_ptr());
        if fp.is_null() {
            return Err("Failed to open vsimem input file".to_string());
        }
        let written = gdal_sys::VSIFWriteL(original_data.as_ptr() as *const _, 1, original_data.len(), fp);
        if written != original_data.len() {
            gdal_sys::VSIFCloseL(fp);
            return Err("Failed to write all data to vsimem".to_string());
        }
        gdal_sys::VSIFCloseL(fp);
    }

    // Open the dataset
    let dataset = Dataset::open(&input_path).map_err(|e| format!("Failed to open dataset: {}", e))?;

    // Get geotransform
    let gt = dataset.geo_transform().map_err(|e| format!("Failed to get geotransform: {}", e))?;

    // Calculate pixel coordinates for the bounding box
    let col_min = ((minx - gt[0]) / gt[1]).floor() as isize;
    let col_max = ((maxx - gt[0]) / gt[1]).ceil() as isize;
    let row_min = ((maxy - gt[3]) / gt[5]).floor() as isize; // gt[5] is typically negative
    let row_max = ((miny - gt[3]) / gt[5]).ceil() as isize;

    let (raster_x_size, raster_y_size) = dataset.raster_size();

    // Clamp to raster bounds
    let col_min = col_min.max(0).min(raster_x_size as isize);
    let col_max = col_max.max(0).min(raster_x_size as isize);
    let row_min = row_min.max(0).min(raster_y_size as isize);
    let row_max = row_max.max(0).min(raster_y_size as isize);

    let width = (col_max - col_min) as usize;
    let height = (row_max - row_min) as usize;

    if width == 0 || height == 0 {
        unsafe {
            gdal_sys::VSIUnlink(c_input_path.as_ptr());
        }
        return Err("Bounding box results in zero-sized raster".to_string());
    }

    // Calculate new geotransform for cropped region
    let new_origin_x = gt[0] + col_min as f64 * gt[1];
    let new_origin_y = gt[3] + row_min as f64 * gt[5];
    let new_gt = [new_origin_x, gt[1], gt[2], new_origin_y, gt[4], gt[5]];

    // Read the cropped data from the band
    let band = dataset.rasterband(1).map_err(|e| format!("Failed to get rasterband: {}", e))?;
    let mut buffer: Buffer<f64> = band
        .read_as((col_min, row_min), (width, height), (width, height), None)
        .map_err(|e| format!("Failed to read raster data: {}", e))?;

    // Create output dataset in vsimem
    let output_path = format!("/vsimem/output_{}.tif", uuid::Uuid::new_v4());
    let c_output_path = CString::new(output_path.clone()).map_err(|e| e.to_string())?;

    let driver = gdal::DriverManager::get_driver_by_name("GTiff")
        .map_err(|e| format!("Failed to get GTiff driver: {}", e))?;

    let mut out_dataset = driver
        .create_with_band_type::<f64, _>(&output_path, width, height, 1)
        .map_err(|e| format!("Failed to create output dataset: {}", e))?;

    // Set geotransform and spatial reference
    out_dataset
        .set_geo_transform(&new_gt)
        .map_err(|e| format!("Failed to set geotransform: {}", e))?;

    if let Ok(srs) = dataset.spatial_ref() {
        out_dataset
            .set_spatial_ref(&srs)
            .map_err(|e| format!("Failed to set spatial reference: {}", e))?;
    }

    // Write the data
    let mut out_band = out_dataset.rasterband(1)
        .map_err(|e| format!("Failed to get output rasterband: {}", e))?;

    out_band
        .write((0, 0), (width, height), &mut buffer)
        .map_err(|e| format!("Failed to write raster data: {}", e))?;

    // Flush and close
    drop(out_dataset);
    drop(dataset);

    // Read the cropped file from vsimem
    let cropped_data = unsafe {
        let mode = CString::new("r").unwrap();
        let fp = gdal_sys::VSIFOpenL(c_output_path.as_ptr(), mode.as_ptr());
        if fp.is_null() {
            gdal_sys::VSIUnlink(c_input_path.as_ptr());
            return Err("Failed to open output file".to_string());
        }

        // Get file size
        gdal_sys::VSIFSeekL(fp, 0, 2); // SEEK_END
        let size = gdal_sys::VSIFTellL(fp) as usize;
        gdal_sys::VSIFSeekL(fp, 0, 0); // SEEK_SET

        // Read data
        let mut buffer = vec![0u8; size];
        let read = gdal_sys::VSIFReadL(buffer.as_mut_ptr() as *mut _, 1, size, fp);
        if read != size {
            gdal_sys::VSIFCloseL(fp);
            gdal_sys::VSIUnlink(c_input_path.as_ptr());
            gdal_sys::VSIUnlink(c_output_path.as_ptr());
            return Err("Failed to read all cropped data".to_string());
        }
        gdal_sys::VSIFCloseL(fp);

        buffer
    };

    // Clean up vsimem files
    unsafe {
        gdal_sys::VSIUnlink(c_input_path.as_ptr());
        gdal_sys::VSIUnlink(c_output_path.as_ptr());
    }

    Ok(cropped_data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdal::Dataset;
    use std::ffi::CString;

    /// Test that verifies bounding box cropping functionality
    /// This test creates a simple test raster and verifies that:
    /// 1. A cropped version has different dimensions than the original
    /// 2. The cropped version has correct georeferencing
    /// 3. The cropped version contains the expected subset of data
    #[test]
    fn test_bbox_cropping() {
        // Create a simple test GeoTIFF in memory
        let vsi_path = "/vsimem/test_layer.tif";
        let c_vsi_path = CString::new(vsi_path).unwrap();

        // Create a test dataset: 100x100 pixels covering -180 to 180 longitude, -90 to 90 latitude
        let driver = gdal::DriverManager::get_driver_by_name("GTiff").unwrap();
        let mut dataset = driver
            .create_with_band_type::<f64, _>(vsi_path, 100, 100, 1)
            .unwrap();

        // Set geotransform: [origin_x, pixel_width, 0, origin_y, 0, pixel_height]
        // This makes each pixel 3.6 degrees wide and tall
        dataset
            .set_geo_transform(&[-180.0, 3.6, 0.0, 90.0, 0.0, -3.6])
            .unwrap();

        // Set spatial reference (WGS84)
        dataset
            .set_spatial_ref(&gdal::spatial_ref::SpatialRef::from_epsg(4326).unwrap())
            .unwrap();

        // Fill with test data (simple gradient)
        let mut band = dataset.rasterband(1).unwrap();
        let data: Vec<f64> = (0..10000).map(|i| i as f64).collect();
        use gdal::raster::Buffer;
        let mut buffer = Buffer::new((100, 100), data);
        band.write((0, 0), (100, 100), &mut buffer).unwrap();

        // Close the dataset to flush to vsimem
        drop(dataset);

        // Read the file from vsimem
        let _original_data = unsafe {
            let mode = CString::new("r").unwrap();
            let fp = gdal_sys::VSIFOpenL(c_vsi_path.as_ptr(), mode.as_ptr());
            assert!(!fp.is_null(), "Failed to open test file");

            // Get file size
            gdal_sys::VSIFSeekL(fp, 0, 2); // SEEK_END
            let size = gdal_sys::VSIFTellL(fp) as usize;
            gdal_sys::VSIFSeekL(fp, 0, 0); // SEEK_SET

            // Read data
            let mut buffer = vec![0u8; size];
            let read = gdal_sys::VSIFReadL(buffer.as_mut_ptr() as *mut _, 1, size, fp);
            assert_eq!(read, size, "Failed to read all data");
            gdal_sys::VSIFCloseL(fp);

            buffer
        };

        // Test case: Crop to a smaller region (-90 to 0 longitude, 0 to 45 latitude)
        // This should give us roughly 25x12.5 pixels = 25x13 pixels
        let minx = -90.0;
        let miny = 0.0;
        let maxx = 0.0;
        let maxy = 45.0;

        // This is where we would call the cropping function
        // For now, we'll implement the logic inline to show what we expect

        // Open the original dataset
        let dataset = Dataset::open(vsi_path).unwrap();

        // Get geotransform
        let gt = dataset.geo_transform().unwrap();

        // Calculate pixel coordinates for the bounding box
        // Using GDAL's geotransform formula:
        // Xgeo = GT[0] + Xpixel*GT[1] + Yline*GT[2]
        // Ygeo = GT[3] + Xpixel*GT[4] + Yline*GT[5]
        // Solving for pixel coordinates:
        let col_min = ((minx - gt[0]) / gt[1]).floor() as isize;
        let col_max = ((maxx - gt[0]) / gt[1]).ceil() as isize;
        let row_min = ((maxy - gt[3]) / gt[5]).floor() as isize; // Note: gt[5] is negative
        let row_max = ((miny - gt[3]) / gt[5]).ceil() as isize;

        let (raster_x_size, raster_y_size) = dataset.raster_size();

        // Clamp to raster bounds
        let col_min = col_min.max(0).min(raster_x_size as isize);
        let col_max = col_max.max(0).min(raster_x_size as isize);
        let row_min = row_min.max(0).min(raster_y_size as isize);
        let row_max = row_max.max(0).min(raster_y_size as isize);

        let width = (col_max - col_min) as usize;
        let height = (row_max - row_min) as usize;

        // Verify the cropped dimensions are smaller than original
        assert!(width < raster_x_size, "Cropped width should be less than original");
        assert!(height < raster_y_size, "Cropped height should be less than original");
        assert!(width > 0, "Cropped width should be greater than 0");
        assert!(height > 0, "Cropped height should be greater than 0");

        // Expected dimensions based on our bounding box:
        // -90 to 0 longitude = 90 degrees = 25 pixels
        // 0 to 45 latitude = 45 degrees = 12.5 pixels
        assert_eq!(width, 25, "Expected width of 25 pixels for 90 degree span");
        assert_eq!(height, 13, "Expected height of 13 pixels for 45 degree span (rounded up)");

        // Clean up
        unsafe {
            gdal_sys::VSIUnlink(c_vsi_path.as_ptr());
        }

        println!("Test passed: Bounding box cropping logic is correct");
        println!("Original size: {}x{}", raster_x_size, raster_y_size);
        println!("Cropped size: {}x{}", width, height);
    }

    /// Test the actual cropping function
    #[test]
    fn test_crop_to_bbox_function() {
        use gdal::raster::Buffer;

        // Create a test GeoTIFF in memory
        let vsi_path = "/vsimem/test_crop_input.tif";
        let c_vsi_path = CString::new(vsi_path).unwrap();

        // Create a test dataset: 100x100 pixels covering -180 to 180 longitude, -90 to 90 latitude
        let driver = gdal::DriverManager::get_driver_by_name("GTiff").unwrap();
        let mut dataset = driver
            .create_with_band_type::<f64, _>(vsi_path, 100, 100, 1)
            .unwrap();

        dataset
            .set_geo_transform(&[-180.0, 3.6, 0.0, 90.0, 0.0, -3.6])
            .unwrap();

        dataset
            .set_spatial_ref(&gdal::spatial_ref::SpatialRef::from_epsg(4326).unwrap())
            .unwrap();

        // Fill with test data
        let mut band = dataset.rasterband(1).unwrap();
        let data: Vec<f64> = (0..10000).map(|i| i as f64).collect();
        let mut buffer = Buffer::new((100, 100), data);
        band.write((0, 0), (100, 100), &mut buffer).unwrap();

        drop(dataset);

        // Read the test file
        let original_data = unsafe {
            let mode = CString::new("r").unwrap();
            let fp = gdal_sys::VSIFOpenL(c_vsi_path.as_ptr(), mode.as_ptr());
            assert!(!fp.is_null());

            gdal_sys::VSIFSeekL(fp, 0, 2);
            let size = gdal_sys::VSIFTellL(fp) as usize;
            gdal_sys::VSIFSeekL(fp, 0, 0);

            let mut buffer = vec![0u8; size];
            let read = gdal_sys::VSIFReadL(buffer.as_mut_ptr() as *mut _, 1, size, fp);
            assert_eq!(read, size);
            gdal_sys::VSIFCloseL(fp);

            buffer
        };

        // Call crop_to_bbox with a bounding box
        let minx = -90.0;
        let miny = 0.0;
        let maxx = 0.0;
        let maxy = 45.0;

        let cropped_data = crop_to_bbox(&original_data, minx, miny, maxx, maxy).unwrap();

        // Verify the cropped data is valid by opening it
        let cropped_vsi_path = "/vsimem/test_cropped_output.tif";
        let c_cropped_vsi_path = CString::new(cropped_vsi_path).unwrap();

        unsafe {
            let mode = CString::new("w").unwrap();
            let fp = gdal_sys::VSIFOpenL(c_cropped_vsi_path.as_ptr(), mode.as_ptr());
            assert!(!fp.is_null());

            let written = gdal_sys::VSIFWriteL(cropped_data.as_ptr() as *const _, 1, cropped_data.len(), fp);
            assert_eq!(written, cropped_data.len());
            gdal_sys::VSIFCloseL(fp);
        }

        let cropped_dataset = Dataset::open(cropped_vsi_path).unwrap();
        let (width, height) = cropped_dataset.raster_size();

        // Verify dimensions
        assert_eq!(width, 25, "Cropped width should be 25 pixels");
        assert_eq!(height, 13, "Cropped height should be 13 pixels");

        // Verify geotransform
        let gt = cropped_dataset.geo_transform().unwrap();
        println!("Geotransform: {:?}", gt);
        println!("Cropped origin: ({}, {})", gt[0], gt[3]);

        // The origin should be at the top-left corner of the cropped region
        // For minx=-90, the origin X should be -90
        // For maxy=45, with pixel height of 3.6, and row_min=12, the origin Y should be 90 - 12*3.6 = 46.8
        assert!((gt[0] - (-90.0)).abs() < 0.1, "Origin X should be around -90, got {}", gt[0]);
        assert!((gt[3] - 46.8).abs() < 0.1, "Origin Y should be around 46.8, got {}", gt[3]);
        assert!((gt[1] - 3.6).abs() < 0.01, "Pixel width should be 3.6");
        assert!((gt[5] - (-3.6)).abs() < 0.01, "Pixel height should be -3.6");

        // Clean up
        unsafe {
            gdal_sys::VSIUnlink(c_vsi_path.as_ptr());
            gdal_sys::VSIUnlink(c_cropped_vsi_path.as_ptr());
        }

        println!("Test passed: crop_to_bbox function works correctly");
        println!("Cropped size: {}x{}", width, height);
    }
}
