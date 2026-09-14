use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The statistics sync rewrites total_views for every active layer every 30 s. An
        // index on the column is what stops those updates being HOT, so each one leaves a
        // dead tuple behind instead of reusing the page. Sorting by views takes a seq scan.
        manager
            .get_connection()
            .execute_unprepared("DROP INDEX IF EXISTS idx_layer_total_views")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("CREATE INDEX idx_layer_total_views ON layer (total_views)")
            .await?;

        Ok(())
    }
}
