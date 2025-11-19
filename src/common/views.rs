use super::models::HealthCheck;
use crate::config::Config;
use axum::{Json, extract::State, http::StatusCode};
use sea_orm::DatabaseConnection;
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
pub fn router(db: &DatabaseConnection) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(healthz))
        .routes(routes!(get_keycloak_config))
        .with_state(db.clone())
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
pub async fn healthz(State(db): State<DatabaseConnection>) -> (StatusCode, Json<HealthCheck>) {
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
pub async fn get_keycloak_config() -> (StatusCode, Json<KeycloakConfig>) {
    // Note: This is a public endpoint that needs config for the keycloak info
    // It's acceptable to call Config::from_env() here since this endpoint is specifically
    // about returning configuration to the client
    let config = Config::from_env();
    let keycloak_config = KeycloakConfig {
        client_id: config.keycloak_client_id,
        realm: config.keycloak_realm,
        url: config.keycloak_url,
    };

    (StatusCode::OK, Json(keycloak_config))
}
