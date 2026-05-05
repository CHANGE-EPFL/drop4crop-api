use crudcrate::ApiError;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement, Value};
use std::collections::HashMap;
use uuid::Uuid;

pub async fn fetch_layer_count(
    db: &DatabaseConnection,
    fk: &str,
    id: Uuid,
) -> Result<i64, ApiError> {
    let sql = format!("SELECT COUNT(*)::bigint AS cnt FROM layer WHERE {fk} = $1");
    let row = db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            &sql,
            [id.into()],
        ))
        .await
        .map_err(ApiError::database)?;
    Ok(row.and_then(|r| r.try_get::<i64>("", "cnt").ok()).unwrap_or(0))
}

pub async fn fetch_layer_counts(
    db: &DatabaseConnection,
    fk: &str,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, i64>, ApiError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = (1..=ids.len())
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {fk} AS fk, COUNT(*)::bigint AS cnt FROM layer \
         WHERE {fk} IN ({placeholders}) GROUP BY {fk}"
    );
    let values: Vec<Value> = ids.iter().map(|id| (*id).into()).collect();
    let rows = db
        .query_all(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            &sql,
            values,
        ))
        .await
        .map_err(ApiError::database)?;
    let mut map = HashMap::with_capacity(rows.len());
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
