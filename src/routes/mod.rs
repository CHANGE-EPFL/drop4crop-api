mod countries;
mod layers;
mod styles;
mod tiles;

use crate::{common::state::AppState, config::Config};
use axum::{Router, extract::DefaultBodyLimit};
use axum_keycloak_auth::{Url, instance::KeycloakAuthInstance, instance::KeycloakConfig};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable};

pub fn build_router(db: &DatabaseConnection) -> Router {
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

    let config: Config = Config::from_env();

    println!(
        "Keycloak config: url={}, realm={}",
        config.keycloak_url, config.keycloak_realm
    );

    // For development, disable authentication to avoid SSL certificate issues
    let keycloak_instance: Option<Arc<KeycloakAuthInstance>> = if config.deployment == "dev" {
        println!("Development mode: authentication disabled");
        None
    } else {
        let instance = Arc::new(KeycloakAuthInstance::new(
            KeycloakConfig::builder()
                .server(Url::parse(&config.keycloak_url).unwrap())
                .realm(String::from(&config.keycloak_realm))
                .build(),
        ));
        println!("Keycloak instance created successfully");
        Some(instance)
    };
    let app_state: AppState = AppState::new(db.clone(), config.clone(), keycloak_instance);

    // Build the router with routes from the plots module
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(crate::common::views::router(db)) // Root routes
        .nest("/api/tiles", tiles::views::router(db))
        .nest(
            "/api/countries",
            countries::views::router(db, keycloak_instance.clone()),
        )
        .nest(
            "/api/layers",
            layers::views::router(db, keycloak_instance.clone()),
        )
        .nest("/api/styles", styles::views::router(&app_state))
        // .nest(
        //     "/api/plot_samples",
        //     samples::views::router(db, Some(keycloak_instance.clone())),
        // )
        // .nest(
        //     "/api/sensors",
        //     sensors::views::router(db, Some(keycloak_instance.clone())),
        // )
        // .nest(
        //     "/api/sensor_profiles",
        //     sensors::profile::views::router(db, Some(keycloak_instance.clone())),
        // )
        // .nest(
        //     "/api/sensor_profile_assignments",
        //     sensors::profile::assignment::views::router(db, Some(keycloak_instance.clone())),
        // )
        // .nest(
        //     "/api/transects",
        //     transects::views::router(db, Some(keycloak_instance.clone())),
        // )
        // .nest(
        //     "/api/instruments",
        //     instrument_experiments::views::router(db, Some(keycloak_instance.clone())),
        // )
        // .nest(
        //     "/api/instrument_channels",
        //     instrument_experiments::channels::views::router(db, Some(keycloak_instance.clone())),
        // )
        // .nest(
        //     "/api/soil_types",
        //     soil::types::views::router(db, Some(keycloak_instance.clone())),
        // )
        // .nest(
        //     "/api/soil_profiles",
        //     soil::profiles::views::router(db, Some(keycloak_instance.clone())),
        // )
        .layer(DefaultBodyLimit::max(30 * 1024 * 1024))
        .split_for_parts();

    router.merge(Scalar::with_url("/api/docs", api))
}
