pub use super::db::{Style, router};
use crate::common::auth::Role;
use crate::common::state::AppState;
use axum_keycloak_auth::{PassthroughMode, layer::KeycloakAuthLayer};
use crudcrate::CRUDResource;
use utoipa_axum::router::OpenApiRouter;

// crud_handlers!(Style, StyleUpdate, StyleCreate);

pub fn router(state: &AppState) -> OpenApiRouter {
    let mut mutating_router = router(&state.db.clone());

    if let Some(instance) = state.keycloak_auth_instance.clone() {
        mutating_router = mutating_router.layer(
            KeycloakAuthLayer::<Role>::builder()
                .instance(instance)
                .passthrough_mode(PassthroughMode::Block)
                .persist_raw_claims(false)
                .expected_audiences(vec![String::from("account")])
                .required_roles(vec![Role::Administrator])
                .build(),
        );
    } else {
        println!(
            "Warning: Mutating routes of {} router are not protected",
            Style::RESOURCE_NAME_PLURAL
        );
    }

    mutating_router
}
