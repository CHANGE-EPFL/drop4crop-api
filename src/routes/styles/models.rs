use super::db::Model;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use crudcrate::{CRUDResource, ToCreateModel, ToUpdateModel};
use rand::Rng;
use sea_orm::{
    ActiveValue, Condition, DatabaseConnection, EntityTrait, FromQueryResult, Order, QueryOrder,
    QuerySelect, entity::prelude::*,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// #[derive(
//     ToSchema, Serialize, Deserialize, FromQueryResult, ToUpdateModel, ToCreateModel, Clone,
// )]
// #[active_model = "super::db::ActiveModel"]
// pub struct Project {
//     #[crudcrate(on_create = generate_random_color())]
//     color: String,
//     #[crudcrate(
//         create_model = false,
//         update_model = false,
//         on_create = chrono::Utc::now(),
//         on_update = chrono::Utc::now()
//     )]
//     last_updated: DateTime<Utc>,
//     description: Option<String>,
//     #[crudcrate(update_model = false, update_model = false, on_create = Uuid::new_v4())]
//     id: Uuid,
//     name: String,
// }

// impl From<Model> for Project {
//     fn from(model: Model) -> Self {
//         Self {
//             color: model.color,
//             last_updated: model.last_updated,
//             description: model.description,
//             id: model.id,
//             name: model.name,
//         }
//     }
// }

// #[async_trait]
// impl CRUDResource for Project {
//     type EntityType = super::db::Entity;
//     type ColumnType = super::db::Column;
//     type ModelType = super::db::Model;
//     type ActiveModelType = super::db::ActiveModel;
//     type ApiModel = Project;
//     type CreateModel = ProjectCreate;
//     type UpdateModel = ProjectUpdate;

//     const ID_COLUMN: Self::ColumnType = super::db::Column::Id;
//     const RESOURCE_NAME_PLURAL: &'static str = "projects";
//     const RESOURCE_NAME_SINGULAR: &'static str = "project";
//     const RESOURCE_DESCRIPTION: &'static str = "This resource allows the data hierarchically beneath each area to be allocated to a specific project. This is useful for grouping data together for analysis. The colour provides a visual representation of the project in the UI.";

//     async fn get_all(
//         db: &DatabaseConnection,
//         condition: Condition,
//         order_column: Self::ColumnType,
//         order_direction: Order,
//         offset: u64,
//         limit: u64,
//     ) -> Result<Vec<Self::ApiModel>, DbErr> {
//         let models = Self::EntityType::find()
//             .filter(condition)
//             .order_by(order_column, order_direction)
//             .offset(offset)
//             .limit(limit)
//             .all(db)
//             .await?;
//         Ok(models.into_iter().map(Self::ApiModel::from).collect())
//     }

//     async fn get_one(db: &DatabaseConnection, id: Uuid) -> Result<Self::ApiModel, DbErr> {
//         let model =
//             Self::EntityType::find_by_id(id)
//                 .one(db)
//                 .await?
//                 .ok_or(DbErr::RecordNotFound(format!(
//                     "{} not found",
//                     Self::RESOURCE_NAME_SINGULAR
//                 )))?;
//         Ok(Self::ApiModel::from(model))
//     }

//     async fn update(
//         db: &DatabaseConnection,
//         id: Uuid,
//         update_data: Self::UpdateModel,
//     ) -> Result<Self::ApiModel, DbErr> {
//         let existing: Self::ActiveModelType = Self::EntityType::find_by_id(id)
//             .one(db)
//             .await?
//             .ok_or(DbErr::RecordNotFound(format!(
//                 "{} not found",
//                 Self::RESOURCE_NAME_PLURAL
//             )))?
//             .into();

//         let updated_model = update_data.merge_into_activemodel(existing);
//         let updated = updated_model.update(db).await?;
//         Ok(Self::ApiModel::from(updated))
//     }

//     fn sortable_columns() -> Vec<(&'static str, Self::ColumnType)> {
//         vec![
//             ("id", Self::ColumnType::Id),
//             ("name", Self::ColumnType::Name),
//             ("description", Self::ColumnType::Description),
//             ("color", Self::ColumnType::Color),
//             ("last_updated", Self::ColumnType::LastUpdated),
//         ]
//     }

//     fn filterable_columns() -> Vec<(&'static str, Self::ColumnType)> {
//         vec![
//             ("name", Self::ColumnType::Name),
//             ("description", Self::ColumnType::Description),
//             ("color", Self::ColumnType::Color),
//         ]
//     }
// }

// fn generate_random_color() -> String {
//     let mut rng = rand::rng();
//     format!("#{:06x}", rng.random::<u32>() & 0xFF_FFFF)
// }

#[derive(Serialize, Deserialize, Clone, ToSchema)]
pub struct StyleItem {
    pub value: f64,
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub opacity: u8,
    pub label: f64,
}
