use super::models::HealthCheck;
use axum::{Json, extract::State, http::StatusCode};
use sea_orm::DatabaseConnection;
use utoipa_axum::{router::OpenApiRouter, routes};

pub fn router(db: &DatabaseConnection) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(healthz))
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
    if db.ping().await.is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(HealthCheck {
                status: "error".to_string(),
            }),
        );
    }

    (
        StatusCode::OK,
        Json(HealthCheck {
            status: "ok".to_string(),
        }),
    )
}
