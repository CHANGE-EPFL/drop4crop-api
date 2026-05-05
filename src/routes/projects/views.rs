pub use super::db::Project;
use crate::common::auth::Role;
use crate::common::state::AppState;
use crate::routes::layers::db as layer;
use crate::routes::styles::db as style;
use crate::routes::tiles::utils::XYZTile;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum_keycloak_auth::{PassthroughMode, layer::KeycloakAuthLayer};
use crudcrate::CRUDResource;
use hyper::StatusCode;
use image::ImageBuffer;
use redis;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, JsonValue, QueryFilter, QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use tokio_retry::{RetryIf, strategy::FixedInterval};
use tracing::{debug, error, warn};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

pub fn router(state: &AppState) -> OpenApiRouter {
    let public_router = OpenApiRouter::new()
        .routes(routes!(get_active_projects))
        .routes(routes!(get_project_config))
        .routes(routes!(get_project_card_tile))
        .with_state(state.clone());

    let mut protected_router = Project::router(&state.db.clone());

    // Cache invalidation: when a project's card settings change, clear card tiles.
    {
        let config = state.config.clone();
        let db = state.db.clone();
        protected_router =
            protected_router.layer(axum::middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
                let config = config.clone();
                let db = db.clone();
                async move {
                    let method = req.method().clone();
                    let path = req.uri().path().to_string();
                    let response = next.run(req).await;

                    let is_mutating = matches!(
                        method,
                        axum::http::Method::PUT | axum::http::Method::DELETE
                    );
                    if response.status().is_success() && is_mutating {
                        if let Some(project_id) = path.rsplit('/').find_map(|s| uuid::Uuid::parse_str(s).ok()) {
                            tokio::spawn(async move {
                                if let Ok(Some(project)) = super::db::Entity::find_by_id(project_id).one(&db).await {
                                    let _ = crate::routes::tiles::cache::invalidate_card_tiles(&config, &project.slug).await;
                                    crate::routes::tiles::warming::warm_card_tiles_for_project(&config, &db, &project).await;
                                }
                            });
                        }
                    }

                    response
                }
            }));
    }

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

/// Project shape returned by `/active`. Includes the resolved
/// `card_layer_name` so the UI can build the card-tile URL without a
/// secondary fetch — the layer UUID alone is not enough since the tile
/// endpoint identifies layers by `layer_name`.
#[derive(Serialize, ToSchema)]
pub struct ActiveProject {
    #[serde(flatten)]
    pub project: super::db::Project,
    pub card_layer_name: Option<String>,
}

#[utoipa::path(
    get,
    path = "/active",
    responses(
        (status = 200, description = "List of all projects ordered by sort_order", body = Vec<ActiveProject>),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get all projects for splash page",
    description = "Returns all projects ordered by sort_order. Both enabled and disabled projects are returned so the UI can show 'Coming Soon' cards."
)]
pub async fn get_active_projects(
    State(app_state): State<AppState>,
) -> Result<Json<Vec<ActiveProject>>, (StatusCode, Json<String>)> {
    let db = &app_state.db;

    let projects = super::db::Entity::find()
        .order_by_asc(super::db::Column::SortOrder)
        .all(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    let mut out = Vec::with_capacity(projects.len());
    for p in projects {
        let card_layer_name = if let Some(layer_id) = p.card_layer_id {
            layer::Entity::find_by_id(layer_id)
                .one(db)
                .await
                .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
                .and_then(|l| l.layer_name)
        } else {
            None
        };
        out.push(ActiveProject {
            project: p.into(),
            card_layer_name,
        });
    }

    Ok(Json(out))
}

// ---------------------------------------------------------------------------
// GET /{slug}/card-tile/{z}/{x}/{y} - Render a tile for the splash card
// preview using the project's chosen card_style_id (or layer default).
// ---------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct CardTileParams {
    layer: String,
}

/// Parse a tile coordinate, handling both integers and floats (truncating).
/// Mirrors the helper in `tiles::views` — kept local to avoid leaking it as
/// a public surface from the tiles module.
fn parse_tile_coord(s: &str) -> Result<u32, StatusCode> {
    if let Ok(v) = s.parse::<u32>() {
        return Ok(v);
    }
    let f = s.parse::<f64>().map_err(|_| StatusCode::BAD_REQUEST)?;
    if f < 0.0 {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(f.trunc() as u32)
}

#[utoipa::path(
    get,
    path = "/{slug}/card-tile/{z}/{x}/{y}",
    params(
        ("slug" = String, Path, description = "Project slug"),
        ("z" = String, Path, description = "Zoom level"),
        ("x" = String, Path, description = "Tile x coordinate"),
        ("y" = String, Path, description = "Tile y coordinate"),
        ("layer" = String, Query, description = "Layer name (must belong to the project)")
    ),
    responses(
        (status = 200, description = "Tile image", body = [u8], content_type = "image/png"),
        (status = 404, description = "Project, layer, or tile not found"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get a splash-card preview tile",
    description = "Renders the requested tile of the given layer using the project's `card_style_id` if set, otherwise the layer's own style. The style UUID is never exposed in the URL."
)]
#[axum::debug_handler]
pub async fn get_project_card_tile(
    Path((slug, z_str, x_str, y_str)): Path<(String, String, String, String)>,
    Query(params): Query<CardTileParams>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    let z = parse_tile_coord(&z_str)?;
    let x = parse_tile_coord(&x_str)?;
    let y = parse_tile_coord(&y_str)?;

    let db = &app_state.db;
    let config = &app_state.config;

    let max_tiles = 1u32 << z;
    if x >= max_tiles || y >= max_tiles {
        return Err(StatusCode::NOT_FOUND);
    }

    // Rendered-PNG cache: keyed on (layer_name, project_card_style_or_default,
    // z/x/y). We don't yet know which style will be applied without a DB
    // lookup, so the cache key uses a stable token tied to the project slug
    // — the style id is resolved server-side from the project record, so the
    // mapping (slug -> effective style) is deterministic.
    // Use the slug as a proxy for the style choice; a style change on the
    // project invalidates by writing a new entry under the same key once the
    // first request after the change repopulates it. For aggressive
    // invalidation, see `cache::remove_downloading_state_raw`.
    let png_key = crate::routes::tiles::cache::build_cache_key(
        config,
        &format!("png-card/{}/{}/{}/{}/{}", slug, params.layer, z, x, y),
    );
    if let Ok(client) = redis::Client::open(config.tile_cache_uri.clone())
        && let Ok(mut con) = client.get_multiplexed_async_connection().await
        && let Ok(Some(cached)) = crate::routes::tiles::cache::redis_get(
            &mut con,
            &png_key,
            config.tile_cache_ttl,
        )
        .await
    {
        return Ok(([(header::CONTENT_TYPE, "image/png")], cached));
    }

    // 1. Resolve the project by slug.
    let project_record = super::db::Entity::find()
        .filter(super::db::Column::Slug.eq(&slug))
        .one(db)
        .await
        .map_err(|e| {
            error!(error = %e, "Database query error");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or_else(|| {
            debug!(slug = %slug, "Project not found");
            StatusCode::NOT_FOUND
        })?;

    // 2. Resolve the layer by layer_name, scoped to this project.
    let layer_record = layer::Entity::find()
        .filter(layer::Column::LayerName.eq(&params.layer))
        .filter(layer::Column::ProjectId.eq(project_record.id))
        .one(db)
        .await
        .map_err(|e| {
            error!(error = %e, "Database query error");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or_else(|| {
            debug!(layer = %params.layer, slug = %slug, "Layer not found in project");
            StatusCode::NOT_FOUND
        })?;

    // 3. Fetch and decode the tile from S3.
    let xyz_tile = XYZTile { x, y, z };
    let project_id = layer_record.project_id;
    let retry_strategy = FixedInterval::from_millis(200).take(5);
    let img: ImageBuffer<image::Luma<f32>, Vec<f32>> = RetryIf::spawn(
        retry_strategy,
        || xyz_tile.get_one(config, project_id, &params.layer),
        |e: &anyhow::Error| {
            error!(layer = %params.layer, z, x, y, error = %e, "Tile generation failed");
            true
        },
    )
    .await
    .map_err(|e| {
        error!(layer = %params.layer, z, x, y, error = %e, "Failed to generate tile after retries");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // 4. Resolve which style to apply: project override wins, otherwise the
    //    layer's own style. Both are optional — if neither is set we fall
    //    back to no styling (consistent with the main tile handler).
    let style_id_to_use = project_record.card_style_id.or(layer_record.style_id);

    let (dbstyle, interpolation_type): (Option<JsonValue>, Option<String>) =
        if let Some(sid) = style_id_to_use {
            style::Entity::find_by_id(sid)
                .one(db)
                .await
                .map_err(|e| {
                    error!(error = %e, "Database query error");
                    StatusCode::INTERNAL_SERVER_ERROR
                })?
                .map(|s| (s.style, Some(s.interpolation_type)))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

    // 5. Apply the style and return the PNG.
    let png_data = crate::routes::tiles::styling::style_layer(
        img,
        dbstyle,
        interpolation_type.as_deref(),
    )
    .map_err(|e| {
        error!(error = %e, "Error applying style");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Best-effort cache write — splash cards repeatedly request the same
    // tiles, so this drops every subsequent request to a single Redis GET.
    let _ = crate::routes::tiles::cache::push_cache_raw(config, &png_key, &png_data).await;

    Ok(([(header::CONTENT_TYPE, "image/png")], png_data))
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

    let all_groups: std::collections::HashMap<uuid::Uuid, crate::routes::variable_groups::db::Model> =
        crate::routes::variable_groups::db::Entity::find()
            .all(db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
            .into_iter()
            .map(|g| (g.id, g))
            .collect();

    let mut variables = Vec::new();
    for junc in &var_junctions {
        if let Some(v) = crate::routes::variables::db::Entity::find_by_id(junc.variable_id)
            .one(db)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?
        {
            let (tier1_group, tier1_help_text, tier1_sort_order, group_name, group_help_text, group_sort_order) =
                if let Some(gid) = v.group_id {
                    if let Some(group) = all_groups.get(&gid) {
                        if let Some(pid) = group.parent_id {
                            let parent = all_groups.get(&pid);
                            (
                                parent.map(|p| p.name.as_str()),
                                parent.and_then(|p| p.help_text.as_deref()),
                                parent.map(|p| p.sort_order).unwrap_or(0),
                                Some(group.name.as_str()),
                                group.help_text.as_deref(),
                                group.sort_order,
                            )
                        } else {
                            (
                                Some(group.name.as_str()),
                                group.help_text.as_deref(),
                                group.sort_order,
                                None,
                                None,
                                0,
                            )
                        }
                    } else {
                        (None, None, 0, v.group_name.as_deref(), None, 0)
                    }
                } else {
                    (None, None, 0, v.group_name.as_deref(), None, 0)
                };

            variables.push(serde_json::json!({
                "id": v.id,
                "slug": v.slug,
                "name": v.name,
                "abbreviation": v.abbreviation,
                "subscript": v.subscript,
                "unit": v.unit,
                "is_crop_specific": v.is_crop_specific,
                "has_time": v.has_time,
                "group_name": group_name,
                "group_help_text": group_help_text,
                "group_sort_order": group_sort_order,
                "tier1_group": tier1_group,
                "tier1_help_text": tier1_help_text,
                "tier1_sort_order": tier1_sort_order,
                "sort_order": junc.sort_order,
                "enabled": true,
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
