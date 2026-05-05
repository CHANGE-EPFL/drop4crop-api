use crudcrate::{ApiError, CRUDResource, EntityToModels};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, EntityToModels, serde::Serialize, serde::Deserialize)]
#[sea_orm(table_name = "climate_model")]
#[crudcrate(
    api_struct = "ClimateModel",
    name_singular = "climate model",
    name_plural = "climate-models",
    generate_router,
    operations = ClimateModelOperations,
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    #[crudcrate(primary_key, filterable, exclude(update, create), on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[sea_orm(unique)]
    #[crudcrate(filterable, fulltext)]
    pub slug: String,
    #[crudcrate(filterable, fulltext)]
    pub name: String,
    #[crudcrate(sortable)]
    pub sort_order: i32,
    #[sea_orm(ignore)]
    #[crudcrate(non_db_attr = true, exclude(create, update))]
    pub layer_count: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

pub struct ClimateModelOperations;

#[async_trait::async_trait]
impl crudcrate::CRUDOperations for ClimateModelOperations {
    type Resource = ClimateModel;

    async fn after_get_one(
        &self,
        db: &sea_orm::DatabaseConnection,
        entity: &mut Self::Resource,
    ) -> Result<(), ApiError> {
        entity.layer_count =
            Some(crate::common::layer_counts::fetch_layer_count(db, "climate_model_id", entity.id).await?);
        Ok(())
    }

    async fn after_get_all(
        &self,
        db: &sea_orm::DatabaseConnection,
        entities: &mut Vec<<Self::Resource as crudcrate::CRUDResource>::ListModel>,
    ) -> Result<(), ApiError> {
        let ids: Vec<Uuid> = entities.iter().map(|e| e.id).collect();
        let map = crate::common::layer_counts::fetch_layer_counts(db, "climate_model_id", &ids).await?;
        for e in entities.iter_mut() {
            e.layer_count = Some(map.get(&e.id).copied().unwrap_or(0));
        }
        Ok(())
    }
}
