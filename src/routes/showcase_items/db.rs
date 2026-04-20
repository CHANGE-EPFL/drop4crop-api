use crudcrate::{CRUDResource, EntityToModels};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, EntityToModels, serde::Serialize, serde::Deserialize)]
#[sea_orm(table_name = "showcase_item")]
#[crudcrate(
    api_struct = "ShowcaseItem",
    name_singular = "showcase item",
    name_plural = "showcase-items",
    generate_router,
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    #[crudcrate(primary_key, filterable, exclude(update, create), on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[crudcrate(filterable)]
    pub project_id: Uuid,
    #[crudcrate(filterable)]
    pub layer_id: Uuid,
    #[crudcrate(filterable, fulltext)]
    pub title: String,
    pub description: Option<String>,
    #[crudcrate(sortable)]
    pub sort_order: i32,
    #[crudcrate(filterable)]
    pub enabled: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "crate::routes::projects::db::Entity",
        from = "Column::ProjectId",
        to = "crate::routes::projects::db::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    Project,
    #[sea_orm(
        belongs_to = "crate::routes::layers::db::Entity",
        from = "Column::LayerId",
        to = "crate::routes::layers::db::Column::Id",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    Layer,
}

impl Related<crate::routes::projects::db::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Project.def()
    }
}

impl Related<crate::routes::layers::db::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Layer.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
