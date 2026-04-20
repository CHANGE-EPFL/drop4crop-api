use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, serde::Serialize, serde::Deserialize)]
#[sea_orm(table_name = "project_water_model")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub project_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub water_model_id: Uuid,
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
        belongs_to = "crate::routes::water_models::db::Entity",
        from = "Column::WaterModelId",
        to = "crate::routes::water_models::db::Column::Id"
    )]
    WaterModel,
}

impl ActiveModelBehavior for ActiveModel {}
