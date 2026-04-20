use crudcrate::{CRUDResource, EntityToModels};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, EntityToModels, serde::Serialize, serde::Deserialize)]
#[sea_orm(table_name = "climate_model")]
#[crudcrate(
    api_struct = "ClimateModel",
    name_singular = "climate model",
    name_plural = "climate-models",
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
    pub name: String,
    #[crudcrate(sortable)]
    pub sort_order: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
