use crudcrate::{CRUDResource, EntityToModels};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, EntityToModels, serde::Serialize, serde::Deserialize)]
#[sea_orm(table_name = "project")]
#[crudcrate(
    api_struct = "Project",
    name_singular = "project",
    name_plural = "projects",
    generate_router,
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    #[crudcrate(primary_key, filterable, exclude(update, create), on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[sea_orm(unique)]
    #[crudcrate(filterable, fulltext)]
    pub slug: String,
    #[crudcrate(filterable, fulltext)]
    pub title: String,
    pub description: Option<String>,
    #[sea_orm(column_type = "Double")]
    pub latitude: f64,
    #[sea_orm(column_type = "Double")]
    pub longitude: f64,
    pub zoom_level: i32,
    #[crudcrate(filterable)]
    pub enabled: bool,
    #[crudcrate(sortable)]
    pub sort_order: i32,
    /// Timeline config for the public UI year slider. Null means no slider.
    /// Shape: {"mode":"range","min":2000,"max":2090,"step":10}
    ///    or: {"mode":"list","values":[2020,2050,2090]}
    pub year_axis: Option<serde_json::Value>,
    /// When set, this year uses the "historical" scenario instead of the
    /// user-selected scenario. Null disables the override.
    pub historical_year: Option<i32>,
    pub tab_config: Option<serde_json::Value>,
    /// Layer rendered as an overlay on the splash card preview map.
    pub card_layer_id: Option<Uuid>,
    /// Optional style override applied to `card_layer_id` on the splash card
    /// preview. Falls back to the layer's own `style_id` when null.
    pub card_style_id: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
