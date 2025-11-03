mod countries;
mod layers;
mod styles;
mod tiles;

use crate::{common::state::AppState, config::Config};
use axum::{Router, extract::DefaultBodyLimit};
use axum_keycloak_auth::{Url, instance::KeycloakAuthInstance, instance::KeycloakConfig};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable};

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

    // Build the router with routes from the plots module
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(crate::common::views::router(db)) // Root routes
        .nest("/api/tiles", tiles::views::router(db))
        .nest("/api/countries", countries::views::router(&app_state))
        .nest("/api/layers", layers::views::router(&app_state))
        .nest("/api/styles", styles::views::router(&app_state))
        .layer(DefaultBodyLimit::max(250 * 1024 * 1024)) // 250MB to match Uppy configuration
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .split_for_parts();

    router.merge(Scalar::with_url("/api/docs", api))
}
