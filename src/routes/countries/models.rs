use super::db::Model;
use async_trait::async_trait;
use crudcrate::{CRUDResource, ToCreateModel, ToUpdateModel};
use sea_orm::{
    ActiveValue, Condition, DatabaseConnection, EntityTrait, Order, QueryOrder, QuerySelect,
    entity::prelude::*,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Serialize, ToUpdateModel, ToCreateModel, Deserialize, Clone, ToSchema)]
#[active_model = "super::db::ActiveModel"]
pub struct Country {
    #[crudcrate(update_model = false, create_model = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    pub name: String,
    pub iso_a2: String,
    pub iso_a3: String,
    pub iso_n3: i32,
    // #[sea_orm(column_type = "custom(\"geometry\")", nullable)]
    // pub geom: Option<String>,
}

impl From<Model> for Country {
    fn from(model: Model) -> Self {
        Self {
            id: model.id,
            name: model.name,
            iso_a2: model.iso_a2,
            iso_a3: model.iso_a3,
            iso_n3: model.iso_n3,
            // geom: model.geom,
        }
    }
}
#[async_trait]
impl CRUDResource for Country {
    type EntityType = super::db::Entity;
    type ColumnType = super::db::Column;
    type ModelType = super::db::Model;
    type ActiveModelType = super::db::ActiveModel;
    type ApiModel = Country;
    type CreateModel = CountryCreate;
    type UpdateModel = CountryUpdate;

    const ID_COLUMN: Self::ColumnType = super::db::Column::Id;
    const RESOURCE_NAME_PLURAL: &'static str = "countries";
    const RESOURCE_NAME_SINGULAR: &'static str = "country";
    const RESOURCE_DESCRIPTION: &'static str = "This resource represents a country. It includes the country name, ISO codes, and geometry data. The geometry data is used for mapping and visualization purposes.";

    async fn get_all(
        db: &DatabaseConnection,
        condition: Condition,
        order_column: Self::ColumnType,
        order_direction: Order,
        offset: u64,
        limit: u64,
    ) -> Result<Vec<Self::ApiModel>, DbErr> {
        let models = Self::EntityType::find()
            .filter(condition)
            .order_by(order_column, order_direction)
            .offset(offset)
            .limit(limit)
            .all(db)
            .await?;
        Ok(models.into_iter().map(Self::ApiModel::from).collect())
    }

    async fn get_one(db: &DatabaseConnection, id: Uuid) -> Result<Self::ApiModel, DbErr> {
        let model =
            Self::EntityType::find_by_id(id)
                .one(db)
                .await?
                .ok_or(DbErr::RecordNotFound(format!(
                    "{} not found",
                    Self::RESOURCE_NAME_SINGULAR
                )))?;
        Ok(Self::ApiModel::from(model))
    }

    async fn update(
        db: &DatabaseConnection,
        id: Uuid,
        update_data: Self::UpdateModel,
    ) -> Result<Self::ApiModel, DbErr> {
        let existing: Self::ActiveModelType = Self::EntityType::find_by_id(id)
            .one(db)
            .await?
            .ok_or(DbErr::RecordNotFound(format!(
                "{} not found",
                Self::RESOURCE_NAME_PLURAL
            )))?
            .into();

        let updated_model = update_data.merge_into_activemodel(existing);
        let updated = updated_model.update(db).await?;
        Ok(Self::ApiModel::from(updated))
    }

    fn sortable_columns() -> Vec<(&'static str, Self::ColumnType)> {
        vec![
            ("id", Self::ColumnType::Id),
            ("name", Self::ColumnType::Name),
            ("iso_a2", Self::ColumnType::IsoA2),
            ("iso_a3", Self::ColumnType::IsoA3),
            ("iso_n3", Self::ColumnType::IsoN3),
        ]
    }

    fn filterable_columns() -> Vec<(&'static str, Self::ColumnType)> {
        vec![
            ("name", Self::ColumnType::Name),
            ("iso_a2", Self::ColumnType::IsoA2),
            ("iso_a3", Self::ColumnType::IsoA3),
            ("iso_n3", Self::ColumnType::IsoN3),
        ]
    }
}
