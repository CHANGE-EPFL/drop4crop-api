use async_trait::async_trait;
use chrono::{DateTime, Utc};
use crudcrate::{CRUDResource, ToCreateModel, ToUpdateModel};
use sea_orm::{
    ActiveValue, Condition, DatabaseConnection, EntityTrait, FromQueryResult, Order, QueryOrder,
    QuerySelect, entity::prelude::*,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(
    ToSchema, Serialize, Deserialize, FromQueryResult, ToUpdateModel, ToCreateModel, Clone,
)]
#[active_model = "super::db::ActiveModel"]
pub struct Style {
    #[crudcrate(update_model = false, create_model = false, on_create = Uuid::new_v4())]
    pub id: Uuid,
    pub name: String,
    #[crudcrate(
        create_model = false,
        update_model = false,
        on_create = chrono::Utc::now(),
        on_update = chrono::Utc::now()
    )]
    pub last_updated: DateTime<Utc>,
    pub style: Option<Value>,
}

impl From<super::db::Model> for Style {
    fn from(model: super::db::Model) -> Self {
        Self {
            id: model.id,
            name: model.name,
            last_updated: model.last_updated,
            style: model.style,
        }
    }
}

#[async_trait]
impl CRUDResource for Style {
    type EntityType = super::db::Entity;
    type ColumnType = super::db::Column;
    type ModelType = super::db::Model;
    type ActiveModelType = super::db::ActiveModel;
    type ApiModel = Style;
    type CreateModel = StyleCreate;
    type UpdateModel = StyleUpdate;

    const ID_COLUMN: Self::ColumnType = super::db::Column::Id;
    const RESOURCE_NAME_PLURAL: &'static str = "styles";
    const RESOURCE_NAME_SINGULAR: &'static str = "style";
    const RESOURCE_DESCRIPTION: &'static str = "This resource represents a style configuration, including a list of style items and a name.";

    async fn get_all(
        db: &DatabaseConnection,
        condition: Condition,
        order_column: Self::ColumnType,
        order_direction: Order,
        offset: u64,
        limit: u64,
    ) -> Result<Vec<Self::ApiModel>, sea_orm::DbErr> {
        let models = Self::EntityType::find()
            .filter(condition)
            .order_by(order_column, order_direction)
            .offset(offset)
            .limit(limit)
            .all(db)
            .await?;
        Ok(models.into_iter().map(Self::ApiModel::from).collect())
    }

    async fn get_one(db: &DatabaseConnection, id: Uuid) -> Result<Self::ApiModel, sea_orm::DbErr> {
        let model = Self::EntityType::find_by_id(id).one(db).await?.ok_or(
            sea_orm::DbErr::RecordNotFound(format!("{} not found", Self::RESOURCE_NAME_SINGULAR)),
        )?;
        Ok(Self::ApiModel::from(model))
    }

    async fn update(
        db: &DatabaseConnection,
        id: Uuid,
        update_data: Self::UpdateModel,
    ) -> Result<Self::ApiModel, sea_orm::DbErr> {
        let existing: Self::ActiveModelType = Self::EntityType::find_by_id(id)
            .one(db)
            .await?
            .ok_or(sea_orm::DbErr::RecordNotFound(format!(
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
            ("last_updated", Self::ColumnType::LastUpdated),
        ]
    }

    fn filterable_columns() -> Vec<(&'static str, Self::ColumnType)> {
        vec![("name", Self::ColumnType::Name)]
    }
}

#[derive(Serialize, Deserialize, Clone, ToSchema)]
pub struct StyleItem {
    pub value: f64,
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub opacity: u8,
    pub label: f64,
}

impl StyleItem {
    // Takes a JSON that is typically stored in the postgres db but rendered
    // as a serde_json::Value, sorts it and returns a Vec<StyleItem>, if the
    // JSON is empty, it generates a grayscale style based on the minimum and maximum
    // raster values of the layer which are passed in as parameters.
    pub fn from_json(
        json: &serde_json::Value,
        layer_min: f64,
        layer_max: f64,
        num_segments: usize,
    ) -> Vec<StyleItem> {
        let json_array = match json.as_array() {
            Some(array) => array,
            None => &vec![],
        };
        let mut style = vec![];

        if json_array.is_empty() {
            Self::generate_grayscale_style(layer_min, layer_max, num_segments)
        } else {
            for item in json_array {
                if let Some(value) = item.get("value") {
                    if let Some(value) = value.as_f64() {
                        let red = item.get("red").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                        let green = item.get("green").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                        let blue = item.get("blue").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                        let opacity =
                            item.get("opacity").and_then(|v| v.as_u64()).unwrap_or(255) as u8;
                        let label = item.get("label").and_then(|v| v.as_f64()).unwrap_or(value);

                        style.push(StyleItem {
                            value,
                            red,
                            green,
                            blue,
                            opacity,
                            label,
                        });
                    }
                }
            }
            Self::sort_styles(style)
        }
    }

    pub fn sort_styles(mut style_list: Vec<StyleItem>) -> Vec<StyleItem> {
        style_list.sort_by(|a, b| {
            a.value
                .partial_cmp(&b.value)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        style_list
    }

    pub fn generate_grayscale_style(min: f64, max: f64, num_segments: usize) -> Vec<StyleItem> {
        let step = (max - min) / num_segments as f64;
        let mut style = Vec::with_capacity(num_segments);

        for i in 0..num_segments {
            let value = min + i as f64 * step;
            let grey_value =
                ((255.0 * i as f64) / (num_segments.saturating_sub(1) as f64)).round() as u8;
            style.push(crate::routes::styles::models::StyleItem {
                value,
                red: grey_value,
                green: grey_value,
                blue: grey_value,
                opacity: 255,
                label: (value * 10000.0).round() / 10000.0, // round to 4 decimal places
            });
        }

        style
    }
}
