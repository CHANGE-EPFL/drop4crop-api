use super::db::Layer;
use super::models::{
    GetPixelValueParams, LayerInfo, PixelValueResponse, UploadError, UploadQueryParams,
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
    ActiveModelTrait, ColumnTrait, EntityTrait, JoinType, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait, Set,
};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::vec;
use std::{collections::HashMap, ffi::CString};
use tracing::{debug, error, info, warn};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

#[derive(Deserialize, utoipa::IntoParams)]
pub struct GroupsQueryParams {
    /// Optional project slug to filter layers by project
    pub project: Option<String>,
}

/// Renders an `UploadError` into the `(StatusCode, Json<Value>)` shape the handler returns.
/// Serialization into `Value` can't fail for this struct but we degrade gracefully just in case.
fn upload_err_json(
    status: StatusCode,
    err: UploadError,
) -> (StatusCode, Json<serde_json::Value>) {
    let body = serde_json::to_value(&err).unwrap_or_else(|_| {
        serde_json::json!({
            "code": "internal",
            "message": err.message,
        })
    });
    (status, Json(body))
}

/// Converts a SeaORM database error into the handler's error shape.
/// Defined at file scope (not as a closure) so it's a plain function pointer
/// that can be passed to `map_err` many times without being moved.
fn db_upload_err(e: sea_orm::DbErr) -> (StatusCode, Json<serde_json::Value>) {
    upload_err_json(
        StatusCode::INTERNAL_SERVER_ERROR,
        UploadError::new("internal", "Database error").with_error(e.to_string()),
    )
}

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
        .routes(routes!(recalculate_stats_by_ids))
        .routes(routes!(get_recalculate_job_status))
        .routes(routes!(cancel_recalculate_job))
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
    params(GroupsQueryParams),
    responses(
        (status = 200, description = "Filtered data found", body = HashMap<String, Vec<JsonValue>>),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get all unique groups",
    description = "This endpoint allows the menu to be populated with available keys. Optionally filter by project slug."
)]
pub async fn get_groups(
    State(app_state): State<AppState>,
    Query(params): Query<GroupsQueryParams>,
) -> Result<Json<HashMap<String, Vec<JsonValue>>>, (StatusCode, Json<String>)> {
    let db = &app_state.db;
    let mut groups: HashMap<String, Vec<JsonValue>> = HashMap::new();

    // Resolve optional project slug to UUID
    let project_uuid = if let Some(ref project_slug) = params.project {
        let project = crate::routes::projects::db::Entity::find()
            .filter(crate::routes::projects::db::Column::Slug.eq(project_slug.as_str()))
            .one(db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?;
        project.map(|p| p.id)
    } else {
        None
    };

    // When a project is specified, the project configuration (junction tables)
    // is the source of truth — not what layers happen to exist. This ensures a
    // newly configured project with zero layers still shows its axes.
    if let Some(pid) = project_uuid {
        let db_err = |e: sea_orm::DbErr| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string()));

        // Crops from project_crop junction
        let crop_junctions = crate::routes::projects::project_crop::Entity::find()
            .filter(crate::routes::projects::project_crop::Column::ProjectId.eq(pid))
            .order_by_asc(crate::routes::projects::project_crop::Column::SortOrder)
            .all(db).await.map_err(db_err)?;
        let mut crops = Vec::new();
        for junc in &crop_junctions {
            if let Some(c) = crate::routes::crops::db::Entity::find_by_id(junc.crop_id)
                .one(db).await.map_err(db_err)? {
                crops.push(serde_json::json!({
                    "id": c.id, "slug": c.slug, "name": c.name,
                    "sort_order": junc.sort_order,
                }));
            }
        }
        if !crops.is_empty() { groups.insert("crop".to_string(), crops); }

        // Water models from project_water_model junction
        let wm_junctions = crate::routes::projects::project_water_model::Entity::find()
            .filter(crate::routes::projects::project_water_model::Column::ProjectId.eq(pid))
            .order_by_asc(crate::routes::projects::project_water_model::Column::SortOrder)
            .all(db).await.map_err(db_err)?;
        let mut water_models = Vec::new();
        for junc in &wm_junctions {
            if let Some(w) = crate::routes::water_models::db::Entity::find_by_id(junc.water_model_id)
                .one(db).await.map_err(db_err)? {
                water_models.push(serde_json::json!({
                    "id": w.id, "slug": w.slug, "name": w.name,
                    "sort_order": junc.sort_order,
                }));
            }
        }
        if !water_models.is_empty() { groups.insert("water_model".to_string(), water_models); }

        // Climate models from project_climate_model junction
        let cm_junctions = crate::routes::projects::project_climate_model::Entity::find()
            .filter(crate::routes::projects::project_climate_model::Column::ProjectId.eq(pid))
            .order_by_asc(crate::routes::projects::project_climate_model::Column::SortOrder)
            .all(db).await.map_err(db_err)?;
        let mut climate_models = Vec::new();
        for junc in &cm_junctions {
            if let Some(c) = crate::routes::climate_models::db::Entity::find_by_id(junc.climate_model_id)
                .one(db).await.map_err(db_err)? {
                climate_models.push(serde_json::json!({
                    "id": c.id, "slug": c.slug, "name": c.name,
                    "sort_order": junc.sort_order,
                }));
            }
        }
        if !climate_models.is_empty() { groups.insert("climate_model".to_string(), climate_models); }

        // Scenarios from project_scenario junction
        let sc_junctions = crate::routes::projects::project_scenario::Entity::find()
            .filter(crate::routes::projects::project_scenario::Column::ProjectId.eq(pid))
            .order_by_asc(crate::routes::projects::project_scenario::Column::SortOrder)
            .all(db).await.map_err(db_err)?;
        let mut scenarios = Vec::new();
        for junc in &sc_junctions {
            if let Some(s) = crate::routes::scenarios::db::Entity::find_by_id(junc.scenario_id)
                .one(db).await.map_err(db_err)? {
                scenarios.push(serde_json::json!({
                    "id": s.id, "slug": s.slug, "name": s.name,
                    "sort_order": junc.sort_order,
                }));
            }
        }
        if !scenarios.is_empty() { groups.insert("scenario".to_string(), scenarios); }

        // Variables from project_variable junction (include is_crop_specific and has_time)
        let var_junctions = crate::routes::projects::project_variable::Entity::find()
            .filter(crate::routes::projects::project_variable::Column::ProjectId.eq(pid))
            .order_by_asc(crate::routes::projects::project_variable::Column::SortOrder)
            .all(db).await.map_err(db_err)?;
        let mut variables = Vec::new();
        for junc in &var_junctions {
            if let Some(v) = crate::routes::variables::db::Entity::find_by_id(junc.variable_id)
                .one(db).await.map_err(db_err)? {
                variables.push(serde_json::json!({
                    "id": v.id, "slug": v.slug, "name": v.name,
                    "abbreviation": v.abbreviation, "subscript": v.subscript,
                    "unit": v.unit, "is_crop_specific": v.is_crop_specific,
                    "has_time": v.has_time, "group_name": v.group_name,
                    "sort_order": junc.sort_order,
                }));
            }
        }
        if !variables.is_empty() { groups.insert("variable".to_string(), variables); }

        // Years: still from layer data (no junction table for years)
        let year_rows = super::db::Entity::find()
            .filter(super::db::Column::Enabled.eq(true))
            .filter(super::db::Column::ProjectId.eq(pid))
            .select_only()
            .column(super::db::Column::Year)
            .distinct()
            .into_json()
            .all(db)
            .await
            .map_err(db_err)?;
        let years: Vec<JsonValue> = year_rows
            .into_iter()
            .filter_map(|mut json| json.as_object_mut()?.remove("year"))
            .filter(|v| !v.is_null())
            .collect();
        if !years.is_empty() { groups.insert("year".to_string(), years); }

        return Ok(Json(groups));
    }

    // Global scope (no project filter): derive axes from actual layer data
    let crops = {
        let q = crate::routes::crops::db::Entity::find()
            .join(JoinType::InnerJoin, super::db::Relation::Crop.def().rev())
            .filter(super::db::Column::Enabled.eq(true))
            .distinct()
            .order_by_asc(crate::routes::crops::db::Column::SortOrder);
        q.into_json().all(db).await
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?;
    if !crops.is_empty() {
        groups.insert("crop".to_string(), crops);
    }

    let water_models = {
        let q = crate::routes::water_models::db::Entity::find()
            .join(JoinType::InnerJoin, super::db::Relation::WaterModel.def().rev())
            .filter(super::db::Column::Enabled.eq(true))
            .distinct()
            .order_by_asc(crate::routes::water_models::db::Column::SortOrder);
        q.into_json().all(db).await
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?;
    if !water_models.is_empty() {
        groups.insert("water_model".to_string(), water_models);
    }

    let climate_models = {
        let q = crate::routes::climate_models::db::Entity::find()
            .join(JoinType::InnerJoin, super::db::Relation::ClimateModel.def().rev())
            .filter(super::db::Column::Enabled.eq(true))
            .distinct()
            .order_by_asc(crate::routes::climate_models::db::Column::SortOrder);
        q.into_json().all(db).await
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?;
    if !climate_models.is_empty() {
        groups.insert("climate_model".to_string(), climate_models);
    }

    let scenarios = {
        let q = crate::routes::scenarios::db::Entity::find()
            .join(JoinType::InnerJoin, super::db::Relation::Scenario.def().rev())
            .filter(super::db::Column::Enabled.eq(true))
            .distinct()
            .order_by_asc(crate::routes::scenarios::db::Column::SortOrder);
        q.into_json().all(db).await
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?;
    if !scenarios.is_empty() {
        groups.insert("scenario".to_string(), scenarios);
    }

    let variables = {
        let q = crate::routes::variables::db::Entity::find()
            .join(JoinType::InnerJoin, super::db::Relation::Variable.def().rev())
            .filter(super::db::Column::Enabled.eq(true))
            .distinct()
            .order_by_asc(crate::routes::variables::db::Column::SortOrder);
        q.into_json().all(db).await
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?;
    if !variables.is_empty() {
        groups.insert("variable".to_string(), variables);
    }

    let year_rows = super::db::Entity::find()
        .filter(super::db::Column::Enabled.eq(true))
        .select_only()
        .column(super::db::Column::Year)
        .distinct()
        .into_json()
        .all(db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?;
    let years: Vec<JsonValue> = year_rows
        .into_iter()
        .filter_map(|mut json| json.as_object_mut()?.remove("year"))
        .filter(|v| !v.is_null())
        .collect();
    if !years.is_empty() {
        groups.insert("year".to_string(), years);
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
    let db = &app_state.db;
    // Build the filename for the TIFF.
    let filename = format!("{}.tif", layer_id);

    // Resolve the layer row so we know which project owns the file — the S3
    // object lives under that project's subpath (see `storage::s3_key_stem`).
    let project_id = super::db::Entity::find()
        .filter(super::db::Column::LayerName.eq(&layer_id))
        .one(db)
        .await
        .map_err(|e| {
            error!(filename, error = %e, "DB lookup failed for pixel value");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .and_then(|l| l.project_id);

    // Fetch the object using your existing S3 integration (with caching).
    let object = storage::get_object(&config, project_id, &filename).await.map_err(|e| {
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
                upload_err_json(
                    StatusCode::BAD_REQUEST,
                    UploadError::new("parse_error", "Invalid filename format")
                        .with_error(e.to_string()),
                )
            })?;
            debug!("Successfully parsed filename");

            // Resolve slugs to UUIDs from reference tables.
            // In the 6-part climate form every middle slot (including variable) can be a
            // `null`/`nan` sentinel; in the 2-part crop form the variable is required.
            debug!("Resolving slugs to UUIDs");
            let crop_slug = match &layer_info {
                LayerInfo::Climate(info) => info.crop.clone(),
                LayerInfo::Crop(info) => info.crop.clone(),
            };
            let variable_slug: Option<String> = match &layer_info {
                LayerInfo::Climate(info) => info.variable.clone(),
                LayerInfo::Crop(info) => Some(info.variable.clone()),
            };

            // Resolve each slug to a UUID via the reference tables. Missing rows return
            // `slug_unknown`. When `project_id` is present, also verify the resolved UUID
            // is attached to that project's junction table — a globally-known slug that
            // isn't attached returns `slug_not_in_project`. The frontend routes both codes
            // into the resolution panel.
            let crop_uuid = crate::routes::crops::db::Entity::find()
                .filter(crate::routes::crops::db::Column::Slug.eq(&crop_slug))
                .one(db)
                .await
                .map_err(db_upload_err)?
                .ok_or_else(|| {
                    upload_err_json(
                        StatusCode::BAD_REQUEST,
                        UploadError::with_slug(
                            "slug_unknown",
                            "crop",
                            &crop_slug,
                            format!("Unknown crop slug: {}", crop_slug),
                        ),
                    )
                })?
                .id;

            if let Some(pid) = params.project_id {
                let attached = crate::routes::projects::project_crop::Entity::find()
                    .filter(crate::routes::projects::project_crop::Column::ProjectId.eq(pid))
                    .filter(crate::routes::projects::project_crop::Column::CropId.eq(crop_uuid))
                    .one(db)
                    .await
                    .map_err(db_upload_err)?;
                if attached.is_none() {
                    return Err(upload_err_json(
                        StatusCode::BAD_REQUEST,
                        UploadError::with_slug(
                            "slug_not_in_project",
                            "crop",
                            &crop_slug,
                            format!("Crop '{}' is not attached to this project", crop_slug),
                        ),
                    ));
                }
            }

            let (variable_uuid, _is_crop_specific): (Option<Uuid>, bool) =
                if let Some(vslug) = &variable_slug {
                    let variable_record = crate::routes::variables::db::Entity::find()
                        .filter(crate::routes::variables::db::Column::Slug.eq(vslug))
                        .one(db)
                        .await
                        .map_err(db_upload_err)?
                        .ok_or_else(|| {
                            upload_err_json(
                                StatusCode::BAD_REQUEST,
                                UploadError::with_slug(
                                    "slug_unknown",
                                    "variable",
                                    vslug,
                                    format!("Unknown variable slug: {}", vslug),
                                ),
                            )
                        })?;
                    if let Some(pid) = params.project_id {
                        let attached = crate::routes::projects::project_variable::Entity::find()
                            .filter(
                                crate::routes::projects::project_variable::Column::ProjectId
                                    .eq(pid),
                            )
                            .filter(
                                crate::routes::projects::project_variable::Column::VariableId
                                    .eq(variable_record.id),
                            )
                            .one(db)
                            .await
                            .map_err(db_upload_err)?;
                        if attached.is_none() {
                            return Err(upload_err_json(
                                StatusCode::BAD_REQUEST,
                                UploadError::with_slug(
                                    "slug_not_in_project",
                                    "variable",
                                    vslug,
                                    format!(
                                        "Variable '{}' is not attached to this project",
                                        vslug
                                    ),
                                ),
                            ));
                        }
                    }
                    (Some(variable_record.id), variable_record.is_crop_specific)
                } else {
                    (None, false)
                };

            // Slots left as `null` in the filename (case-insensitive) become `None` here and
            // the corresponding FK column is stored as NULL. Real slugs are resolved to UUIDs
            // and, when a project is set, checked against the project's junction tables.
            let (water_model_uuid, climate_model_uuid, scenario_uuid) = if let LayerInfo::Climate(info) = &layer_info {
                let wm = if let Some(slug) = &info.water_model {
                    let id = crate::routes::water_models::db::Entity::find()
                        .filter(crate::routes::water_models::db::Column::Slug.eq(slug))
                        .one(db)
                        .await
                        .map_err(db_upload_err)?
                        .ok_or_else(|| {
                            upload_err_json(
                                StatusCode::BAD_REQUEST,
                                UploadError::with_slug(
                                    "slug_unknown",
                                    "water_model",
                                    slug,
                                    format!("Unknown water_model slug: {}", slug),
                                ),
                            )
                        })?
                        .id;
                    if let Some(pid) = params.project_id {
                        let attached = crate::routes::projects::project_water_model::Entity::find()
                            .filter(
                                crate::routes::projects::project_water_model::Column::ProjectId
                                    .eq(pid),
                            )
                            .filter(
                                crate::routes::projects::project_water_model::Column::WaterModelId
                                    .eq(id),
                            )
                            .one(db)
                            .await
                            .map_err(db_upload_err)?;
                        if attached.is_none() {
                            return Err(upload_err_json(
                                StatusCode::BAD_REQUEST,
                                UploadError::with_slug(
                                    "slug_not_in_project",
                                    "water_model",
                                    slug,
                                    format!(
                                        "Water model '{}' is not attached to this project",
                                        slug
                                    ),
                                ),
                            ));
                        }
                    }
                    Some(id)
                } else {
                    None
                };
                let cm = if let Some(slug) = &info.climate_model {
                    let id = crate::routes::climate_models::db::Entity::find()
                        .filter(crate::routes::climate_models::db::Column::Slug.eq(slug))
                        .one(db)
                        .await
                        .map_err(db_upload_err)?
                        .ok_or_else(|| {
                            upload_err_json(
                                StatusCode::BAD_REQUEST,
                                UploadError::with_slug(
                                    "slug_unknown",
                                    "climate_model",
                                    slug,
                                    format!("Unknown climate_model slug: {}", slug),
                                ),
                            )
                        })?
                        .id;
                    if let Some(pid) = params.project_id {
                        let attached = crate::routes::projects::project_climate_model::Entity::find()
                            .filter(
                                crate::routes::projects::project_climate_model::Column::ProjectId
                                    .eq(pid),
                            )
                            .filter(
                                crate::routes::projects::project_climate_model::Column::ClimateModelId
                                    .eq(id),
                            )
                            .one(db)
                            .await
                            .map_err(db_upload_err)?;
                        if attached.is_none() {
                            return Err(upload_err_json(
                                StatusCode::BAD_REQUEST,
                                UploadError::with_slug(
                                    "slug_not_in_project",
                                    "climate_model",
                                    slug,
                                    format!(
                                        "Climate model '{}' is not attached to this project",
                                        slug
                                    ),
                                ),
                            ));
                        }
                    }
                    Some(id)
                } else {
                    None
                };
                let sc = if let Some(slug) = &info.scenario {
                    let id = crate::routes::scenarios::db::Entity::find()
                        .filter(crate::routes::scenarios::db::Column::Slug.eq(slug))
                        .one(db)
                        .await
                        .map_err(db_upload_err)?
                        .ok_or_else(|| {
                            upload_err_json(
                                StatusCode::BAD_REQUEST,
                                UploadError::with_slug(
                                    "slug_unknown",
                                    "scenario",
                                    slug,
                                    format!("Unknown scenario slug: {}", slug),
                                ),
                            )
                        })?
                        .id;
                    if let Some(pid) = params.project_id {
                        let attached = crate::routes::projects::project_scenario::Entity::find()
                            .filter(
                                crate::routes::projects::project_scenario::Column::ProjectId
                                    .eq(pid),
                            )
                            .filter(
                                crate::routes::projects::project_scenario::Column::ScenarioId
                                    .eq(id),
                            )
                            .one(db)
                            .await
                            .map_err(db_upload_err)?;
                        if attached.is_none() {
                            return Err(upload_err_json(
                                StatusCode::BAD_REQUEST,
                                UploadError::with_slug(
                                    "slug_not_in_project",
                                    "scenario",
                                    slug,
                                    format!(
                                        "Scenario '{}' is not attached to this project",
                                        slug
                                    ),
                                ),
                            ));
                        }
                    }
                    Some(id)
                } else {
                    None
                };
                (wm, cm, sc)
            } else {
                (None, None, None)
            };

            // Check for duplicate layer — scoped to the current project (project_id filter),
            // and matching NULL columns with .is_null() rather than .eq(None) since the latter
            // would serialize as `= NULL` which is never true in SQL.
            debug!("Checking for duplicate layers");
            let duplicate_query = match &layer_info {
                LayerInfo::Climate(info) => {
                    use crate::routes::layers::db::{Column, Entity as LayerEntity};
                    let mut q = LayerEntity::find()
                        .filter(Column::CropId.eq(crop_uuid));
                    q = match info.year {
                        Some(y) => q.filter(Column::Year.eq(y)),
                        None => q.filter(Column::Year.is_null()),
                    };
                    q = match variable_uuid {
                        Some(id) => q.filter(Column::VariableId.eq(id)),
                        None => q.filter(Column::VariableId.is_null()),
                    };
                    q = match water_model_uuid {
                        Some(id) => q.filter(Column::WaterModelId.eq(id)),
                        None => q.filter(Column::WaterModelId.is_null()),
                    };
                    q = match climate_model_uuid {
                        Some(id) => q.filter(Column::ClimateModelId.eq(id)),
                        None => q.filter(Column::ClimateModelId.is_null()),
                    };
                    q = match scenario_uuid {
                        Some(id) => q.filter(Column::ScenarioId.eq(id)),
                        None => q.filter(Column::ScenarioId.is_null()),
                    };
                    q = match params.project_id {
                        Some(pid) => q.filter(Column::ProjectId.eq(pid)),
                        None => q.filter(Column::ProjectId.is_null()),
                    };
                    q
                }
                LayerInfo::Crop(_info) => {
                    // 2-part crop form always has a real variable, so `variable_uuid` is Some here.
                    use crate::routes::layers::db::{Column, Entity as LayerEntity};
                    let mut q = LayerEntity::find().filter(Column::CropId.eq(crop_uuid));
                    q = match variable_uuid {
                        Some(id) => q.filter(Column::VariableId.eq(id)),
                        None => q.filter(Column::VariableId.is_null()),
                    };
                    q = match params.project_id {
                        Some(pid) => q.filter(Column::ProjectId.eq(pid)),
                        None => q.filter(Column::ProjectId.is_null()),
                    };
                    q
                }
            };

            let existing_layer = duplicate_query.one(db).await.map_err(db_upload_err)?;

            // Track if we're updating an existing layer (to preserve DB record)
            let existing_layer_id: Option<Uuid> = if let Some(existing) = existing_layer {
                if overwrite_duplicates {
                    info!(
                        layer = existing.filename.clone().unwrap_or_else(|| "unknown".to_string()),
                        "Found existing layer, will update record and replace S3 file"
                    );
                    Some(existing.id)
                } else {
                    warn!(filename, "Rejecting duplicate file");
                    return Err(upload_err_json(
                        StatusCode::CONFLICT,
                        UploadError::new(
                            "duplicate",
                            format!(
                                "Layer already exists for {}. Delete layer first to re-upload, or set overwrite_duplicates=true",
                                filename
                            ),
                        ),
                    ));
                }
            } else {
                None
            };

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

            // Upload to S3 under the project's subpath, so the same filename
            // in two different projects doesn't collide on a single object.
            // S3 PUTs create the key unconditionally — there is no separate
            // "folder" to ensure exists.
            let s3_key = storage::get_s3_key(&config, params.project_id, &filename);
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

            // Create or update layer record in database
            let layer_name = filename.strip_suffix(".tif").unwrap_or(&filename);
            let cog_file_size = cog_bytes.len() as i64;
            let now = chrono::Utc::now();

            // Different details message for new upload vs reupload
            let stats_status_json = if existing_layer_id.is_some() {
                serde_json::json!({
                    "status": "success",
                    "last_run": now,
                    "error": null,
                    "details": format!("File reuploaded on {} - min: {}, max: {}, avg: {}, file_size: {} bytes",
                        now.format("%Y-%m-%d %H:%M UTC"), min_val, max_val, global_avg, cog_file_size)
                })
            } else {
                serde_json::json!({
                    "status": "success",
                    "last_run": now,
                    "error": null,
                    "details": format!("Initial upload - min: {}, max: {}, avg: {}, file_size: {} bytes", min_val, max_val, global_avg, cog_file_size)
                })
            };

            let saved_layer = if let Some(existing_id) = existing_layer_id {
                // Update existing layer record (preserves style_id, enabled, uploaded_at, etc.)
                use crate::routes::layers::db::ActiveModel as LayerActiveModel;
                debug!(filename, "Updating existing layer record");

                let update_model = LayerActiveModel {
                    id: Set(existing_id),
                    min_value: Set(Some(min_val)),
                    max_value: Set(Some(max_val)),
                    global_average: Set(Some(global_avg)),
                    file_size: Set(Some(cog_file_size)),
                    stats_status: Set(Some(stats_status_json.clone())),
                    last_updated: Set(now),
                    ..Default::default()
                };

                match update_model.update(db).await {
                    Ok(layer) => {
                        info!(filename, "Successfully updated existing layer record");
                        layer
                    }
                    Err(e) => {
                        error!(filename, error = %e, "Failed to update layer in database");
                        return Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "message": "Failed to update layer in database",
                                "error": e.to_string()
                            })),
                        ));
                    }
                }
            } else {
                // Create new layer record
                debug!(layer_name, "Creating new layer record");
                let layer_record = match &layer_info {
                    LayerInfo::Climate(info) => {
                        use crate::routes::layers::db::ActiveModel as LayerActiveModel;
                        LayerActiveModel {
                            id: Set(Uuid::new_v4()),
                            filename: Set(Some(filename.clone())),
                            layer_name: Set(Some(layer_name.to_string())),
                            crop_id: Set(Some(crop_uuid)),
                            water_model_id: Set(water_model_uuid),
                            climate_model_id: Set(climate_model_uuid),
                            scenario_id: Set(scenario_uuid),
                            variable_id: Set(variable_uuid),
                            year: Set(info.year),
                            min_value: Set(Some(min_val)),
                            max_value: Set(Some(max_val)),
                            global_average: Set(Some(global_avg)),
                            file_size: Set(Some(cog_file_size)),
                            stats_status: Set(Some(stats_status_json.clone())),
                            project_id: Set(params.project_id),
                            enabled: Set(true),
                            ..Default::default()
                        }
                    }
                    LayerInfo::Crop(_info) => {
                        use crate::routes::layers::db::ActiveModel as LayerActiveModel;
                        LayerActiveModel {
                            id: Set(Uuid::new_v4()),
                            filename: Set(Some(filename.clone())),
                            layer_name: Set(Some(layer_name.to_string())),
                            crop_id: Set(Some(crop_uuid)),
                            variable_id: Set(variable_uuid),
                            min_value: Set(Some(min_val)),
                            max_value: Set(Some(max_val)),
                            global_average: Set(Some(global_avg)),
                            file_size: Set(Some(cog_file_size)),
                            stats_status: Set(Some(stats_status_json.clone())),
                            project_id: Set(params.project_id),
                            enabled: Set(true),
                            ..Default::default()
                        }
                    }
                };

                match layer_record.insert(db).await {
                    Ok(layer) => {
                        info!(filename, "Successfully saved new layer to database");
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

    // Fetch the file directly from S3 (bypassing cache to avoid polluting Redis)
    let object = storage::get_object_direct(&config, layer.project_id, &filename).await.map_err(|e| {
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
    /// Limit number of layers to process
    pub limit: Option<u64>,
    /// Filter by stats_status_value: "success", "error", "pending", or "null" for layers never calculated
    pub stats_status_value: Option<String>,
    /// Force restart: cancel any running job and start a new one
    pub force: Option<bool>,
}

/// Request body for bulk recalculation by IDs
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct BulkRecalculateByIdsRequest {
    /// List of layer IDs to recalculate
    pub ids: Vec<Uuid>,
}

/// Response when starting a background recalculation job
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct RecalculateJobStartResponse {
    /// Whether the job was started successfully
    pub started: bool,
    /// Message describing the result
    pub message: String,
    /// Number of layers queued for processing
    pub total_layers: u64,
}

#[utoipa::path(
    post,
    path = "/recalculate-stats",
    params(BulkRecalculateParams),
    responses(
        (status = 200, description = "Background job started", body = RecalculateJobStartResponse),
        (status = 409, description = "Job already running"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Start background bulk recalculation of layer statistics",
    description = "Starts a background job to recalculate statistics for all layers. Returns immediately. Use GET /recalculate-stats/status to check progress."
)]
pub async fn recalculate_all_layer_stats(
    Query(params): Query<BulkRecalculateParams>,
    State(app_state): State<AppState>,
) -> Result<Json<RecalculateJobStartResponse>, (StatusCode, Json<serde_json::Value>)> {
    let db = &app_state.db;
    let config = &app_state.config;

    // Generate a unique worker ID for this request
    let worker_id = super::worker::generate_worker_id();

    // Check if a job is already running
    let current_status = super::jobs::get_job_status(config).await;
    if current_status.is_running {
        // If force=true, clear the existing job and start fresh
        if params.force.unwrap_or(false) {
            info!("Force restart requested, clearing existing job");
            if let Err(e) = super::jobs::clear_job_data(config).await {
                warn!(error = %e, "Failed to clear job data");
            }
        } else {
            return Err((
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "message": "A recalculation job is already running. Use force=true to cancel and restart.",
                    "started_at": current_status.started_at,
                    "progress": format!("{}/{}", current_status.processed_count, current_status.total_layers),
                    "active_workers": current_status.active_workers
                })),
            ));
        }
    }

    // Resolve slug filters to UUIDs
    let crop_filter_id = if let Some(crop) = &params.crop {
        Some(
            crate::routes::crops::db::Entity::find()
                .filter(crate::routes::crops::db::Column::Slug.eq(crop.as_str()))
                .one(db)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "Database error", "error": e.to_string()}))))?
                .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": format!("Unknown crop slug: {}", crop)}))))?
                .id,
        )
    } else {
        None
    };

    let variable_filter_id = if let Some(variable) = &params.variable {
        Some(
            crate::routes::variables::db::Entity::find()
                .filter(crate::routes::variables::db::Column::Slug.eq(variable.as_str()))
                .one(db)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "Database error", "error": e.to_string()}))))?
                .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": format!("Unknown variable slug: {}", variable)}))))?
                .id,
        )
    } else {
        None
    };

    let water_model_filter_id = if let Some(water_model) = &params.water_model {
        Some(
            crate::routes::water_models::db::Entity::find()
                .filter(crate::routes::water_models::db::Column::Slug.eq(water_model.as_str()))
                .one(db)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "Database error", "error": e.to_string()}))))?
                .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": format!("Unknown water_model slug: {}", water_model)}))))?
                .id,
        )
    } else {
        None
    };

    let climate_model_filter_id = if let Some(climate_model) = &params.climate_model {
        Some(
            crate::routes::climate_models::db::Entity::find()
                .filter(crate::routes::climate_models::db::Column::Slug.eq(climate_model.as_str()))
                .one(db)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "Database error", "error": e.to_string()}))))?
                .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": format!("Unknown climate_model slug: {}", climate_model)}))))?
                .id,
        )
    } else {
        None
    };

    let scenario_filter_id = if let Some(scenario) = &params.scenario {
        Some(
            crate::routes::scenarios::db::Entity::find()
                .filter(crate::routes::scenarios::db::Column::Slug.eq(scenario.as_str()))
                .one(db)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"message": "Database error", "error": e.to_string()}))))?
                .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({"message": format!("Unknown scenario slug: {}", scenario)}))))?
                .id,
        )
    } else {
        None
    };

    // Build query with resolved UUID filters
    let mut query = super::db::Entity::find();

    if let Some(id) = crop_filter_id {
        query = query.filter(super::db::Column::CropId.eq(id));
    }
    if let Some(id) = variable_filter_id {
        query = query.filter(super::db::Column::VariableId.eq(id));
    }
    if let Some(id) = water_model_filter_id {
        query = query.filter(super::db::Column::WaterModelId.eq(id));
    }
    if let Some(id) = climate_model_filter_id {
        query = query.filter(super::db::Column::ClimateModelId.eq(id));
    }
    if let Some(id) = scenario_filter_id {
        query = query.filter(super::db::Column::ScenarioId.eq(id));
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

    // Filter by stats_status_value
    if let Some(status_filter) = &params.stats_status_value {
        if status_filter == "null" || status_filter.is_empty() {
            query = query.filter(super::db::Column::StatsStatusValue.is_null());
        } else {
            query = query.filter(super::db::Column::StatsStatusValue.eq(status_filter));
        }
    }

    // Get the layers to process
    let layers: Vec<super::db::Model> = if let Some(limit) = params.limit {
        query.limit(limit).all(db)
    } else {
        query.all(db)
    }
        .await
        .map_err(|e| {
            error!(error = %e, "Database error fetching layers");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "message": "Database error", "error": e.to_string() })),
            )
        })?;

    if layers.is_empty() {
        return Ok(Json(RecalculateJobStartResponse {
            started: false,
            message: "No layers match the specified filters".to_string(),
            total_layers: 0,
        }));
    }

    // Extract layer IDs
    let layer_ids: Vec<Uuid> = layers.iter().map(|l| l.id).collect();
    let total_layers = layer_ids.len() as u64;

    // Mark all selected layers as 'pending' in the database
    let pending_status = serde_json::json!({
        "status": "pending",
        "last_run": chrono::Utc::now(),
        "error": null,
        "details": "Queued for distributed bulk recalculation"
    });

    if let Err(e) = super::db::Entity::update_many()
        .col_expr(super::db::Column::StatsStatus, sea_orm::sea_query::Expr::value(pending_status))
        .filter(super::db::Column::Id.is_in(layer_ids.clone()))
        .exec(db)
        .await
    {
        warn!(error = %e, "Failed to mark layers as pending (continuing anyway)");
    }

    // Start the distributed job - populate Redis queue
    match super::jobs::start_job(config, layer_ids, &worker_id).await {
        Ok(total) => {
            info!(total_layers = total, worker_id, "Started distributed recalculation job");
            Ok(Json(RecalculateJobStartResponse {
                started: true,
                message: format!("Distributed job started with {} layers. Workers will process automatically.", total),
                total_layers,
            }))
        }
        Err(e) => {
            error!(error = %e, "Failed to start distributed job");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "message": "Failed to start job", "error": e })),
            ))
        }
    }
}

/// Response for job status endpoint
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct RecalculateJobStatusResponse {
    /// Whether a job is currently running
    pub is_running: bool,
    /// When the job started (ISO 8601)
    pub started_at: Option<String>,
    /// Total layers to process
    pub total_layers: u64,
    /// Layers in the todo queue
    pub todo_count: u64,
    /// Layers currently being processed by workers
    pub processing_count: u64,
    /// Layers processed so far (completed + errors)
    pub processed_count: u64,
    /// Successful recalculations
    pub success_count: u64,
    /// Failed recalculations
    pub error_count: u64,
    /// Progress percentage (0-100)
    pub progress_percent: f64,
    /// Elapsed time in seconds
    pub elapsed_seconds: Option<i64>,
    /// Recent errors (last 10)
    pub recent_errors: Vec<String>,
    /// When the job completed (if finished)
    pub completed_at: Option<String>,
    /// Who started the job
    pub started_by: Option<String>,
    /// Number of active workers processing items
    pub active_workers: u64,
    /// Whether any items appear stale (no progress for >60s)
    pub has_stale_items: bool,
    /// Count of stale items
    pub stale_count: u64,
}

#[utoipa::path(
    get,
    path = "/recalculate-stats/status",
    responses(
        (status = 200, description = "Job status", body = RecalculateJobStatusResponse)
    ),
    summary = "Get recalculation job status",
    description = "Returns the current status of the background recalculation job, including progress and any errors."
)]
pub async fn get_recalculate_job_status(
    State(app_state): State<AppState>,
) -> Json<RecalculateJobStatusResponse> {
    let status = super::jobs::get_job_status(&app_state.config).await;

    Json(RecalculateJobStatusResponse {
        is_running: status.is_running,
        started_at: status.started_at.map(|t| t.to_rfc3339()),
        total_layers: status.total_layers,
        todo_count: status.todo_count,
        processing_count: status.processing_count,
        processed_count: status.processed_count,
        success_count: status.success_count,
        error_count: status.error_count,
        progress_percent: status.progress_percent(),
        elapsed_seconds: status.elapsed_seconds(),
        recent_errors: status.recent_errors,
        completed_at: status.completed_at.map(|t| t.to_rfc3339()),
        started_by: status.started_by,
        active_workers: status.active_workers,
        has_stale_items: status.has_stale_items,
        stale_count: status.stale_count,
    })
}

/// Response for cancel job endpoint
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct CancelJobResponse {
    /// Whether cancellation was requested
    pub cancelled: bool,
    /// Message describing the result
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/recalculate-stats/cancel",
    responses(
        (status = 200, description = "Cancellation requested", body = CancelJobResponse),
        (status = 400, description = "No job running")
    ),
    summary = "Cancel the running recalculation job",
    description = "Requests cancellation of the currently running background recalculation job. The job will stop after completing the current layer."
)]
pub async fn cancel_recalculate_job(
    State(app_state): State<AppState>,
) -> Result<Json<CancelJobResponse>, (StatusCode, Json<serde_json::Value>)> {
    let status = super::jobs::get_job_status(&app_state.config).await;

    if !status.is_running {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "message": "No recalculation job is currently running"
            })),
        ));
    }

    if let Err(e) = super::jobs::request_cancellation(&app_state.config).await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "message": "Failed to request cancellation",
                "error": e
            })),
        ));
    }

    Ok(Json(CancelJobResponse {
        cancelled: true,
        message: "Cancellation requested. Job will stop after completing the current layer.".to_string(),
    }))
}

#[utoipa::path(
    post,
    path = "/recalculate-stats-by-ids",
    request_body = BulkRecalculateByIdsRequest,
    responses(
        (status = 200, description = "Bulk recalculation completed", body = BulkRecalculateResponse),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Bulk recalculate layer statistics by IDs",
    description = "Recalculates statistics for specific layers identified by their IDs"
)]
pub async fn recalculate_stats_by_ids(
    State(app_state): State<AppState>,
    Json(body): Json<BulkRecalculateByIdsRequest>,
) -> Result<Json<BulkRecalculateResponse>, (StatusCode, Json<serde_json::Value>)> {
    let db = &app_state.db;
    let config = &app_state.config;

    if body.ids.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "message": "No layer IDs provided" })),
        ));
    }

    // Fetch all layers by IDs
    let layers = super::db::Entity::find()
        .filter(super::db::Column::Id.is_in(body.ids.clone()))
        .all(db)
        .await
        .map_err(|e| {
            error!(error = %e, "Database error fetching layers");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "message": "Database error", "error": e.to_string() })),
            )
        })?;

    info!(count = layers.len(), requested = body.ids.len(), "Starting bulk recalculation by IDs");

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

        // Fetch directly from S3 (bypassing cache to avoid polluting Redis)
        let object = match storage::get_object_direct(&config, layer.project_id, &filename).await {
            Ok(o) => o,
            Err(e) => {
                let error_msg = format!("Failed to fetch from S3: {}", e);
                errors.push(format!("Layer {}: {}", layer_id, error_msg));
                // Update stats_status with error
                let mut active_layer: super::db::ActiveModel = layer.into();
                active_layer.stats_status = Set(Some(serde_json::json!({
                    "status": "error",
                    "last_run": chrono::Utc::now(),
                    "error": error_msg,
                    "details": format!("filename: {}", filename)
                })));
                let _ = active_layer.update(db).await;
                error_count += 1;
                continue;
            }
        };

        let file_size = object.len() as i64;

        // Validate file size
        if file_size < 1024 {
            let error_msg = format!("File too small: {} bytes", file_size);
            errors.push(format!("Layer {}: {}", layer_id, error_msg));
            let mut active_layer: super::db::ActiveModel = layer.into();
            active_layer.stats_status = Set(Some(serde_json::json!({
                "status": "error",
                "last_run": chrono::Utc::now(),
                "error": error_msg,
                "details": format!("filename: {}", filename)
            })));
            active_layer.file_size = Set(Some(file_size));
            let _ = active_layer.update(db).await;
            error_count += 1;
            continue;
        }

        // Calculate statistics
        let (min_val, max_val) = match get_min_max_of_raster(&object) {
            Ok(v) => v,
            Err(e) => {
                let error_msg = format!("Failed to calculate min/max: {}", e);
                errors.push(format!("Layer {}: {}", layer_id, error_msg));
                let mut active_layer: super::db::ActiveModel = layer.into();
                active_layer.stats_status = Set(Some(serde_json::json!({
                    "status": "error",
                    "last_run": chrono::Utc::now(),
                    "error": error_msg,
                    "details": format!("filename: {}, file_size: {} bytes", filename, file_size)
                })));
                active_layer.file_size = Set(Some(file_size));
                let _ = active_layer.update(db).await;
                error_count += 1;
                continue;
            }
        };

        let global_avg = match get_global_average_of_raster(&object) {
            Ok(v) => v,
            Err(e) => {
                let error_msg = format!("Failed to calculate average: {}", e);
                errors.push(format!("Layer {}: {}", layer_id, error_msg));
                let mut active_layer: super::db::ActiveModel = layer.into();
                active_layer.stats_status = Set(Some(serde_json::json!({
                    "status": "error",
                    "last_run": chrono::Utc::now(),
                    "error": error_msg,
                    "details": format!("filename: {}, file_size: {} bytes", filename, file_size)
                })));
                active_layer.file_size = Set(Some(file_size));
                let _ = active_layer.update(db).await;
                error_count += 1;
                continue;
            }
        };

        // Validate
        if !min_val.is_finite() || !max_val.is_finite() || !global_avg.is_finite() {
            let error_msg = "Invalid statistics (NaN/Inf)";
            errors.push(format!("Layer {}: {}", layer_id, error_msg));
            let mut active_layer: super::db::ActiveModel = layer.into();
            active_layer.stats_status = Set(Some(serde_json::json!({
                "status": "error",
                "last_run": chrono::Utc::now(),
                "error": error_msg,
                "details": format!("filename: {}, file_size: {} bytes", filename, file_size)
            })));
            active_layer.file_size = Set(Some(file_size));
            let _ = active_layer.update(db).await;
            error_count += 1;
            continue;
        }

        // Store previous values
        let previous_min = layer.min_value;
        let previous_max = layer.max_value;
        let previous_avg = layer.global_average;

        // Update with success
        let mut active_layer: super::db::ActiveModel = layer.into();
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

    info!(success_count, error_count, "Bulk recalculation by IDs completed");

    Ok(Json(BulkRecalculateResponse {
        success_count,
        error_count,
        results,
        errors,
    }))
}

