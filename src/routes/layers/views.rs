use super::db::Layer;
use super::models::{
    GetPixelValueParams, LayerInfo, PixelValueResponse, UploadQueryParams,
};
use super::utils::{
    convert_to_cog_in_memory, get_global_average_of_raster, get_min_max_of_raster,
    parse_filename,
};
use crate::common::auth::Role;
use crate::common::state::AppState;
use crate::routes::tiles::storage;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::{
    extract::Multipart,
    response::IntoResponse,
};
use axum_keycloak_auth::{PassthroughMode, layer::KeycloakAuthLayer};
use crudcrate::CRUDResource;
use gdal::Dataset;
use hyper::StatusCode;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QuerySelect, Set,
};
use serde_json::Value as JsonValue;
use std::vec;
use std::{collections::HashMap, ffi::CString};
use tracing::{debug, error, info, warn};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

pub fn router(state: &AppState) -> OpenApiRouter {
    let public_router = OpenApiRouter::new()
        .routes(routes!(get_groups))
        .routes(routes!(get_pixel_value))
        .with_state(state.clone());

    // Get the base crudcrate router
    let mut protected_router = Layer::router(&state.db.clone());

    // Add custom routes
    let protected_custom_routes = OpenApiRouter::new()
        .routes(routes!(upload_file))
        .routes(routes!(recalculate_layer_stats))
        .routes(routes!(recalculate_all_layer_stats))
        .with_state(state.clone());

    protected_router = protected_router
        .merge(protected_custom_routes);

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
        warn!(
            resource = Layer::RESOURCE_NAME_PLURAL,
            deployment = %state.config.deployment,
            "Mutating routes are not protected by authentication. This is only allowed in development environments"
        );
    }

    public_router.merge(protected_router)
}

/// S3-compatible COG data router (for /cog endpoint under /layers)
/// This provides a clean S3-like path structure for COG files
pub fn cog_router(state: &AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(super::cog::views::get_cog_data))
        .with_state(state.clone())
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
    State(app_state): State<AppState>,
) -> Result<Json<HashMap<String, Vec<JsonValue>>>, (StatusCode, Json<String>)> {
    let db = &app_state.db;
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
            .all(db)
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
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    let config = &app_state.config;
    // Build the filename for the TIFF.
    let filename = format!("{}.tif", layer_id);

    // Fetch the object using your existing S3 integration (with caching).
    let object = storage::get_object(&config, &filename).await.map_err(|e| {
        error!(filename, error = %e, "Error fetching object for pixel value");
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
                error!("Failed to open /vsimem file for pixel value query");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
            let written = gdal_sys::VSIFWriteL(object.as_ptr() as *const _, 1, object.len(), fp);
            if written != object.len() {
                gdal_sys::VSIFCloseL(fp);
                error!("Failed to write all data to /vsimem file for pixel value query");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
            gdal_sys::VSIFCloseL(fp);
        }
    }

    // Open the dataset with GDAL.
    let dataset = Dataset::open(&vsi_path).map_err(|e| {
        error!(error = %e, "Error opening dataset for pixel value query");
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
        error!(error = %e, "Error getting geo_transform for pixel value query");
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
        debug!(
            col,
            row, raster_x_size, raster_y_size, "Pixel value query coordinates out of bounds"
        );
        return Err(StatusCode::BAD_REQUEST);
    }

    // Read the pixel value from band 1.
    let band = dataset.rasterband(1).map_err(|e| {
        error!(error = %e, "Error accessing raster band for pixel value query");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let buf_result = band
        .read_as::<f64>((col, row), (1, 1), (1, 1), None)
        .map_err(|e| {
            error!(error = %e, "Error reading pixel value");
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
    State(app_state): State<AppState>,
    Query(params): Query<UploadQueryParams>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    debug!("Starting upload request");
    let db = &app_state.db;
    let config = &app_state.config;
    let overwrite_duplicates = params
        .overwrite_duplicates
        .unwrap_or(config.overwrite_duplicate_layers);

    debug!("About to process multipart data");
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        error!(error = %e, "Error reading multipart field");
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Error parsing `multipart/form-data` request",
                "message": "Failed to read file data"
            })),
        )
    })? {
        debug!("Got a field from multipart");
        let name = field.name().unwrap_or("file");

        if name == "file" {
            debug!("Processing file field");
            let filename = field
                .file_name()
                .ok_or_else(|| {
                    error!("No filename provided");
                    (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "message": "No filename provided"
                        })),
                    )
                })?
                .to_lowercase();

            debug!(filename, "Processing upload file");
            debug!("About to read file bytes");

            let data = match field.bytes().await {
                Ok(data) => {
                    debug!(size = data.len(), "Successfully read file bytes");
                    data
                }
                Err(e) => {
                    error!(error = %e, "Error reading file bytes");
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "Error reading file bytes",
                            "message": format!("Failed to read file data: {}", e)
                        })),
                    ));
                }
            };

            debug!(size = data.len(), "Successfully read bytes");

            // Parse filename to extract layer information
            debug!("Parsing filename");
            let layer_info = parse_filename(&config, &filename).map_err(|e| {
                error!(filename, error = %e, "Error parsing filename");
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "message": "Invalid filename format",
                        "error": e.to_string()
                    })),
                )
            })?;
            debug!("Successfully parsed filename");

            // Check for duplicate layer
            debug!("Checking for duplicate layers");
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

            let existing_layer = duplicate_query.one(db).await.map_err(|e| {
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
                        let s3_key = storage::get_s3_key(&config, filename);
                        storage::delete_object(&config, &s3_key)
                            .await
                            .map_err(|e| {
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
                        .exec(db)
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

                    info!(
                        layer = existing.filename.unwrap_or_else(|| "unknown".to_string()),
                        "Deleted existing layer"
                    );
                    debug!(filename, "Continuing with upload of duplicate file");
                } else {
                    warn!(filename, "Rejecting duplicate file");
                    return Err((
                        StatusCode::CONFLICT,
                        Json(serde_json::json!({
                            "message": format!("Layer already exists for {}. Delete layer first to re-upload, or set overwrite_duplicates=true", filename)
                        })),
                    ));
                }
            }

            // Convert to COG
            debug!("Converting to COG format");
            let cog_bytes = convert_to_cog_in_memory(&data).map_err(|e| {
                error!(error = %e, "Error converting to COG");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "message": "Failed to convert to COG",
                        "error": e.to_string()
                    })),
                )
            })?;
            info!(size = cog_bytes.len(), "Successfully converted to COG");

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
                debug!(min_val, max_val, global_avg, "Raster statistics calculated");
            } else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "message": "Invalid raster statistics: min, max, or global_average value is infinite"
                    })),
                ));
            }

            // Upload to S3
            let s3_key = storage::get_s3_key(&config, &filename);
            storage::upload_object(&config, &s3_key, &cog_bytes)
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
            let cog_file_size = cog_bytes.len() as i64;
            let stats_status_json = serde_json::json!({
                "status": "success",
                "last_run": chrono::Utc::now(),
                "error": null,
                "details": format!("Initial upload - min: {}, max: {}, avg: {}, file_size: {} bytes", min_val, max_val, global_avg, cog_file_size)
            });
            debug!(layer_name, "Creating layer record");
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
                        file_size: Set(Some(cog_file_size)),
                        stats_status: Set(Some(stats_status_json.clone())),
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
                        file_size: Set(Some(cog_file_size)),
                        stats_status: Set(Some(stats_status_json.clone())),
                        enabled: Set(true),
                        is_crop_specific: Set(true),
                        ..Default::default()
                    }
                }
            };

            debug!(filename, "Attempting to save layer to database");
            let saved_layer = match layer_record.insert(db).await {
                Ok(layer) => {
                    info!(filename, "Successfully saved layer to database");
                    layer
                }
                Err(e) => {
                    error!(filename, error = %e, "Failed to save layer to database");
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "message": "Failed to save layer to database",
                            "error": e.to_string()
                        })),
                    ));
                }
            };

            info!(filename, "Successfully uploaded layer");

            // Return the saved layer as Layer model
            debug!(filename, "Creating response object for layer");
            let layer_response = match std::panic::catch_unwind(|| Layer::from(saved_layer)) {
                Ok(response) => {
                    debug!(filename, "Successfully created response object for layer");
                    response
                }
                Err(panic_info) => {
                    error!(
                        filename,
                        "PANIC during Layer::from() conversion for layer: {:?}", panic_info
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
            debug!(filename, "Response object created, sending response");
            return Ok((StatusCode::OK, Json(layer_response)));
        }
    }

    error!("No file found in multipart data");
    Err((
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "message": "No file found in upload"
        })),
    ))
}

/// Response for recalculated statistics
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct RecalculatedStats {
    pub id: Uuid,
    pub layer_name: Option<String>,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub global_average: Option<f64>,
    pub previous_min_value: Option<f64>,
    pub previous_max_value: Option<f64>,
    pub previous_global_average: Option<f64>,
}

/// Response for bulk recalculation
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct BulkRecalculateResponse {
    pub success_count: usize,
    pub error_count: usize,
    pub results: Vec<RecalculatedStats>,
    pub errors: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/{layer_id}/recalculate-stats",
    params(
        ("layer_id" = Uuid, Path, description = "Layer ID")
    ),
    responses(
        (status = 200, description = "Statistics recalculated", body = RecalculatedStats),
        (status = 404, description = "Layer not found"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Recalculate layer statistics",
    description = "Fetches the layer from S3 and recalculates min_value, max_value, and global_average using GDAL"
)]
pub async fn recalculate_layer_stats(
    Path(layer_id): Path<Uuid>,
    State(app_state): State<AppState>,
) -> Result<Json<RecalculatedStats>, (StatusCode, Json<serde_json::Value>)> {
    let db = &app_state.db;
    let config = &app_state.config;

    // Find the layer
    let layer = super::db::Entity::find_by_id(layer_id)
        .one(db)
        .await
        .map_err(|e| {
            error!(error = %e, "Database error finding layer");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "message": "Database error", "error": e.to_string() })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "message": "Layer not found" })),
            )
        })?;

    let filename = layer.filename.clone().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "message": "Layer has no filename" })),
        )
    })?;

    // Fetch the file from S3
    let object = storage::get_object(&config, &filename).await.map_err(|e| {
        error!(filename, error = %e, "Error fetching object from S3");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": "Failed to fetch layer from S3", "error": e.to_string() })),
        )
    })?;

    // Validate file size - a valid GeoTIFF should be at least a few KB
    let file_size = object.len() as i64;
    if file_size < 1024 {
        error!(filename, file_size, "File too small to be a valid GeoTIFF");

        // Update stats_status with error
        use super::db::ActiveModel as LayerActiveModel;
        let mut active_layer: LayerActiveModel = layer.clone().into();
        active_layer.stats_status = Set(Some(serde_json::json!({
            "status": "error",
            "last_run": chrono::Utc::now(),
            "error": format!("File too small: {} bytes", file_size),
            "details": format!("filename: {}", filename)
        })));
        active_layer.file_size = Set(Some(file_size));
        let _ = active_layer.update(db).await;

        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "message": "File appears to be corrupted or invalid",
                "error": format!("File size is only {} bytes, expected a valid GeoTIFF", file_size),
                "filename": filename
            })),
        ));
    }

    debug!(filename = %filename, file_size, "Fetched file from S3, calculating statistics");

    // Helper to update stats_status on error
    async fn update_error_status(
        db: &sea_orm::DatabaseConnection,
        layer: super::db::Model,
        error_msg: &str,
        filename: &str,
        file_size: i64,
    ) {
        use super::db::ActiveModel as LayerActiveModel;
        let mut active_layer: LayerActiveModel = layer.into();
        active_layer.stats_status = Set(Some(serde_json::json!({
            "status": "error",
            "last_run": chrono::Utc::now(),
            "error": error_msg,
            "details": format!("filename: {}, file_size: {} bytes", filename, file_size)
        })));
        active_layer.file_size = Set(Some(file_size));
        let _ = active_layer.update(db).await;
    }

    // Calculate statistics
    let (min_val, max_val) = match get_min_max_of_raster(&object) {
        Ok(v) => v,
        Err(e) => {
            let error_msg = e.to_string();
            error!(filename = %filename, file_size, error = %e, "Error calculating min/max");
            update_error_status(db, layer.clone(), &error_msg, &filename, file_size).await;
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "message": "Failed to calculate min/max",
                    "error": error_msg,
                    "filename": filename,
                    "file_size": file_size
                })),
            ));
        }
    };

    let global_avg = match get_global_average_of_raster(&object) {
        Ok(v) => v,
        Err(e) => {
            let error_msg = e.to_string();
            error!(filename = %filename, file_size, error = %e, "Error calculating global average");
            update_error_status(db, layer.clone(), &error_msg, &filename, file_size).await;
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "message": "Failed to calculate global average",
                    "error": error_msg,
                    "filename": filename,
                    "file_size": file_size
                })),
            ));
        }
    };

    // Validate values
    if !min_val.is_finite() || !max_val.is_finite() || !global_avg.is_finite() {
        let error_msg = "Calculated statistics contain invalid values (NaN/Inf)";
        update_error_status(db, layer.clone(), error_msg, &filename, file_size).await;
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "message": error_msg })),
        ));
    }

    // Store previous values for response
    let previous_min = layer.min_value;
    let previous_max = layer.max_value;
    let previous_avg = layer.global_average;

    // Update the layer with stats and success status
    use super::db::ActiveModel as LayerActiveModel;
    let mut active_layer: LayerActiveModel = layer.clone().into();
    active_layer.min_value = Set(Some(min_val));
    active_layer.max_value = Set(Some(max_val));
    active_layer.global_average = Set(Some(global_avg));
    active_layer.file_size = Set(Some(file_size));
    active_layer.stats_status = Set(Some(serde_json::json!({
        "status": "success",
        "last_run": chrono::Utc::now(),
        "error": null,
        "details": format!("min: {}, max: {}, avg: {}, file_size: {} bytes", min_val, max_val, global_avg, file_size)
    })));

    active_layer.update(db).await.map_err(|e| {
        error!(error = %e, "Error updating layer");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "message": "Failed to update layer", "error": e.to_string() })),
        )
    })?;

    info!(
        layer_id = %layer_id,
        layer_name = layer.layer_name,
        min_val, max_val, global_avg,
        "Recalculated layer statistics"
    );

    Ok(Json(RecalculatedStats {
        id: layer_id,
        layer_name: layer.layer_name,
        min_value: Some(min_val),
        max_value: Some(max_val),
        global_average: Some(global_avg),
        previous_min_value: previous_min,
        previous_max_value: previous_max,
        previous_global_average: previous_avg,
    }))
}

/// Query parameters for bulk recalculation
#[derive(serde::Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct BulkRecalculateParams {
    /// Filter by crop
    pub crop: Option<String>,
    /// Filter by variable
    pub variable: Option<String>,
    /// Filter by water model
    pub water_model: Option<String>,
    /// Filter by climate model
    pub climate_model: Option<String>,
    /// Filter by scenario
    pub scenario: Option<String>,
    /// Filter by year
    pub year: Option<i32>,
    /// Only recalculate layers with null statistics
    pub only_null_stats: Option<bool>,
    /// Limit number of layers to process (default 100, max 1000)
    pub limit: Option<u64>,
}

#[utoipa::path(
    post,
    path = "/recalculate-stats",
    params(BulkRecalculateParams),
    responses(
        (status = 200, description = "Bulk recalculation completed", body = BulkRecalculateResponse),
        (status = 500, description = "Internal server error")
    ),
    summary = "Bulk recalculate layer statistics",
    description = "Recalculates statistics for multiple layers. Use filters to target specific layers."
)]
pub async fn recalculate_all_layer_stats(
    Query(params): Query<BulkRecalculateParams>,
    State(app_state): State<AppState>,
) -> Result<Json<BulkRecalculateResponse>, (StatusCode, Json<serde_json::Value>)> {
    let db = &app_state.db;
    let config = &app_state.config;

    // Build query with filters
    let mut query = super::db::Entity::find();

    if let Some(crop) = &params.crop {
        query = query.filter(super::db::Column::Crop.eq(crop));
    }
    if let Some(variable) = &params.variable {
        query = query.filter(super::db::Column::Variable.eq(variable));
    }
    if let Some(water_model) = &params.water_model {
        query = query.filter(super::db::Column::WaterModel.eq(water_model));
    }
    if let Some(climate_model) = &params.climate_model {
        query = query.filter(super::db::Column::ClimateModel.eq(climate_model));
    }
    if let Some(scenario) = &params.scenario {
        query = query.filter(super::db::Column::Scenario.eq(scenario));
    }
    if let Some(year) = params.year {
        query = query.filter(super::db::Column::Year.eq(year));
    }
    if params.only_null_stats.unwrap_or(false) {
        query = query.filter(
            super::db::Column::MinValue.is_null()
                .or(super::db::Column::MaxValue.is_null())
                .or(super::db::Column::GlobalAverage.is_null())
        );
    }

    // Apply limit (default 100, max 1000)
    let limit = params.limit.unwrap_or(100).min(1000);

    let layers = query
        .limit(limit)
        .all(db)
        .await
        .map_err(|e| {
            error!(error = %e, "Database error fetching layers");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "message": "Database error", "error": e.to_string() })),
            )
        })?;

    info!(count = layers.len(), "Starting bulk recalculation");

    let mut results = Vec::new();
    let mut errors = Vec::new();
    let mut success_count = 0;
    let mut error_count = 0;

    for layer in layers {
        let layer_id = layer.id;
        let layer_name = layer.layer_name.clone();

        let filename = match &layer.filename {
            Some(f) => f.clone(),
            None => {
                errors.push(format!("Layer {} has no filename", layer_id));
                error_count += 1;
                continue;
            }
        };

        // Fetch from S3
        let object = match storage::get_object(&config, &filename).await {
            Ok(o) => o,
            Err(e) => {
                errors.push(format!("Layer {}: Failed to fetch from S3: {}", layer_id, e));
                error_count += 1;
                continue;
            }
        };

        // Calculate statistics
        let (min_val, max_val) = match get_min_max_of_raster(&object) {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("Layer {}: Failed to calculate min/max: {}", layer_id, e));
                error_count += 1;
                continue;
            }
        };

        let global_avg = match get_global_average_of_raster(&object) {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("Layer {}: Failed to calculate average: {}", layer_id, e));
                error_count += 1;
                continue;
            }
        };

        // Validate
        if !min_val.is_finite() || !max_val.is_finite() || !global_avg.is_finite() {
            errors.push(format!("Layer {}: Invalid statistics (NaN/Inf)", layer_id));
            error_count += 1;
            continue;
        }

        // Store previous values
        let previous_min = layer.min_value;
        let previous_max = layer.max_value;
        let previous_avg = layer.global_average;

        // Update
        use super::db::ActiveModel as LayerActiveModel;
        let mut active_layer: LayerActiveModel = layer.into();
        active_layer.min_value = Set(Some(min_val));
        active_layer.max_value = Set(Some(max_val));
        active_layer.global_average = Set(Some(global_avg));

        if let Err(e) = active_layer.update(db).await {
            errors.push(format!("Layer {}: Failed to update: {}", layer_id, e));
            error_count += 1;
            continue;
        }

        results.push(RecalculatedStats {
            id: layer_id,
            layer_name,
            min_value: Some(min_val),
            max_value: Some(max_val),
            global_average: Some(global_avg),
            previous_min_value: previous_min,
            previous_max_value: previous_max,
            previous_global_average: previous_avg,
        });
        success_count += 1;
    }

    info!(success_count, error_count, "Bulk recalculation completed");

    Ok(Json(BulkRecalculateResponse {
        success_count,
        error_count,
        results,
        errors,
    }))
}

