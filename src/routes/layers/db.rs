use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "layer")]
pub struct Model {
    #[sea_orm(unique)]
    pub layer_name: Option<String>,
    pub crop: Option<String>,
    pub water_model: Option<String>,
    pub climate_model: Option<String>,
    pub scenario: Option<String>,
    pub variable: Option<String>,
    pub year: Option<i32>,
    pub last_updated: DateTime<Utc>,
    #[sea_orm(primary_key)]
    // pub iterator: i32,
    // #[sea_orm(unique)]
    pub id: Uuid,
    pub enabled: bool,
    pub uploaded_at: DateTime<Utc>,
    #[sea_orm(column_type = "Double", nullable)]
    pub global_average: Option<f64>,
    pub filename: Option<String>,
    #[sea_orm(column_type = "Double", nullable)]
    pub min_value: Option<f64>,
    #[sea_orm(column_type = "Double", nullable)]
    pub max_value: Option<f64>,
    pub style_id: Option<Uuid>,
    pub is_crop_specific: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::countries::db::Entity")]
    Layercountrylink,
    #[sea_orm(
        belongs_to = "crate::routes::styles::db::Entity",
        from = "Column::StyleId",
        to = "crate::routes::styles::db::Column::Id",
        on_update = "NoAction",
        on_delete = "NoAction"
    )]
    Style,
}

impl Related<super::countries::db::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Layercountrylink.def()
    }
}

impl Related<crate::routes::styles::db::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Style.def()
    }
}

impl Related<crate::routes::countries::db::Entity> for Entity {
    fn to() -> RelationDef {
        super::countries::db::Relation::Country.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::countries::db::Relation::Layer.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
