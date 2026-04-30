pub use super::db::Style;
use crate::common::auth::Role;
use crate::common::state::AppState;
use crate::config::Config;
use axum::extract::Request;
use axum::middleware::{self, Next};
use axum_keycloak_auth::{PassthroughMode, layer::KeycloakAuthLayer};
use crudcrate::CRUDResource;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use tracing::{error, warn};
use utoipa_axum::router::OpenApiRouter;

pub fn router(state: &AppState) -> OpenApiRouter {
    let mut crud_router = Style::router(&state.db.clone());

    // Cache invalidation middleware: when a style is updated or deleted,
    // clear all rendered tiles that used it and re-warm affected globe/card tiles.
    let config = state.config.clone();
    let db = state.db.clone();
    crud_router = crud_router.layer(middleware::from_fn(move |req: Request, next: Next| {
        let config = config.clone();
        let db = db.clone();
        async move {
            let method = req.method().clone();
            let path = req.uri().path().to_string();
            let response = next.run(req).await;

            let is_mutating = matches!(
                method,
                axum::http::Method::PUT | axum::http::Method::DELETE
            );
            if response.status().is_success() && is_mutating {
                if let Some(style_id) = extract_uuid_from_path(&path) {
                    tokio::spawn(async move {
                        if let Err(e) =
                            on_style_changed(&config, &db, style_id).await
                        {
                            error!(error = %e, %style_id, "Style cache invalidation failed");
                        }
                    });
                }
            }

            response
        }
    }));

    if let Some(instance) = state.keycloak_auth_instance.clone() {
        crud_router = crud_router.layer(
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

    crud_router
}

fn extract_uuid_from_path(path: &str) -> Option<uuid::Uuid> {
    path.rsplit('/').find_map(|seg| uuid::Uuid::parse_str(seg).ok())
}

async fn on_style_changed(
    config: &Config,
    db: &DatabaseConnection,
    style_id: uuid::Uuid,
) -> anyhow::Result<()> {
    use crate::routes::tiles::cache;

    // 1. Invalidate XYZ tiles rendered with this style
    cache::invalidate_style_tiles(config, style_id).await?;

    // 2. Check if globe uses this style
    let settings = crate::routes::site_settings::db::Entity::find()
        .one(db)
        .await?;
    if let Some(ref s) = settings {
        if s.globe_style_id == Some(style_id) {
            cache::invalidate_globe_tiles(config).await?;
        }
    }

    // 3. Check if any project card uses this style
    let projects = crate::routes::projects::db::Entity::find()
        .filter(crate::routes::projects::db::Column::CardStyleId.eq(style_id))
        .all(db)
        .await?;
    for project in &projects {
        cache::invalidate_card_tiles(config, &project.slug).await?;
    }

    // 4. Warm affected tiles in the background
    crate::routes::tiles::warming::warm_after_style_change(config, db, style_id).await;

    Ok(())
}
