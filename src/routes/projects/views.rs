pub use super::db::Project;
use crate::common::auth::Role;
use crate::common::state::AppState;
use axum::Json;
use axum::extract::State;
use axum_keycloak_auth::{PassthroughMode, layer::KeycloakAuthLayer};
use crudcrate::CRUDResource;
use hyper::StatusCode;
use sea_orm::{EntityTrait, QueryOrder};
use tracing::warn;
use utoipa_axum::{router::OpenApiRouter, routes};

pub fn router(state: &AppState) -> OpenApiRouter {
    let public_router = OpenApiRouter::new()
        .routes(routes!(get_active_projects))
        .with_state(state.clone());

    let mut crud_router = Project::router(&state.db.clone());

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
            resource = Project::RESOURCE_NAME_PLURAL,
            "Mutating routes are not protected"
        );
    }

    public_router.merge(crud_router)
}

#[utoipa::path(
    get,
    path = "/active",
    responses(
        (status = 200, description = "List of all projects ordered by sort_order", body = Vec<super::db::Project>),
        (status = 500, description = "Internal server error")
    ),
    summary = "Get all projects for splash page",
    description = "Returns all projects ordered by sort_order. Both enabled and disabled projects are returned so the UI can show 'Coming Soon' cards."
)]
pub async fn get_active_projects(
    State(app_state): State<AppState>,
) -> Result<Json<Vec<super::db::Project>>, (StatusCode, Json<String>)> {
    let db = &app_state.db;

    let projects = super::db::Entity::find()
        .order_by_asc(super::db::Column::SortOrder)
        .all(db)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, Json(err.to_string())))?;

    // Convert Sea-ORM models to API structs
    let projects: Vec<super::db::Project> = projects.into_iter().map(|m| m.into()).collect();

    Ok(Json(projects))
}
