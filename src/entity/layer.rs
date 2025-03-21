// use sea_orm::entity::prelude::*;

// #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
// #[sea_orm(table_name = "layer")]
// pub struct Model {
//     #[sea_orm(unique)]
//     pub layer_name: Option<String>,
//     pub crop: Option<String>,
//     pub water_model: Option<String>,
//     pub climate_model: Option<String>,
//     pub scenario: Option<String>,
//     pub variable: Option<String>,
//     pub year: Option<i32>,
//     pub last_updated: DateTime,
//     #[sea_orm(primary_key)]
//     pub iterator: i32,
//     #[sea_orm(unique)]
//     pub id: Uuid,
//     pub enabled: bool,
//     pub uploaded_at: DateTime,
//     #[sea_orm(column_type = "Double", nullable)]
//     pub global_average: Option<f64>,
//     pub filename: Option<String>,
//     #[sea_orm(column_type = "Double", nullable)]
//     pub min_value: Option<f64>,
//     #[sea_orm(column_type = "Double", nullable)]
//     pub max_value: Option<f64>,
//     pub style_id: Option<Uuid>,
//     pub is_crop_specific: bool,
// }

// #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
// pub enum Relation {
//     #[sea_orm(has_many = "super::layercountrylink::Entity")]
//     Layercountrylink,
//     #[sea_orm(
//         belongs_to = "super::style::Entity",
//         from = "Column::StyleId",
//         to = "super::style::Column::Id",
//         on_update = "NoAction",
//         on_delete = "NoAction"
//     )]
//     Style,
// }

// impl Related<super::layercountrylink::Entity> for Entity {
//     fn to() -> RelationDef {
//         Relation::Layercountrylink.def()
//     }
// }

// impl Related<super::style::Entity> for Entity {
//     fn to() -> RelationDef {
//         Relation::Style.def()
//     }
// }

// impl Related<super::country::Entity> for Entity {
//     fn to() -> RelationDef {
//         super::layercountrylink::Relation::Country.def()
//     }
//     fn via() -> Option<RelationDef> {
//         Some(super::layercountrylink::Relation::Layer.def().rev())
//     }
// }

// impl ActiveModelBehavior for ActiveModel {}
