use chrono::{DateTime, Utc};
use crudcrate::{CRUDResource, EntityToModels};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, DeriveEntityModel, EntityToModels)]
#[sea_orm(table_name = "layer")]
#[crudcrate(
    api_struct = "Layer",
    name_singular = "layer",
    name_plural = "layers",
    fn_delete_many = delete_many,
    generate_router,
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    #[crudcrate(primary_key, exclude(update, create), on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[sea_orm(unique)]
    #[crudcrate(filterable)]
    pub layer_name: Option<String>,
    #[crudcrate(filterable)]
    pub crop: Option<String>,
    #[crudcrate(filterable)]
    pub water_model: Option<String>,
    #[crudcrate(filterable)]
    pub climate_model: Option<String>,
    #[crudcrate(filterable)]
    pub scenario: Option<String>,
    #[crudcrate(filterable)]
    pub variable: Option<String>,
    #[crudcrate(filterable)]
    pub year: Option<i32>,
    #[crudcrate(filterable, sortable)]
    pub last_updated: DateTime<Utc>,
    #[crudcrate(filterable)]
    pub enabled: bool,
    pub uploaded_at: DateTime<Utc>,
    #[sea_orm(column_type = "Double", nullable, sortable)]
    pub global_average: Option<f64>,
    pub filename: Option<String>,
    #[sea_orm(column_type = "Double", nullable)]
    pub min_value: Option<f64>,
    #[sea_orm(column_type = "Double", nullable)]
    pub max_value: Option<f64>,
    #[crudcrate(filterable)]
    pub style_id: Option<Uuid>,
    #[crudcrate(filterable)]
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

pub async fn delete_many(
    db: &sea_orm::DatabaseConnection,
    ids: Vec<Uuid>,
) -> Result<Vec<Uuid>, sea_orm::DbErr> {
    println!("Called delete_many with IDs: {:?}", ids);
    let mut deleted_ids = Vec::new();

    for id in &ids {
        let _ = crate::routes::tiles::storage::delete_s3_object_by_db_id(db, id).await;

        if Entity::delete_by_id(*id).exec(db).await.is_ok() {
            deleted_ids.push(*id);
        }
    }

    Ok(deleted_ids)
}
