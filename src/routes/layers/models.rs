use std::vec;

use super::db::Model;
use async_trait::async_trait;
use chrono::Utc;
use crudcrate::{CRUDResource, ToCreateModel, ToUpdateModel};
use sea_orm::{
    ActiveValue, Condition, DatabaseConnection, EntityTrait, Order, QueryOrder, QuerySelect,
    entity::prelude::*,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, ToSchema)]
pub struct LayerStyle {
    pub id: Option<uuid::Uuid>,
    pub name: Option<String>,
    pub last_updated: Option<chrono::DateTime<Utc>>,
    pub style: Vec<crate::routes::styles::models::StyleItem>,
}

#[derive(ToSchema, Serialize, Deserialize, ToUpdateModel, ToCreateModel, Clone)]
#[active_model = "super::db::ActiveModel"]
pub struct Layer {
    #[crudcrate(update_model = false, update_model = false, on_create = Uuid::new_v4())]
    id: Uuid,
    layer_name: Option<String>,
    crop: Option<String>,
    water_model: Option<String>,
    climate_model: Option<String>,
    scenario: Option<String>,
    variable: Option<String>,
    year: Option<i32>,
    // iterator: i32,
    enabled: bool,
    uploaded_at: chrono::DateTime<Utc>,
    #[crudcrate(update_model = false, create_model = false, on_update = chrono::Utc::now(), on_create = chrono::Utc::now())]
    last_updated: chrono::DateTime<Utc>,
    global_average: Option<f64>,
    filename: Option<String>,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    style_id: Option<Uuid>,
    is_crop_specific: bool,
    #[crudcrate(non_db_attr=true)]
    pub style: Option<LayerStyle>,
}

impl From<Model> for Layer {
    fn from(model: Model) -> Self {
        Self {
            id: model.id,
            layer_name: model.layer_name,
            crop: model.crop,
            water_model: model.water_model,
            climate_model: model.climate_model,
            scenario: model.scenario,
            variable: model.variable,
            year: model.year,
            // iterator: model.iterator,
            enabled: model.enabled,
            uploaded_at: model.uploaded_at,
            last_updated: model.last_updated,
            global_average: model.global_average,
            filename: model.filename,
            min_value: model.min_value,
            max_value: model.max_value,
            style_id: model.style_id,
            is_crop_specific: model.is_crop_specific,
            style: None,
        }
    }
}

impl From<(Model, crate::routes::styles::db::Model)> for Layer {
    fn from((model, style_model): (Model, crate::routes::styles::db::Model)) -> Self {
        let style_items = crate::routes::styles::models::StyleItem::from_json(
            &style_model.style.unwrap_or_default(),
            model.min_value.unwrap_or_default(),
            model.max_value.unwrap_or_default(),
            10,
        );

        let layer_style = LayerStyle {
            id: Some(style_model.id),
            name: Some(style_model.name),
            last_updated: Some(style_model.last_updated),
            style: style_items,
        };

        let mut layer = Self::from(model);
        layer.style = Some(layer_style);
        layer
    }
}

#[async_trait]
impl CRUDResource for Layer {
    type EntityType = super::db::Entity;
    type ColumnType = super::db::Column;
    type ModelType = super::db::Model;
    type ActiveModelType = super::db::ActiveModel;
    type ApiModel = Layer;
    type CreateModel = LayerCreate;
    type UpdateModel = LayerUpdate;

    const ID_COLUMN: Self::ColumnType = super::db::Column::Id;
    const RESOURCE_NAME_PLURAL: &'static str = "layers";
    const RESOURCE_NAME_SINGULAR: &'static str = "layer";
    const RESOURCE_DESCRIPTION: &'static str = "This resource represents a raster layer and its metadata. It includes information about the layer name, crop, water model, climate model, scenario, variable, year, and other attributes. The resource also includes the last updated timestamp and the global average value for the layer.";

    async fn get_all(
        db: &DatabaseConnection,
        condition: Condition,
        order_column: Self::ColumnType,
        order_direction: Order,
        offset: u64,
        limit: u64,
    ) -> Result<Vec<Self::ApiModel>, DbErr> {
        let objs = Self::EntityType::find()
            .filter(condition)
            .order_by(order_column, order_direction)
            .offset(offset)
            .limit(limit)
            .all(db)
            .await?;
        let styles = objs.load_one(crate::routes::styles::db::Entity, db).await?;

        let mut models: Vec<Layer> = vec![];
        for (model, style_option) in objs.into_iter().zip(styles.into_iter()) {
            let layer: Layer = match style_option {
                Some(style) => Self::ApiModel::from((model, style)),
                None => {
                    // Create a default style if none exists
                    let style_items = crate::routes::styles::models::StyleItem::from_json(
                        &serde_json::Value::Null,
                        model.min_value.unwrap_or_default(),
                        model.max_value.unwrap_or_default(),
                        10,
                    );
                    let layer_style = LayerStyle {
                        id: None,
                        name: None,
                        last_updated: None,
                        style: style_items,
                    };
                    let mut layer = Self::ApiModel::from(model);
                    layer.style = Some(layer_style);
                    layer
                }
            };
            models.push(layer);
        }

        Ok(models)
    }

    async fn get_one(db: &DatabaseConnection, id: Uuid) -> Result<Self::ApiModel, DbErr> {
        let (model, style_option) = Self::EntityType::find_by_id(id)
            .find_with_related(crate::routes::styles::db::Entity)
            .one(db)
            .await?
            .ok_or(DbErr::RecordNotFound(format!(
                "{} not found",
                Self::RESOURCE_NAME_SINGULAR
            )))?;

        let layer: Layer = match style_option {
            Some(style) => Self::ApiModel::from((model, style)),
            None => {
                // Create a default style if none exists
                let style_items = crate::routes::styles::models::StyleItem::from_json(
                    &serde_json::Value::Null,
                    model.min_value.unwrap_or_default(),
                    model.max_value.unwrap_or_default(),
                    10,
                );
                let layer_style = LayerStyle {
                    id: None,
                    name: None,
                    last_updated: None,
                    style: style_items,
                };
                let mut layer = Self::ApiModel::from(model);
                layer.style = Some(layer_style);
                layer
            }
        };
        Ok(layer)
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
            ("name", Self::ColumnType::LayerName),
            ("crop", Self::ColumnType::Crop),
            ("water_model", Self::ColumnType::WaterModel),
            ("climate_model", Self::ColumnType::ClimateModel),
            ("scenario", Self::ColumnType::Scenario),
            ("variable", Self::ColumnType::Variable),
            ("year", Self::ColumnType::Year),
            ("enabled", Self::ColumnType::Enabled),
            ("uploaded_at", Self::ColumnType::UploadedAt),
            ("last_updated", Self::ColumnType::LastUpdated),
            ("global_average", Self::ColumnType::GlobalAverage),
            ("filename", Self::ColumnType::Filename),
            ("min_value", Self::ColumnType::MinValue),
            ("max_value", Self::ColumnType::MaxValue),
        ]
    }

    fn filterable_columns() -> Vec<(&'static str, Self::ColumnType)> {
        vec![
            ("name", Self::ColumnType::LayerName),
            ("crop", Self::ColumnType::Crop),
            ("water_model", Self::ColumnType::WaterModel),
            ("climate_model", Self::ColumnType::ClimateModel),
            ("scenario", Self::ColumnType::Scenario),
            ("variable", Self::ColumnType::Variable),
            ("year", Self::ColumnType::Year),
            ("enabled", Self::ColumnType::Enabled),
            ("uploaded_at", Self::ColumnType::UploadedAt),
            ("last_updated", Self::ColumnType::LastUpdated),
            ("global_average", Self::ColumnType::GlobalAverage),
            ("filename", Self::ColumnType::Filename),
            ("min_value", Self::ColumnType::MinValue),
            ("max_value", Self::ColumnType::MaxValue),
        ]
    }
}
