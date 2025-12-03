// Test-specific router builder without rate limiting

use axum::Router;
use drop4crop_api::config::Config;
use sea_orm::DatabaseConnection;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

/// Build router for testing (without rate limiting middleware)
pub fn build_test_router(db: &DatabaseConnection, config: &Config) -> Router {
    #[derive(OpenApi)]
    struct ApiDoc;

    let app_state = drop4crop_api::common::state::AppState::new(
        db.clone(),
        config.clone(),
        None, // No Keycloak in tests
    );

    // Build router WITHOUT rate limiting for tests
    let (router, _api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest("/api/statistics", drop4crop_api::routes::admin::views::stats_router(&app_state))
        .nest("/api/cache", drop4crop_api::routes::admin::views::cache_router(&app_state))
        .nest("/api/layers", drop4crop_api::routes::layers::views::router(&app_state))
        .nest("/api/layers/xyz", drop4crop_api::routes::tiles::views::xyz_router(db))
        .nest("/api/styles", drop4crop_api::routes::styles::views::router(&app_state))
        .split_for_parts();

    // Merge health check routes
    router.merge(drop4crop_api::common::views::router(db))
}
