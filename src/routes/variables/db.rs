use crudcrate::{CRUDResource, EntityToModels};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, EntityToModels, serde::Serialize, serde::Deserialize)]
#[sea_orm(table_name = "variable")]
#[crudcrate(
    api_struct = "Variable",
    name_singular = "variable",
    name_plural = "variables",
    generate_router,
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    #[crudcrate(primary_key, exclude(update, create), on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[sea_orm(unique)]
    #[crudcrate(filterable, fulltext)]
    pub slug: String,
    #[crudcrate(filterable, fulltext)]
    pub name: String,
    pub abbreviation: String,
    pub subscript: Option<String>,
    pub unit: String,
    #[crudcrate(filterable)]
    pub is_crop_specific: bool,
    /// Whether this variable varies over time. Controls the year slider in
    /// the public UI. Default true for time-series (climate) variables; false
    /// for crop-specific single-snapshot variables.
    #[crudcrate(filterable, default_value = "true")]
    pub has_time: bool,
    #[crudcrate(filterable)]
    pub group_name: Option<String>,
    #[crudcrate(sortable)]
    pub sort_order: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
