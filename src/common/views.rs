use super::models::HealthCheck;
use crate::config::Config;
use axum::{Json, extract::State, http::StatusCode};
use sea_orm::DatabaseConnection;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

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
        println!(
            "[{} | {:15} | healthz | 500] Database connection FAILED",
            now.format("%Y-%m-%d %H:%M:%S"),
            "system"
        );
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(HealthCheck {
                status: "error".to_string(),
            }),
        );
    }
    println!(
        "[{} | {:15} | healthz | 200] Database connection is healthy",
        now.format("%Y-%m-%d %H:%M:%S"),
        "system"
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
    let config = Config::from_env();
    let keycloak_config = KeycloakConfig {
        client_id: config.keycloak_client_id,
        realm: config.keycloak_realm,
        url: config.keycloak_url,
    };

    (StatusCode::OK, Json(keycloak_config))
}
