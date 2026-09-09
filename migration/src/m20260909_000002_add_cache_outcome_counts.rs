use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Hits and misses are an outcome of the tile requests already counted in
        // xyz_tile_count, so they stay out of the total_views trigger.
        db.execute_unprepared(
            "ALTER TABLE layer_statistics
                ADD COLUMN cache_hit_count INTEGER NOT NULL DEFAULT 0,
                ADD COLUMN cache_miss_count INTEGER NOT NULL DEFAULT 0;",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE layer_statistics
                    DROP COLUMN cache_hit_count,
                    DROP COLUMN cache_miss_count;",
            )
            .await?;

        Ok(())
    }
}
