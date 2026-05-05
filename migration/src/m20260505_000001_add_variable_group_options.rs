use sea_orm_migration::prelude::*;
use tracing::info;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            "ALTER TABLE variable_group
               ADD COLUMN required_crop_id UUID REFERENCES crop(id) ON DELETE SET NULL,
               ADD COLUMN display_stacked BOOLEAN NOT NULL DEFAULT false",
        )
        .await?;
        info!("Added required_crop_id and display_stacked to variable_group");

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            "ALTER TABLE variable_group
               DROP COLUMN IF EXISTS required_crop_id,
               DROP COLUMN IF EXISTS display_stacked",
        )
        .await?;

        Ok(())
    }
}
