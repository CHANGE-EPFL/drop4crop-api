use axum::{routing::get, Router};
use crate::common::state::AppState;

pub fn router(state: &AppState) -> Router {
    Router::new()
        .route("/", get(super::stac::stac_root))
        .route("/conformance", get(super::stac::stac_conformance))
        .route("/collections", get(super::stac::stac_collections))
        .route("/collections/{collection_id}", get(super::stac::stac_collection))
        .route("/collections/{collection_id}/items", get(super::stac::stac_items))
        .route("/collections/{collection_id}/items/{item_id}", get(super::stac::stac_item))
        .route("/search", get(super::stac::stac_search))
        .with_state(state.clone())
}
