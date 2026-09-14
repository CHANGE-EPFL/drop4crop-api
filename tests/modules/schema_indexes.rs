// Index coverage tests for the queries that run on first page load

use sea_orm::{ConnectionTrait, Statement};

use crate::common::db::create_test_db;

async fn index_definitions(table: &str) -> Vec<String> {
    let db = create_test_db().await.expect("test database");
    let backend = db.get_database_backend();
    db.query_all(Statement::from_sql_and_values(
        backend,
        "SELECT indexdef FROM pg_indexes WHERE schemaname = 'public' AND tablename = $1",
        [table.into()],
    ))
    .await
    .expect("read pg_indexes")
    .into_iter()
    .map(|row| row.try_get::<String>("", "indexdef").expect("indexdef"))
    .collect()
}

#[tokio::test]
async fn test_layer_year_axis_index_covers_project_enabled_year() {
    let defs = index_definitions("layer").await;
    assert!(
        defs.iter()
            .any(|d| d.contains("(project_id, enabled, year)")),
        "no index covers the groups year query, found: {defs:?}"
    );
}

#[tokio::test]
async fn test_layer_total_views_is_unindexed_so_its_updates_are_hot() {
    let defs = index_definitions("layer").await;
    assert!(
        !defs.iter().any(|d| d.contains("(total_views)")),
        "total_views is indexed, so the statistics sync cannot update it HOT: {defs:?}"
    );
}
