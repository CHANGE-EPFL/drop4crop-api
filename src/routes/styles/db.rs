use chrono::{DateTime, Utc};
use crudcrate::EntityToModels;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, EntityToModels)]
#[sea_orm(table_name = "style")]
#[crudcrate(generate_router)]
pub struct Model {
    #[sea_orm(unique)]
    pub name: String,
    pub last_updated: DateTime<Utc>,
    #[sea_orm(primary_key)]
    // pub iterator: i32,
    // #[sea_orm(unique)]
    pub id: Uuid,
    pub style: Option<Json>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "crate::routes::layers::db::Entity")]
    Layer,
}

impl Related<crate::routes::layers::db::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Layer.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
