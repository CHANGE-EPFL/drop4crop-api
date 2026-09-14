use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // The year axis of /layers/groups reads DISTINCT year for a project's
        // enabled layers; with year in the index it never visits the heap.
        db.execute_unprepared(
            "CREATE INDEX idx_layer_project_enabled_year ON layer (project_id, enabled, year)",
        )
        .await?;

        // Its columns are a prefix of the new index.
        db.execute_unprepared("DROP INDEX IF EXISTS idx_layer_project_enabled")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared("CREATE INDEX idx_layer_project_enabled ON layer (project_id, enabled)")
            .await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_layer_project_enabled_year")
            .await?;

        Ok(())
    }
}
