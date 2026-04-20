pub use super::db::ShowcaseItem;
use crate::common::auth::Role;
use crate::common::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum_keycloak_auth::{PassthroughMode, layer::KeycloakAuthLayer};
use crudcrate::CRUDResource;
use hyper::StatusCode;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use tracing::warn;
use utoipa_axum::{router::OpenApiRouter, routes};

pub fn router(state: &AppState) -> OpenApiRouter {
    let public_router = OpenApiRouter::new()
        .routes(routes!(get_showcase_items_by_project))
        .with_state(state.clone());

    let mut crud_router = ShowcaseItem::router(&state.db.clone());

    if let Some(instance) = state.keycloak_auth_instance.clone() {
        crud_router = crud_router.layer(
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
            resource = ShowcaseItem::RESOURCE_NAME_PLURAL,
            "Mutating routes are not protected"
        );
    }

    public_router.merge(crud_router)
}

#[utoipa::path(
    get,
    path = "/by-project/{slug}",
    params(
        ("slug" = String, Path, description = "Project slug")
    ),
    responses(
        (status = 200, description = "Showcase items for the project"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get showcase items for a project by slug",
    description = "Returns enabled showcase items for a project, ordered by sort_order. Includes the associated layer data."
)]
pub async fn get_showcase_items_by_project(
    Path(slug): Path<String>,
    State(app_state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<String>)> {
    let db = &app_state.db;

    // Find the project by slug
    let project = crate::routes::projects::db::Entity::find()
        .filter(crate::routes::projects::db::Column::Slug.eq(&slug))
        .one(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    let project = match project {
        Some(p) => p,
        None => return Ok(Json(serde_json::json!([]))),
    };

    // Find showcase items for this project
    let items = super::db::Entity::find()
        .filter(super::db::Column::ProjectId.eq(project.id))
        .filter(super::db::Column::Enabled.eq(true))
        .order_by_asc(super::db::Column::SortOrder)
        .all(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    // For each item, fetch the associated layer and its reference data
    let mut results = Vec::new();
    for item in items {
        let layer = crate::routes::layers::db::Entity::find_by_id(item.layer_id)
            .one(db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

        if let Some(layer) = layer {
            // Resolve FK references
            let crop = if let Some(crop_id) = layer.crop_id {
                crate::routes::crops::db::Entity::find_by_id(crop_id).one(db).await.ok().flatten()
            } else { None };

            let variable = if let Some(var_id) = layer.variable_id {
                crate::routes::variables::db::Entity::find_by_id(var_id).one(db).await.ok().flatten()
            } else { None };

            let water_model = if let Some(wm_id) = layer.water_model_id {
                crate::routes::water_models::db::Entity::find_by_id(wm_id).one(db).await.ok().flatten()
            } else { None };

            let climate_model = if let Some(cm_id) = layer.climate_model_id {
                crate::routes::climate_models::db::Entity::find_by_id(cm_id).one(db).await.ok().flatten()
            } else { None };

            let scenario = if let Some(sc_id) = layer.scenario_id {
                crate::routes::scenarios::db::Entity::find_by_id(sc_id).one(db).await.ok().flatten()
            } else { None };

            results.push(serde_json::json!({
                "id": item.id,
                "title": item.title,
                "description": item.description,
                "sort_order": item.sort_order,
                "layer_id": item.layer_id,
                "year": layer.year,
                "crop": crop.map(|c| serde_json::json!({"id": c.id, "slug": c.slug, "name": c.name})),
                "variable": variable.map(|v| serde_json::json!({
                    "id": v.id, "slug": v.slug, "name": v.name,
                    "abbreviation": v.abbreviation, "subscript": v.subscript,
                    "unit": v.unit, "is_crop_specific": v.is_crop_specific,
                    "group_name": v.group_name
                })),
                "water_model": water_model.map(|w| serde_json::json!({"id": w.id, "slug": w.slug, "name": w.name})),
                "climate_model": climate_model.map(|c| serde_json::json!({"id": c.id, "slug": c.slug, "name": c.name})),
                "scenario": scenario.map(|s| serde_json::json!({"id": s.id, "slug": s.slug, "name": s.name})),
            }));
        }
    }

    Ok(Json(serde_json::json!(results)))
}
