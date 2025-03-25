use crate::common::auth::Role;
// use axum::{Json, extract::State, http::StatusCode};
use super::models::{Layer, LayerCreate, LayerUpdate};
use crate::routes::tiles::storage;
use axum::response::IntoResponse;
use axum_keycloak_auth::{
    PassthroughMode, instance::KeycloakAuthInstance, layer::KeycloakAuthLayer,
};
use crudcrate::{CRUDResource, crud_handlers};
use gdal::Dataset;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, LoaderTrait, QueryFilter, QuerySelect,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
crud_handlers!(Layer, LayerUpdate, LayerCreate);

pub fn router(
    db: &DatabaseConnection,
    keycloak_auth_instance: Option<Arc<KeycloakAuthInstance>>,
) -> OpenApiRouter
where
    Layer: CRUDResource,
{
    let mut mutating_router = OpenApiRouter::new()
        .routes(routes!(get_one_handler))
        .routes(routes!(get_all_handler))
        .routes(routes!(create_one_handler))
        .routes(routes!(update_one_handler))
        .routes(routes!(delete_one_handler))
        .routes(routes!(delete_many_handler))
        .routes(routes!(get_groups))
        .routes(routes!(get_all_map_layers))
        .routes(routes!(get_pixel_value))
        .with_state(db.clone());

    if let Some(instance) = keycloak_auth_instance {
        mutating_router = mutating_router.layer(
            KeycloakAuthLayer::<Role>::builder()
                .instance(instance)
                .passthrough_mode(PassthroughMode::Block)
                .persist_raw_claims(false)
                .expected_audiences(vec![String::from("account")])
                .required_roles(vec![Role::Administrator])
                .build(),
        );
    } else {
        println!(
            "Warning: Mutating routes of {} router are not protected",
            Layer::RESOURCE_NAME_PLURAL
        );
    }

    mutating_router
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
        (status = 200, description = "Layer list", body = [Layer]),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get all enabled layers for map",
    description = "Fetches enabled layers with filtering for use in Drop4Crop map"
)]
pub async fn get_all_map_layers(
    State(db): State<DatabaseConnection>,
    Query(params): Query<LayerQueryParams>,
) -> Result<Json<Vec<Layer>>, (StatusCode, Json<String>)> {
    use crate::routes::layers::db::{Column, Entity as LayerEntity};
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

    let layers = query.all(&db).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(format!("DB error: {}", e)),
        )
    })?;

    let style = layers
        .load_one(crate::routes::styles::db::Entity, &db)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(format!("DB error: {}", e)),
            )
        })?;

    let mut response: Vec<Layer> = vec![];
    for (layer, style) in layers.into_iter().zip(style.into_iter()) {
        let layer: Layer = match style {
            Some(style) => Layer::from((layer, style)),
            None => Layer::from(layer),
        };
        response.push(layer);
    }
    // println!("[get_all_map_layers] response: {:?}", response);
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
    let value = buf.get(0).cloned().unwrap_or(0.0);

    let response = PixelValueResponse { value };
    Ok(Json(response))
}
