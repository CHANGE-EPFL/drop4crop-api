use crate::common::state::AppState;
use crate::routes::layers::db as layer;
use crate::routes::projects::db as project;
use crate::routes::tiles::stac::{get_base_url, project_extent_to_bbox};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{json, Value};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Zoom range the tile handler serves. The public map requests layer tiles to
/// zoom 20 (`drop4crop-ui/src/components/Map/MapView.jsx:177`).
const MIN_ZOOM: u32 = 0;
const MAX_ZOOM: u32 = 20;

const WORLD_BBOX: [f64; 4] = [-180.0, -90.0, 180.0, 90.0];

pub fn router(state: &AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(get_tilejson))
        .with_state(state.clone())
}

/// The TileJSON document for a layer, as the STAC item's `tilejson` asset cites it.
pub fn tilejson_href(base_url: &str, layer_name: &str) -> String {
    format!("{}/api/layers/tilejson/{}.json", base_url, layer_name)
}

/// TileJSON 3.0.0 document for one layer's XYZ tiles.
pub fn build_tilejson(layer_name: &str, bounds: [f64; 4], base_url: &str) -> Value {
    json!({
        "tilejson": "3.0.0",
        "name": layer_name,
        "scheme": "xyz",
        "tiles": [format!("{}/api/layers/xyz/{{z}}/{{x}}/{{y}}?layer={}", base_url, layer_name)],
        "bounds": bounds,
        "minzoom": MIN_ZOOM,
        "maxzoom": MAX_ZOOM,
    })
}

/// Path format: /api/layers/tilejson/{filename} (e.g. /api/layers/tilejson/barley_cwatm_gfdl-esm2m_historical_wfb_2000.json)
#[utoipa::path(
    get,
    path = "/{filename}",
    params(
        ("filename" = String, Path, description = "Layer name with .json extension"),
    ),
    responses(
        (status = 200, description = "TileJSON document", body = Value),
        (status = 404, description = "Layer not found"),
    ),
    summary = "TileJSON for a layer",
    description = "Returns a TileJSON 3.0.0 document holding the XYZ tile template for the layer."
)]
pub async fn get_tilejson(
    State(app_state): State<AppState>,
    headers: HeaderMap,
    Path(filename): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let layer_name = filename.trim_end_matches(".json");
    let db = &app_state.db;

    let layer_record = layer::Entity::find()
        .filter(layer::Column::LayerName.eq(layer_name))
        .filter(layer::Column::Enabled.eq(true))
        .one(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let bounds = match layer_record.project_id {
        Some(project_id) => project::Entity::find_by_id(project_id)
            .one(db)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .and_then(|p| p.extent.as_ref().and_then(project_extent_to_bbox))
            .unwrap_or(WORLD_BBOX),
        None => WORLD_BBOX,
    };

    Ok(Json(build_tilejson(layer_name, bounds, &get_base_url(&headers))))
}

#[cfg(test)]
#[path = "tests/tilejson.rs"]
mod tests;
