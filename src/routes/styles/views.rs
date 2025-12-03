pub use super::db::Style;
use super::db::{self as style, ActiveModel};
use super::utils::{parse_qgis_colormap, export_to_qgis, QgisImportRequest};
use crate::common::auth::Role;
use crate::common::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use axum_keycloak_auth::{PassthroughMode, layer::KeycloakAuthLayer};
use crudcrate::CRUDResource;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use tracing::{error, warn};
use uuid::Uuid;

/// Response for QGIS import
#[derive(Debug, Serialize, ToSchema)]
pub struct ImportResponse {
    pub id: Uuid,
    pub name: String,
    pub interpolation_type: String,
    pub stop_count: usize,
}

/// Request body for QGIS import
#[derive(Debug, Deserialize, ToSchema)]
pub struct ImportRequest {
    /// Name for the new style
    pub name: String,
    /// Raw QGIS color map content
    pub qgis_content: String,
}

/// Response for QGIS export
#[derive(Debug, Serialize, ToSchema)]
pub struct ExportResponse {
    pub qgis_content: String,
}

/// Preview response for QGIS import (without saving)
#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewResponse {
    pub stops: serde_json::Value,
    pub interpolation_type: String,
    pub stop_count: usize,
}

pub fn router(state: &AppState) -> OpenApiRouter {
    let crud_router = Style::router(&state.db.clone());

    // Custom routes for QGIS import/export
    let custom_router = OpenApiRouter::new()
        .routes(routes!(import_qgis_style))
        .routes(routes!(preview_qgis_style))
        .routes(routes!(export_qgis_style))
        .with_state(state.db.clone());

    let mut combined_router = crud_router.merge(custom_router);

    if let Some(instance) = state.keycloak_auth_instance.clone() {
        combined_router = combined_router.layer(
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
            resource = Style::RESOURCE_NAME_PLURAL,
            "Mutating routes are not protected"
        );
    }

    combined_router
}

/// Import a QGIS color map file and create a new style
#[utoipa::path(
    post,
    path = "/import/qgis",
    request_body = ImportRequest,
    responses(
        (status = 201, description = "Style created successfully", body = ImportResponse),
        (status = 400, description = "Invalid QGIS content"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Import QGIS color map",
    description = "Parses a QGIS color map export file and creates a new style with the parsed color stops."
)]
pub async fn import_qgis_style(
    State(db): State<DatabaseConnection>,
    Json(request): Json<ImportRequest>,
) -> Result<(StatusCode, Json<ImportResponse>), StatusCode> {
    // Parse the QGIS content
    let (stops, interpolation_type) = parse_qgis_colormap(&request.qgis_content)
        .map_err(|e| {
            error!(error = %e, "Failed to parse QGIS color map");
            StatusCode::BAD_REQUEST
        })?;

    let stop_count = stops.len();

    // Convert stops to JSON
    let style_json = serde_json::to_value(&stops)
        .map_err(|e| {
            error!(error = %e, "Failed to serialize color stops");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Create new style record
    let new_style = ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set(request.name.clone()),
        style: Set(Some(style_json)),
        interpolation_type: Set(interpolation_type.clone()),
        ..Default::default()
    };

    let result = new_style.insert(&db).await.map_err(|e| {
        error!(error = %e, "Failed to insert style");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok((
        StatusCode::CREATED,
        Json(ImportResponse {
            id: result.id,
            name: result.name,
            interpolation_type,
            stop_count,
        }),
    ))
}

/// Preview QGIS color map parsing without saving
#[utoipa::path(
    post,
    path = "/preview/qgis",
    request_body = ImportRequest,
    responses(
        (status = 200, description = "Preview of parsed color stops", body = PreviewResponse),
        (status = 400, description = "Invalid QGIS content"),
    ),
    summary = "Preview QGIS color map",
    description = "Parses a QGIS color map export file and returns the parsed color stops without saving."
)]
pub async fn preview_qgis_style(
    Json(request): Json<ImportRequest>,
) -> Result<Json<PreviewResponse>, StatusCode> {
    // Parse the QGIS content
    let (stops, interpolation_type) = parse_qgis_colormap(&request.qgis_content)
        .map_err(|e| {
            error!(error = %e, "Failed to parse QGIS color map");
            StatusCode::BAD_REQUEST
        })?;

    let stop_count = stops.len();

    // Convert stops to JSON
    let stops_json = serde_json::to_value(&stops)
        .map_err(|e| {
            error!(error = %e, "Failed to serialize color stops");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(PreviewResponse {
        stops: stops_json,
        interpolation_type,
        stop_count,
    }))
}

/// Export a style to QGIS color map format
#[utoipa::path(
    get,
    path = "/{id}/export/qgis",
    params(
        ("id" = Uuid, Path, description = "Style ID")
    ),
    responses(
        (status = 200, description = "QGIS color map content", body = ExportResponse),
        (status = 404, description = "Style not found"),
        (status = 500, description = "Internal server error")
    ),
    summary = "Export style to QGIS format",
    description = "Exports a style to QGIS color map format that can be imported into QGIS."
)]
pub async fn export_qgis_style(
    State(db): State<DatabaseConnection>,
    Path(id): Path<Uuid>,
) -> Result<Json<ExportResponse>, StatusCode> {
    // Find the style
    let style_record = style::Entity::find_by_id(id)
        .one(&db)
        .await
        .map_err(|e| {
            error!(error = %e, "Database query error");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Parse the style JSON to ColorStop array
    let stops: Vec<crate::routes::tiles::styling::ColorStop> = style_record
        .style
        .as_ref()
        .and_then(|s| serde_json::from_value(s.clone()).ok())
        .unwrap_or_default();

    // Export to QGIS format
    let qgis_content = export_to_qgis(&stops, &style_record.interpolation_type);

    Ok(Json(ExportResponse { qgis_content }))
}
