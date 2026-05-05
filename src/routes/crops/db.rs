use crudcrate::{ApiError, CRUDResource, EntityToModels};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, EntityToModels, serde::Serialize, serde::Deserialize)]
#[sea_orm(table_name = "crop")]
#[crudcrate(
    api_struct = "Crop",
    name_singular = "crop",
    name_plural = "crops",
    generate_router,
    operations = CropOperations,
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

pub struct CropOperations;

#[async_trait::async_trait]
impl crudcrate::CRUDOperations for CropOperations {
    type Resource = Crop;

    async fn after_get_one(
        &self,
        db: &sea_orm::DatabaseConnection,
        entity: &mut Self::Resource,
    ) -> Result<(), ApiError> {
        entity.layer_count = Some(fetch_layer_count(db, "crop_id", entity.id).await?);
        Ok(())
    }

    async fn after_get_all(
        &self,
        db: &sea_orm::DatabaseConnection,
        entities: &mut Vec<<Self::Resource as crudcrate::CRUDResource>::ListModel>,
    ) -> Result<(), ApiError> {
        if entities.is_empty() {
            return Ok(());
        }
        let ids: Vec<Uuid> = entities.iter().map(|e| e.id).collect();
        let map = fetch_layer_counts(db, "crop_id", &ids).await?;
        for e in entities.iter_mut() {
            e.layer_count = Some(map.get(&e.id).copied().unwrap_or(0));
        }
        Ok(())
    }
}

async fn fetch_layer_count(
    db: &sea_orm::DatabaseConnection,
    fk: &str,
    id: Uuid,
) -> Result<i64, ApiError> {
    let sql = format!("SELECT COUNT(*)::bigint AS cnt FROM layer WHERE {fk} = $1");
    let row = db
        .query_one(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            &sql,
            [id.into()],
        ))
        .await
        .map_err(ApiError::database)?;
    Ok(row.and_then(|r| r.try_get::<i64>("", "cnt").ok()).unwrap_or(0))
}

async fn fetch_layer_counts(
    db: &sea_orm::DatabaseConnection,
    fk: &str,
    ids: &[Uuid],
) -> Result<std::collections::HashMap<Uuid, i64>, ApiError> {
    let placeholders = (1..=ids.len())
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {fk} AS fk, COUNT(*)::bigint AS cnt FROM layer \
         WHERE {fk} IN ({placeholders}) GROUP BY {fk}"
    );
    let values: Vec<sea_orm::Value> = ids.iter().map(|id| (*id).into()).collect();
    let rows = db
        .query_all(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            &sql,
            values,
        ))
        .await
        .map_err(ApiError::database)?;
    let mut map = std::collections::HashMap::with_capacity(rows.len());
    for row in rows {
        if let (Ok(id), Ok(cnt)) = (
            row.try_get::<Uuid>("", "fk"),
            row.try_get::<i64>("", "cnt"),
        ) {
            map.insert(id, cnt);
        }
    }
    Ok(map)
}
