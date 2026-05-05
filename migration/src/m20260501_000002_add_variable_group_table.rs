use sea_orm_migration::prelude::*;
use tracing::info;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            "CREATE TABLE variable_group (
                id UUID NOT NULL UNIQUE,
                iterator SERIAL PRIMARY KEY,
                name VARCHAR NOT NULL,
                help_text TEXT,
                sort_order INTEGER NOT NULL DEFAULT 0,
                parent_id UUID REFERENCES variable_group(id)
            )",
        )
        .await?;
        info!("Created variable_group table");

        db.execute_unprepared(
            "ALTER TABLE variable ADD COLUMN group_id UUID REFERENCES variable_group(id)",
        )
        .await?;
        info!("Added group_id FK to variable table");

        db.execute_unprepared(
            "INSERT INTO variable_group (id, name, sort_order)
             SELECT gen_random_uuid(), sub.group_name, 0
             FROM (SELECT DISTINCT group_name FROM variable WHERE group_name IS NOT NULL) sub",
        )
        .await?;
        info!("Seeded variable_group from existing group_name values");

        db.execute_unprepared(
            "UPDATE variable SET group_id = vg.id
             FROM variable_group vg
             WHERE variable.group_name = vg.name",
        )
        .await?;
        info!("Populated group_id from existing group_name matches");

        db.execute_unprepared("CREATE INDEX idx_variable_group_id ON variable (group_id)")
            .await?;
        db.execute_unprepared("CREATE INDEX idx_variable_group_parent_id ON variable_group (parent_id)")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared("DROP INDEX IF EXISTS idx_variable_group_id")
            .await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_variable_group_parent_id")
            .await?;
        db.execute_unprepared("ALTER TABLE variable DROP COLUMN IF EXISTS group_id")
            .await?;
        db.execute_unprepared("DROP TABLE IF EXISTS variable_group")
            .await?;

        Ok(())
    }
}
