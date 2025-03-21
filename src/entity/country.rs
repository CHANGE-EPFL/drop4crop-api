// use sea_orm::entity::prelude::*;

// #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
// #[sea_orm(table_name = "country")]
// pub struct Model {
//     #[sea_orm(primary_key)]
//     pub iterator: i32,
//     #[sea_orm(unique)]
//     pub id: Uuid,
//     #[sea_orm(unique)]
//     pub name: String,
//     pub iso_a2: String,
//     pub iso_a3: String,
//     pub iso_n3: i32,
//     #[sea_orm(column_type = "custom(\"geometry\")", nullable)]
//     pub geom: Option<String>,
// }

// #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
// pub enum Relation {
//     #[sea_orm(has_many = "super::layercountrylink::Entity")]
//     Layercountrylink,
// }

// impl Related<super::layercountrylink::Entity> for Entity {
//     fn to() -> RelationDef {
//         Relation::Layercountrylink.def()
//     }
// }

// impl Related<super::layer::Entity> for Entity {
//     fn to() -> RelationDef {
//         super::layercountrylink::Relation::Layer.def()
//     }
//     fn via() -> Option<RelationDef> {
//         Some(super::layercountrylink::Relation::Country.def().rev())
//     }
// }

// impl ActiveModelBehavior for ActiveModel {}
