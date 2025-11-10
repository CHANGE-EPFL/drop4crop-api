use axum::{routing::get, Router};
use sea_orm::DatabaseConnection;

pub fn router(db: &DatabaseConnection) -> Router {
    Router::new()
        .route("/", get(super::stac::stac_root))
        .route("/conformance", get(super::stac::stac_conformance))
        .route("/collections", get(super::stac::stac_collections))
        .route("/collections/drop4crop-tiles", get(super::stac::stac_collection))
        .route("/collections/drop4crop-tiles/items", get(super::stac::stac_items))
        .route("/search", get(super::stac::stac_search))
        .with_state(db.clone())
}
