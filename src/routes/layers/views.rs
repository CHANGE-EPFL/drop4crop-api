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

