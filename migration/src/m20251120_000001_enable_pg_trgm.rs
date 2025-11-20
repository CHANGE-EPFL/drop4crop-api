use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Enable pg_trgm extension for similarity search
        manager
            .get_connection()
            .execute_unprepared("CREATE EXTENSION IF NOT EXISTS pg_trgm;")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop pg_trgm extension
        manager
            .get_connection()
            .execute_unprepared("DROP EXTENSION IF EXISTS pg_trgm;")
            .await?;

        Ok(())
    }
}
