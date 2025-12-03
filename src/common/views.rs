use super::models::HealthCheck;
use super::state::AppState;
use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use tracing::{info, error};

#[derive(Serialize, ToSchema)]
pub struct KeycloakConfig {
    #[serde(rename = "clientId")]
    client_id: String,
    realm: String,
    url: String,
}

pub fn router(state: &AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(healthz))
        .routes(routes!(get_keycloak_config))
        .with_state(state.clone())
}

#[utoipa::path(
    get,
    path = "/healthz",
    responses(
        (
            status = OK,
            description = "Kubernetes health check",
            body = str,
            content_type = "text/plain"
        )
    )
)]
pub async fn healthz(State(app_state): State<AppState>) -> (StatusCode, Json<HealthCheck>) {
    let db = &app_state.db;
    let now = chrono::Utc::now();
    if db.ping().await.is_err() {
        error!(
            timestamp = %now.format("%Y-%m-%d %H:%M:%S"),
            endpoint = "healthz",
            status = 500,
            "Database connection FAILED"
        );
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(HealthCheck {
                status: "error".to_string(),
            }),
        );
    }
    info!(
        timestamp = %now.format("%Y-%m-%d %H:%M:%S"),
        endpoint = "healthz",
        status = 200,
        "Database connection is healthy"
    );
    (
        StatusCode::OK,
        Json(HealthCheck {
            status: "ok".to_string(),
        }),
    )
}

#[utoipa::path(
    get,
    path = "/api/config/keycloak",
    responses(
        (
            status = OK,
            description = "Get Keycloak configuration",
            body = KeycloakConfig,
            content_type = "application/json"
        )
    )
)]
pub async fn get_keycloak_config(
    State(app_state): State<AppState>,
) -> (StatusCode, Json<KeycloakConfig>) {
    let config = &app_state.config;
    let keycloak_config = KeycloakConfig {
        client_id: config.keycloak_client_id.clone(),
        realm: config.keycloak_realm.clone(),
        url: config.keycloak_url.clone(),
    };

    (StatusCode::OK, Json(keycloak_config))
}
