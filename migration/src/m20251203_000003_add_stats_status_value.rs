use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add a generated column that extracts the 'status' value from the stats_status JSON
        // This allows us to filter by stats_status_value using standard SQL
        // Values will be: 'success', 'error', 'pending', or NULL
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE layer
                ADD COLUMN stats_status_value TEXT
                GENERATED ALWAYS AS (stats_status->>'status') STORED;
                "#,
            )
            .await?;

        // Create an index for efficient filtering
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE INDEX idx_layer_stats_status_value ON layer (stats_status_value);
                "#,
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop the index first
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                DROP INDEX IF EXISTS idx_layer_stats_status_value;
                "#,
            )
            .await?;

        // Drop the generated column
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE layer DROP COLUMN IF EXISTS stats_status_value;
                "#,
            )
            .await?;

        Ok(())
    }
}
