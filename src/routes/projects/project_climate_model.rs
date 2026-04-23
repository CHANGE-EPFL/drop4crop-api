use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, serde::Serialize, serde::Deserialize)]
#[sea_orm(table_name = "project_climate_model")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub project_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub climate_model_id: Uuid,
    pub sort_order: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::db::Entity",
        from = "Column::ProjectId",
        to = "super::db::Column::Id"
    )]
    Project,
    #[sea_orm(
        belongs_to = "crate::routes::climate_models::db::Entity",
        from = "Column::ClimateModelId",
        to = "crate::routes::climate_models::db::Column::Id"
    )]
    ClimateModel,
}

impl ActiveModelBehavior for ActiveModel {}
