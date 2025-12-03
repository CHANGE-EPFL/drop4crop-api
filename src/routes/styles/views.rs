pub use super::db::Style;
use crate::common::auth::Role;
use crate::common::state::AppState;
use axum_keycloak_auth::{PassthroughMode, layer::KeycloakAuthLayer};
use crudcrate::CRUDResource;
use utoipa_axum::router::OpenApiRouter;
use tracing::warn;

pub fn router(state: &AppState) -> OpenApiRouter {
    let mut crud_router = Style::router(&state.db.clone());

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
