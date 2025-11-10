mod countries;
mod layers;
mod styles;
mod tiles;

use crate::{common::state::AppState, config::Config};
use axum::{Router, extract::DefaultBodyLimit, extract::Request, middleware::{self, Next}, response::Response};
use axum_keycloak_auth::{Url, instance::KeycloakAuthInstance, instance::KeycloakConfig};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable};
use axum_governor::GovernorLayer;
use real::{RealIpLayer, RealIp};
use tower::ServiceBuilder;

async fn log_request_ip(request: Request, next: Next) -> Response {
    // Extract the real IP from the request extensions (set by RealIpLayer)
    if let Some(real_ip) = request.extensions().get::<RealIp>() {
        println!("[{}] {} {}", real_ip.ip(), request.method(), request.uri());
    } else {
        println!("[unknown IP] {} {}", request.method(), request.uri());
    }
    next.run(request).await
}

pub fn build_router(db: &DatabaseConnection, config: &Config) -> Router {
    #[derive(OpenApi)]
    #[openapi(
        modifiers(&SecurityAddon),
        security(
            ("bearerAuth" = [])
        )
    )]
    struct ApiDoc;

    struct SecurityAddon;

    impl utoipa::Modify for SecurityAddon {
        fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
            if let Some(components) = openapi.components.as_mut() {
                components.add_security_scheme(
                    "bearerAuth",
                    utoipa::openapi::security::SecurityScheme::Http(
                        utoipa::openapi::security::HttpBuilder::new()
                            .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                            .bearer_format("JWT")
                            .build(),
                    ),
                );
            }
        }
    }

    let keycloak_instance: Option<Arc<KeycloakAuthInstance>> = if config.keycloak_url.is_empty() {
        // Skip Keycloak initialization for tests
        None
    } else {
        Some(Arc::new(KeycloakAuthInstance::new(
            KeycloakConfig::builder()
                .server(Url::parse(&config.keycloak_url).unwrap())
                .realm(String::from(&config.keycloak_realm))
                .build(),
        )))
    };

    let app_state: AppState = AppState::new(db.clone(), config.clone(), keycloak_instance);

    // Build rate-limited middleware stack
    // Middleware order (outer to inner):
    //   1. RealIpLayer - Extracts client IP and stores in request extensions
    //   2. log_request_ip - Logs IP, method, and URI for each request
    //   3. GovernorLayer - Applies rate limiting based on IP
    let rate_limit_stack = ServiceBuilder::new()
        .layer(RealIpLayer::default())
        .layer(middleware::from_fn(log_request_ip))
        .layer(GovernorLayer::default());

    // Build the router with routes from the plots module
    // Apply rate limiting to API routes, but NOT to health check endpoints
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest("/api/tiles", tiles::views::router(db))
        .nest("/api/countries", countries::views::router(&app_state))
        .nest("/api/layers", layers::views::router(&app_state))
        .nest("/api/styles", styles::views::router(&app_state))
        .layer(DefaultBodyLimit::max(250 * 1024 * 1024)) // 250MB to match Uppy configuration
        .layer(rate_limit_stack.clone()) // Apply rate limiting to API routes
        .split_for_parts();

    // Merge health check routes (NO rate limiting), STAC router (with rate limiting), and docs
    router
        .merge(crate::common::views::router(db)) // Health check routes - no rate limiting
        .nest("/api/stac", tiles::stac_router::router(db).layer(rate_limit_stack)) // STAC with rate limiting
        .merge(Scalar::with_url("/api/docs", api))
}
