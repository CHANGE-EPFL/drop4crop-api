use crate::common::auth::Role;
// use axum::{Json, extract::State, http::StatusCode};
use axum_keycloak_auth::{
    PassthroughMode, instance::KeycloakAuthInstance, layer::KeycloakAuthLayer,
};
use crudcrate::{CRUDResource, crud_handlers};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::models::{Layer, LayerCreate, LayerUpdate};

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

    let mut response = Vec::new();

    for layer in layers {
        let mut layer: super::models::Layer = layer.into();
        layer.style = vec![];

        let style = if !layer.style.is_empty() {
            sort_styles(layer.style.clone())
        } else {
            generate_grayscale_style(layer.min_value.unwrap(), layer.max_value.unwrap(), 10)
        };
        layer.style = style;
        // let mut read = Layer::from(layer);
        // read.style = style;
        response.push(layer);
    }

    Ok(Json(response))
}

pub fn sort_styles(
    mut style_list: Vec<crate::routes::styles::models::StyleItem>,
) -> Vec<crate::routes::styles::models::StyleItem> {
    style_list.sort_by(|a, b| {
        a.value
            .partial_cmp(&b.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    style_list
}

pub fn generate_grayscale_style(
    min: f64,
    max: f64,
    num_segments: usize,
) -> Vec<crate::routes::styles::models::StyleItem> {
    let step = (max - min) / num_segments as f64;
    let mut style = Vec::with_capacity(num_segments);

    for i in 0..num_segments {
        let value = min + i as f64 * step;
        let grey_value =
            ((255.0 * i as f64) / (num_segments.saturating_sub(1) as f64)).round() as u8;
        style.push(crate::routes::styles::models::StyleItem {
            value,
            red: grey_value,
            green: grey_value,
            blue: grey_value,
            opacity: 255,
            label: (value * 10000.0).round() / 10000.0, // round to 4 decimal places
        });
    }

    style
}
