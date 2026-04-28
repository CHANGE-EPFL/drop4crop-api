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
use hyper::StatusCode;
use image::ImageBuffer;
use redis;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, JsonValue, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use tokio_retry::{RetryIf, strategy::FixedInterval};
use tracing::{debug, error, warn};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

const SINGLETON_ID: &str = "00000000-0000-0000-0000-000000000001";

fn singleton_uuid() -> Uuid {
    Uuid::parse_str(SINGLETON_ID).unwrap()
}

pub fn router(state: &AppState) -> OpenApiRouter {
    let public_router = OpenApiRouter::new()
        .routes(routes!(get_site_settings))
        .routes(routes!(get_site_settings_by_id))
        .routes(routes!(get_globe_tile))
        .with_state(state.clone());

    let mut protected_router = OpenApiRouter::new()
        .routes(routes!(update_site_settings))
        .routes(routes!(update_site_settings_by_id))
        .with_state(state.clone());

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
        warn!("site-settings mutating routes are not protected");
    }

    public_router.merge(protected_router)
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct SiteSettingsResponse {
    pub id: Uuid,
    pub globe_layer_id: Option<Uuid>,
    pub globe_style_id: Option<Uuid>,
    pub globe_layer_name: Option<String>,
}

async fn load_settings(
    db: &sea_orm::DatabaseConnection,
) -> Result<SiteSettingsResponse, (StatusCode, Json<String>)> {
    let row = super::db::Entity::find_by_id(singleton_uuid())
        .one(db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json("site_settings row missing".to_string()),
            )
        })?;

    let globe_layer_name = if let Some(lid) = row.globe_layer_id {
        layer::Entity::find_by_id(lid)
            .one(db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?
            .and_then(|l| l.layer_name)
    } else {
        None
    };

    Ok(SiteSettingsResponse {
        id: row.id,
        globe_layer_id: row.globe_layer_id,
        globe_style_id: row.globe_style_id,
        globe_layer_name,
    })
}

#[utoipa::path(
    get,
    path = "/config",
    responses(
        (status = 200, description = "Site settings", body = SiteSettingsResponse),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get site-wide settings",
)]
pub async fn get_site_settings(
    State(app_state): State<AppState>,
) -> Result<Json<SiteSettingsResponse>, (StatusCode, Json<String>)> {
    load_settings(&app_state.db).await.map(Json)
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateSiteSettings {
    pub globe_layer_id: Option<Uuid>,
    pub globe_style_id: Option<Uuid>,
}

#[utoipa::path(
    put,
    path = "/config",
    request_body = UpdateSiteSettings,
    responses(
        (status = 200, description = "Updated site settings", body = SiteSettingsResponse),
        (status = 500, description = "Internal server error")
    ),
    summary = "Update site-wide settings",
)]
pub async fn update_site_settings(
    State(app_state): State<AppState>,
    Json(body): Json<UpdateSiteSettings>,
) -> Result<Json<SiteSettingsResponse>, (StatusCode, Json<String>)> {
    let db = &app_state.db;

    let mut active: super::db::ActiveModel = super::db::Entity::find_by_id(singleton_uuid())
        .one(db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json("site_settings row missing".to_string()),
            )
        })?
        .into();

    active.globe_layer_id = Set(body.globe_layer_id);
    active.globe_style_id = Set(body.globe_style_id);

    active
        .update(db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?;

    load_settings(db).await.map(Json)
}

// react-admin expects GET/PUT /{id}; these delegate to the singleton handlers.

#[utoipa::path(get, path = "/{id}", params(("id" = Uuid, Path,)))]
pub async fn get_site_settings_by_id(
    Path(_id): Path<Uuid>,
    State(app_state): State<AppState>,
) -> Result<Json<SiteSettingsResponse>, (StatusCode, Json<String>)> {
    get_site_settings(State(app_state)).await
}

#[utoipa::path(put, path = "/{id}", params(("id" = Uuid, Path,)), request_body = UpdateSiteSettings)]
pub async fn update_site_settings_by_id(
    Path(_id): Path<Uuid>,
    State(app_state): State<AppState>,
    Json(body): Json<UpdateSiteSettings>,
) -> Result<Json<SiteSettingsResponse>, (StatusCode, Json<String>)> {
    update_site_settings(State(app_state), Json(body)).await
}

// ---------------------------------------------------------------------------
// GET /globe-tile/{z}/{x}/{y} - Render a tile for the splash page globe
// ---------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
pub struct GlobeTileParams {
    layer: String,
}

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
    path = "/globe-tile/{z}/{x}/{y}",
    params(
        ("z" = String, Path, description = "Zoom level"),
        ("x" = String, Path, description = "Tile x coordinate"),
        ("y" = String, Path, description = "Tile y coordinate"),
        ("layer" = String, Query, description = "Layer name")
    ),
    responses(
        (status = 200, description = "Tile image", body = [u8], content_type = "image/png"),
        (status = 404, description = "Layer or tile not found"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get a globe background tile",
    description = "Renders a tile using the site-wide `globe_style_id` if set, otherwise the layer's own style."
)]
#[axum::debug_handler]
pub async fn get_globe_tile(
    Path((z_str, x_str, y_str)): Path<(String, String, String)>,
    Query(params): Query<GlobeTileParams>,
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

    let png_key = crate::routes::tiles::cache::build_cache_key(
        config,
        &format!("png-globe/{}/{}/{}/{}", params.layer, z, x, y),
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

    let settings = super::db::Entity::find_by_id(singleton_uuid())
        .one(db)
        .await
        .map_err(|e| {
            error!(error = %e, "Database query error");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let layer_record = layer::Entity::find()
        .filter(layer::Column::LayerName.eq(&params.layer))
        .one(db)
        .await
        .map_err(|e| {
            error!(error = %e, "Database query error");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or_else(|| {
            debug!(layer = %params.layer, "Layer not found");
            StatusCode::NOT_FOUND
        })?;

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

    let style_id_to_use = settings
        .as_ref()
        .and_then(|s| s.globe_style_id)
        .or(layer_record.style_id);

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

    let png_data = crate::routes::tiles::styling::style_layer(
        img,
        dbstyle,
        interpolation_type.as_deref(),
    )
    .map_err(|e| {
        error!(error = %e, "Error applying style");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let _ = crate::routes::tiles::cache::push_cache_raw(config, &png_key, &png_data).await;

    Ok(([(header::CONTENT_TYPE, "image/png")], png_data))
}
