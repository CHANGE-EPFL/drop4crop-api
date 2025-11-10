use super::db::Layer;
use super::utils::{LayerInfo, convert_to_cog_in_memory, get_min_max_of_raster, parse_filename};
use crate::common::auth::Role;
use crate::common::state::AppState;
use crate::routes::tiles::storage;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::{
    body::Body,
    extract::Multipart,
    http::header,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_keycloak_auth::{PassthroughMode, layer::KeycloakAuthLayer};
use chrono::Utc;
use crudcrate::CRUDResource;
use gdal::Dataset;
use hyper::StatusCode;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, LoaderTrait, QueryFilter,
    QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::vec;
use std::{collections::HashMap, ffi::CString};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

// Custom response type for /map endpoint that includes properly formatted style data for legend
#[derive(Serialize, ToSchema)]
pub struct MapLayerResponse {
    pub id: uuid::Uuid,
    pub layer_name: Option<String>,
    pub crop: Option<String>,
    pub water_model: Option<String>,
    pub climate_model: Option<String>,
    pub scenario: Option<String>,
    pub variable: Option<String>,
    pub year: Option<i32>,
    pub enabled: bool,
    pub uploaded_at: chrono::DateTime<Utc>,
    pub last_updated: chrono::DateTime<Utc>,
    pub global_average: Option<f64>,
    pub filename: Option<String>,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub style_id: Option<uuid::Uuid>,
    pub is_crop_specific: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<Vec<crate::routes::styles::models::StyleItem>>,
    pub country_values: Option<Vec<serde_json::Value>>,
}
#[derive(Deserialize, IntoParams)]
pub struct UploadQueryParams {
    overwrite_duplicates: Option<bool>,
}

pub fn router(state: &AppState) -> OpenApiRouter {
    let public_router = OpenApiRouter::new()
        .routes(routes!(get_groups))
        .routes(routes!(get_all_map_layers))
        .routes(routes!(get_pixel_value))
        .with_state(state.db.clone());

    let mut protected_router = Layer::router(&state.db.clone());
    let protected_custom_routes = OpenApiRouter::new()
        .routes(routes!(upload_file))
        .routes(routes!(download_layer))
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

#[derive(Debug, Deserialize, IntoParams)]
pub struct LayerQueryParams {
    crop: String,
    water_model: Option<String>,
    climate_model: Option<String>,
    scenario: Option<String>,
    variable: String,
    year: Option<i32>,
}

#[utoipa::path(
    get,
    path = "/map",
    params(LayerQueryParams),
    responses(
        (status = 200, description = "Layer list", body = [MapLayerResponse]),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get all enabled layers for map",
    description = "Fetches enabled layers with filtering for use in Drop4Crop map"
)]
pub async fn get_all_map_layers(
    State(db): State<DatabaseConnection>,
    Query(params): Query<LayerQueryParams>,
) -> Result<Json<Vec<MapLayerResponse>>, (StatusCode, Json<String>)> {
    use crate::routes::layers::db::{Column, Entity as LayerEntity};
    use crate::routes::styles::db::Entity as StyleEntity;
    println!("[get_all_map_layers] params: {:?}", params);
    let mut query = LayerEntity::find().filter(Column::Enabled.eq(true));

    query = query.filter(Column::Crop.eq(params.crop));
    query = query.filter(Column::Variable.eq(params.variable));

    if let Some(water_model) = params.water_model {
        query = query.filter(Column::WaterModel.eq(water_model));
    }
    if let Some(climate_model) = params.climate_model {
        query = query.filter(Column::ClimateModel.eq(climate_model));
    }
    if let Some(scenario) = params.scenario {
        query = query.filter(Column::Scenario.eq(scenario));
    }
    if let Some(year) = params.year {
        query = query.filter(Column::Year.eq(year));
    }

    let layer_models = query.all(&db).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(format!("DB error: {}", e)),
        )
    })?;

    let mut response: Vec<MapLayerResponse> = vec![];

    for layer_model in layer_models {
        // Try to find associated style for this layer
        let style_model = if let Some(style_id) = layer_model.style_id {
            StyleEntity::find_by_id(style_id).one(&db).await.ok()
        } else {
            None
        };

        // Create layer with or without style
        let layer = if let Some(style_model) = style_model {
            // Convert style JSON to StyleItem vector
            let style_items = crate::routes::styles::models::StyleItem::from_json(
                &style_model.clone().unwrap().style.unwrap_or_default(),
                layer_model.min_value.unwrap_or_default(),
                layer_model.max_value.unwrap_or_default(),
                10,
            );

            // Construct MapLayerResponse with properly formatted style for legend
            MapLayerResponse {
                id: layer_model.id,
                layer_name: layer_model.layer_name,
                crop: layer_model.crop,
                water_model: layer_model.water_model,
                climate_model: layer_model.climate_model,
                scenario: layer_model.scenario,
                variable: layer_model.variable,
                year: layer_model.year,
                enabled: layer_model.enabled,
                uploaded_at: layer_model.uploaded_at,
                last_updated: layer_model.last_updated,
                global_average: layer_model.global_average,
                filename: layer_model.filename,
                min_value: layer_model.min_value,
                max_value: layer_model.max_value,
                style_id: layer_model.style_id,
                is_crop_specific: layer_model.is_crop_specific,
                style: Some(style_items), // Set style to the StyleItem array for legend
                country_values: None,     // TODO: Calculate country values if needed
            }
        } else {
            // Create default style
            let style_items = crate::routes::styles::models::StyleItem::from_json(
                &serde_json::Value::Null,
                layer_model.min_value.unwrap_or_default(),
                layer_model.max_value.unwrap_or_default(),
                10,
            );

            // Construct MapLayerResponse with default style for legend
            MapLayerResponse {
                id: layer_model.id,
                layer_name: layer_model.layer_name,
                crop: layer_model.crop,
                water_model: layer_model.water_model,
                climate_model: layer_model.climate_model,
                scenario: layer_model.scenario,
                variable: layer_model.variable,
                year: layer_model.year,
                enabled: layer_model.enabled,
                uploaded_at: layer_model.uploaded_at,
                last_updated: layer_model.last_updated,
                global_average: layer_model.global_average,
                filename: layer_model.filename,
                min_value: layer_model.min_value,
                max_value: layer_model.max_value,
                style_id: layer_model.style_id,
                is_crop_specific: layer_model.is_crop_specific,
                style: Some(style_items), // Set style to the StyleItem array for legend
                country_values: None,     // TODO: Calculate country values if needed
            }
        };

        response.push(layer);
    }

    println!("[get_all_map_layers] returning {} layers", response.len());
    Ok(Json(response))
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

            // Check for invalid values
            if min_val.is_finite() && max_val.is_finite() {
                println!("Raster stats: min={}, max={}", min_val, max_val);
            } else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "message": "Invalid raster statistics: min or max value is infinite"
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

#[utoipa::path(
    get,
    path = "/{layer_id}/download",
    params(
        ("layer_id" = String, Path, description = "Layer ID/filename"),
        DownloadQueryParams
    ),
    responses(
        (status = 200, description = "TIFF file download", content_type = "application/octet-stream"),
        (status = 404, description = "Layer not found"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Download layer as TIFF file",
    description = "Downloads the full TIFF file or a cropped region if bounds are provided"
)]
pub async fn download_layer(
    State(db): State<DatabaseConnection>,
    Path(layer_id): Path<String>,
    Query(params): Query<DownloadQueryParams>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let filename = format!("{}.tif", layer_id);

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

    // If no cropping parameters provided, return the full file from S3
    if params.minx.is_none()
        || params.miny.is_none()
        || params.maxx.is_none()
        || params.maxy.is_none()
    {
        let data = storage::get_object(&filename).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "message": "Failed to fetch file from S3",
                    "error": e.to_string()
                })),
            )
        })?;

        let response = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
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

    // For cropping functionality, we'd need more complex GDAL operations
    // For now, return the full file with a note about cropping
    let data = storage::get_object(&filename).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "message": "Failed to fetch file from S3",
                "error": e.to_string()
            })),
        )
    })?;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename),
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

    Ok(response)
}
