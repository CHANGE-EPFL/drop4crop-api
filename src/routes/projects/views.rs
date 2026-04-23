pub use super::db::Project;
use crate::common::auth::Role;
use crate::common::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum_keycloak_auth::{PassthroughMode, layer::KeycloakAuthLayer};
use crudcrate::CRUDResource;
use hyper::StatusCode;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use tracing::warn;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

pub fn router(state: &AppState) -> OpenApiRouter {
    let public_router = OpenApiRouter::new()
        .routes(routes!(get_active_projects))
        .routes(routes!(get_project_config))
        .with_state(state.clone());

    let mut protected_router = Project::router(&state.db.clone());

    let protected_custom_routes = OpenApiRouter::new()
        .routes(routes!(set_project_crops))
        .routes(routes!(set_project_water_models))
        .routes(routes!(set_project_climate_models))
        .routes(routes!(set_project_scenarios))
        .routes(routes!(set_project_variables))
        .with_state(state.clone());

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
        warn!(
            resource = Project::RESOURCE_NAME_PLURAL,
            "Mutating routes are not protected"
        );
    }

    public_router.merge(protected_router)
}

#[utoipa::path(
    get,
    path = "/active",
    responses(
        (status = 200, description = "List of all projects ordered by sort_order", body = Vec<super::db::Project>),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get all projects for splash page",
    description = "Returns all projects ordered by sort_order. Both enabled and disabled projects are returned so the UI can show 'Coming Soon' cards."
)]
pub async fn get_active_projects(
    State(app_state): State<AppState>,
) -> Result<Json<Vec<super::db::Project>>, (StatusCode, Json<String>)> {
    let db = &app_state.db;

    let projects = super::db::Entity::find()
        .order_by_asc(super::db::Column::SortOrder)
        .all(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    // Convert Sea-ORM models to API structs
    let projects: Vec<super::db::Project> = projects.into_iter().map(|m| m.into()).collect();

    Ok(Json(projects))
}

// ---------------------------------------------------------------------------
// GET /config/{slug} - Public endpoint returning full project configuration
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/config/{slug}",
    params(
        ("slug" = String, Path, description = "Project slug")
    ),
    responses(
        (status = 200, description = "Full project configuration"),
        (status = 404, description = "Project not found"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get full project configuration by slug",
    description = "Returns the project and all associated crops, water models, climate models, scenarios, and variables via junction tables."
)]
pub async fn get_project_config(
    Path(slug): Path<String>,
    State(app_state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<String>)> {
    let db = &app_state.db;

    // Find the project by slug
    let project = super::db::Entity::find()
        .filter(super::db::Column::Slug.eq(&slug))
        .one(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json("Project not found".to_string())))?;

    let project_id = project.id;

    // Query each junction table and load the full reference entities
    let crop_junctions = super::project_crop::Entity::find()
        .filter(super::project_crop::Column::ProjectId.eq(project_id))
        .order_by_asc(super::project_crop::Column::SortOrder)
        .all(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    let mut crops = Vec::new();
    for junc in &crop_junctions {
        if let Some(c) = crate::routes::crops::db::Entity::find_by_id(junc.crop_id)
            .one(db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
        {
            crops.push(serde_json::json!({
                "id": c.id,
                "slug": c.slug,
                "name": c.name,
                "sort_order": junc.sort_order,
            }));
        }
    }

    let wm_junctions = super::project_water_model::Entity::find()
        .filter(super::project_water_model::Column::ProjectId.eq(project_id))
        .order_by_asc(super::project_water_model::Column::SortOrder)
        .all(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    let mut water_models = Vec::new();
    for junc in &wm_junctions {
        if let Some(w) = crate::routes::water_models::db::Entity::find_by_id(junc.water_model_id)
            .one(db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
        {
            water_models.push(serde_json::json!({
                "id": w.id,
                "slug": w.slug,
                "name": w.name,
                "sort_order": junc.sort_order,
            }));
        }
    }

    let cm_junctions = super::project_climate_model::Entity::find()
        .filter(super::project_climate_model::Column::ProjectId.eq(project_id))
        .order_by_asc(super::project_climate_model::Column::SortOrder)
        .all(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    let mut climate_models = Vec::new();
    for junc in &cm_junctions {
        if let Some(c) =
            crate::routes::climate_models::db::Entity::find_by_id(junc.climate_model_id)
                .one(db)
                .await
                .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
        {
            climate_models.push(serde_json::json!({
                "id": c.id,
                "slug": c.slug,
                "name": c.name,
                "sort_order": junc.sort_order,
            }));
        }
    }

    let sc_junctions = super::project_scenario::Entity::find()
        .filter(super::project_scenario::Column::ProjectId.eq(project_id))
        .order_by_asc(super::project_scenario::Column::SortOrder)
        .all(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    let mut scenarios = Vec::new();
    for junc in &sc_junctions {
        if let Some(s) = crate::routes::scenarios::db::Entity::find_by_id(junc.scenario_id)
            .one(db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
        {
            scenarios.push(serde_json::json!({
                "id": s.id,
                "slug": s.slug,
                "name": s.name,
                "sort_order": junc.sort_order,
            }));
        }
    }

    let var_junctions = super::project_variable::Entity::find()
        .filter(super::project_variable::Column::ProjectId.eq(project_id))
        .order_by_asc(super::project_variable::Column::SortOrder)
        .all(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    let mut variables = Vec::new();
    for junc in &var_junctions {
        if let Some(v) = crate::routes::variables::db::Entity::find_by_id(junc.variable_id)
            .one(db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
        {
            variables.push(serde_json::json!({
                "id": v.id,
                "slug": v.slug,
                "name": v.name,
                "abbreviation": v.abbreviation,
                "subscript": v.subscript,
                "unit": v.unit,
                "is_crop_specific": v.is_crop_specific,
                "group_name": v.group_name,
                "sort_order": junc.sort_order,
            }));
        }
    }

    let project_json: super::db::Project = project.into();

    Ok(Json(serde_json::json!({
        "project": project_json,
        "crops": crops,
        "water_models": water_models,
        "climate_models": climate_models,
        "scenarios": scenarios,
        "variables": variables,
    })))
}

// ---------------------------------------------------------------------------
// PUT /{id}/crops - Replace all crops for a project
// ---------------------------------------------------------------------------

#[utoipa::path(
    put,
    path = "/{id}/crops",
    params(
        ("id" = Uuid, Path, description = "Project ID")
    ),
    request_body = Vec<Uuid>,
    responses(
        (status = 200, description = "Crops updated", body = serde_json::Value),
        (status = 404, description = "Project not found"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Set crops for a project",
    description = "Replaces all crop associations for the given project with the provided list of crop IDs."
)]
pub async fn set_project_crops(
    Path(id): Path<Uuid>,
    State(app_state): State<AppState>,
    Json(crop_ids): Json<Vec<Uuid>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<String>)> {
    let db = &app_state.db;

    // Verify project exists
    super::db::Entity::find_by_id(id)
        .one(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json("Project not found".to_string())))?;

    // Delete existing junction rows
    super::project_crop::Entity::delete_many()
        .filter(super::project_crop::Column::ProjectId.eq(id))
        .exec(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    // Insert new junction rows — list position becomes the per-project sort_order.
    for (idx, crop_id) in crop_ids.iter().enumerate() {
        let model = super::project_crop::ActiveModel {
            project_id: Set(id),
            crop_id: Set(*crop_id),
            sort_order: Set(idx as i32),
        };
        model
            .insert(db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;
    }

    Ok(Json(serde_json::json!({ "count": crop_ids.len() })))
}

// ---------------------------------------------------------------------------
// PUT /{id}/water-models - Replace all water models for a project
// ---------------------------------------------------------------------------

#[utoipa::path(
    put,
    path = "/{id}/water-models",
    params(
        ("id" = Uuid, Path, description = "Project ID")
    ),
    request_body = Vec<Uuid>,
    responses(
        (status = 200, description = "Water models updated", body = serde_json::Value),
        (status = 404, description = "Project not found"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Set water models for a project",
    description = "Replaces all water model associations for the given project with the provided list of water model IDs."
)]
pub async fn set_project_water_models(
    Path(id): Path<Uuid>,
    State(app_state): State<AppState>,
    Json(water_model_ids): Json<Vec<Uuid>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<String>)> {
    let db = &app_state.db;

    super::db::Entity::find_by_id(id)
        .one(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json("Project not found".to_string())))?;

    super::project_water_model::Entity::delete_many()
        .filter(super::project_water_model::Column::ProjectId.eq(id))
        .exec(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    for (idx, wm_id) in water_model_ids.iter().enumerate() {
        let model = super::project_water_model::ActiveModel {
            project_id: Set(id),
            water_model_id: Set(*wm_id),
            sort_order: Set(idx as i32),
        };
        model
            .insert(db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;
    }

    Ok(Json(
        serde_json::json!({ "count": water_model_ids.len() }),
    ))
}

// ---------------------------------------------------------------------------
// PUT /{id}/climate-models - Replace all climate models for a project
// ---------------------------------------------------------------------------

#[utoipa::path(
    put,
    path = "/{id}/climate-models",
    params(
        ("id" = Uuid, Path, description = "Project ID")
    ),
    request_body = Vec<Uuid>,
    responses(
        (status = 200, description = "Climate models updated", body = serde_json::Value),
        (status = 404, description = "Project not found"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Set climate models for a project",
    description = "Replaces all climate model associations for the given project with the provided list of climate model IDs."
)]
pub async fn set_project_climate_models(
    Path(id): Path<Uuid>,
    State(app_state): State<AppState>,
    Json(climate_model_ids): Json<Vec<Uuid>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<String>)> {
    let db = &app_state.db;

    super::db::Entity::find_by_id(id)
        .one(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json("Project not found".to_string())))?;

    super::project_climate_model::Entity::delete_many()
        .filter(super::project_climate_model::Column::ProjectId.eq(id))
        .exec(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    for (idx, cm_id) in climate_model_ids.iter().enumerate() {
        let model = super::project_climate_model::ActiveModel {
            project_id: Set(id),
            climate_model_id: Set(*cm_id),
            sort_order: Set(idx as i32),
        };
        model
            .insert(db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;
    }

    Ok(Json(
        serde_json::json!({ "count": climate_model_ids.len() }),
    ))
}

// ---------------------------------------------------------------------------
// PUT /{id}/scenarios - Replace all scenarios for a project
// ---------------------------------------------------------------------------

#[utoipa::path(
    put,
    path = "/{id}/scenarios",
    params(
        ("id" = Uuid, Path, description = "Project ID")
    ),
    request_body = Vec<Uuid>,
    responses(
        (status = 200, description = "Scenarios updated", body = serde_json::Value),
        (status = 404, description = "Project not found"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Set scenarios for a project",
    description = "Replaces all scenario associations for the given project with the provided list of scenario IDs."
)]
pub async fn set_project_scenarios(
    Path(id): Path<Uuid>,
    State(app_state): State<AppState>,
    Json(scenario_ids): Json<Vec<Uuid>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<String>)> {
    let db = &app_state.db;

    super::db::Entity::find_by_id(id)
        .one(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json("Project not found".to_string())))?;

    super::project_scenario::Entity::delete_many()
        .filter(super::project_scenario::Column::ProjectId.eq(id))
        .exec(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    for (idx, sc_id) in scenario_ids.iter().enumerate() {
        let model = super::project_scenario::ActiveModel {
            project_id: Set(id),
            scenario_id: Set(*sc_id),
            sort_order: Set(idx as i32),
        };
        model
            .insert(db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;
    }

    Ok(Json(serde_json::json!({ "count": scenario_ids.len() })))
}

// ---------------------------------------------------------------------------
// PUT /{id}/variables - Replace all variables for a project
// ---------------------------------------------------------------------------

#[utoipa::path(
    put,
    path = "/{id}/variables",
    params(
        ("id" = Uuid, Path, description = "Project ID")
    ),
    request_body = Vec<Uuid>,
    responses(
        (status = 200, description = "Variables updated", body = serde_json::Value),
        (status = 404, description = "Project not found"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Set variables for a project",
    description = "Replaces all variable associations for the given project with the provided list of variable IDs."
)]
pub async fn set_project_variables(
    Path(id): Path<Uuid>,
    State(app_state): State<AppState>,
    Json(variable_ids): Json<Vec<Uuid>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<String>)> {
    let db = &app_state.db;

    super::db::Entity::find_by_id(id)
        .one(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json("Project not found".to_string())))?;

    super::project_variable::Entity::delete_many()
        .filter(super::project_variable::Column::ProjectId.eq(id))
        .exec(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    for (idx, var_id) in variable_ids.iter().enumerate() {
        let model = super::project_variable::ActiveModel {
            project_id: Set(id),
            variable_id: Set(*var_id),
            sort_order: Set(idx as i32),
        };
        model
            .insert(db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;
    }

    Ok(Json(serde_json::json!({ "count": variable_ids.len() })))
}
