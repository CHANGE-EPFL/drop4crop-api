use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, serde::Serialize, serde::Deserialize)]
#[sea_orm(table_name = "project_scenario")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub project_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub scenario_id: Uuid,
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
        belongs_to = "crate::routes::scenarios::db::Entity",
        from = "Column::ScenarioId",
        to = "crate::routes::scenarios::db::Column::Id"
    )]
    Scenario,
}

impl ActiveModelBehavior for ActiveModel {}
