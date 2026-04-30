use sea_orm_migration::prelude::*;
use tracing::info;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // --- A1: Make variable.name and variable.abbreviation nullable ---
        manager
            .alter_table(
                Table::alter()
                    .table(Variable::Table)
                    .modify_column(ColumnDef::new(Variable::Name).string().null())
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Variable::Table)
                    .modify_column(ColumnDef::new(Variable::Abbreviation).string().null())
                    .to_owned(),
            )
            .await?;

        info!("Made variable.name and variable.abbreviation nullable");

        // --- A2b: Missing FK indexes ---
        db.execute_unprepared("CREATE INDEX idx_layer_style_id ON layer (style_id)")
            .await?;
        db.execute_unprepared("CREATE INDEX idx_showcase_item_layer_id ON showcase_item (layer_id)")
            .await?;
        db.execute_unprepared("CREATE INDEX idx_project_card_layer_id ON project (card_layer_id)")
            .await?;
        db.execute_unprepared("CREATE INDEX idx_project_card_style_id ON project (card_style_id)")
            .await?;

        info!("Added missing FK indexes");

        // --- Boolean filter indexes ---
        db.execute_unprepared("CREATE INDEX idx_layer_enabled ON layer (enabled)")
            .await?;
        db.execute_unprepared("CREATE INDEX idx_showcase_item_enabled ON showcase_item (enabled)")
            .await?;

        info!("Added boolean filter indexes");

        // --- Junction table sort_order indexes ---
        db.execute_unprepared(
            "CREATE INDEX idx_project_crop_sort ON project_crop (project_id, sort_order)",
        )
        .await?;
        db.execute_unprepared(
            "CREATE INDEX idx_project_water_model_sort ON project_water_model (project_id, sort_order)",
        )
        .await?;
        db.execute_unprepared(
            "CREATE INDEX idx_project_climate_model_sort ON project_climate_model (project_id, sort_order)",
        )
        .await?;
        db.execute_unprepared(
            "CREATE INDEX idx_project_scenario_sort ON project_scenario (project_id, sort_order)",
        )
        .await?;
        db.execute_unprepared(
            "CREATE INDEX idx_project_variable_sort ON project_variable (project_id, sort_order)",
        )
        .await?;

        info!("Added junction table sort_order indexes");

        // --- Reference table sort_order indexes ---
        db.execute_unprepared("CREATE INDEX idx_crop_sort_order ON crop (sort_order)")
            .await?;
        db.execute_unprepared("CREATE INDEX idx_water_model_sort_order ON water_model (sort_order)")
            .await?;
        db.execute_unprepared(
            "CREATE INDEX idx_climate_model_sort_order ON climate_model (sort_order)",
        )
        .await?;
        db.execute_unprepared("CREATE INDEX idx_scenario_sort_order ON scenario (sort_order)")
            .await?;
        db.execute_unprepared("CREATE INDEX idx_variable_sort_order ON variable (sort_order)")
            .await?;

        info!("Added reference table sort_order indexes");

        // --- Compound index for common multi-filter queries ---
        db.execute_unprepared(
            "CREATE INDEX idx_layer_project_enabled ON layer (project_id, enabled)",
        )
        .await?;

        // --- Fulltext GIN indexes (pg_trgm was enabled in m20251120) ---
        db.execute_unprepared(
            "CREATE INDEX idx_layer_name_trgm ON layer USING GIN (layer_name gin_trgm_ops)",
        )
        .await?;
        db.execute_unprepared(
            "CREATE INDEX idx_variable_name_trgm ON variable USING GIN (name gin_trgm_ops)",
        )
        .await?;
        db.execute_unprepared(
            "CREATE INDEX idx_project_title_trgm ON project USING GIN (title gin_trgm_ops)",
        )
        .await?;
        db.execute_unprepared(
            "CREATE INDEX idx_showcase_item_title_trgm ON showcase_item USING GIN (title gin_trgm_ops)",
        )
        .await?;

        info!("Added fulltext GIN indexes");

        // --- Drop redundant indexes (duplicated by PK or UNIQUE constraints) ---
        db.execute_unprepared("DROP INDEX IF EXISTS ix_layer_id").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS ix_layer_layer_name").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS ix_style_id").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS ix_style_name").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS ix_project_id").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS ix_project_slug").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS ix_showcase_item_id").await?;

        info!("Dropped redundant indexes");

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Restore redundant indexes
        db.execute_unprepared("CREATE INDEX ix_layer_id ON layer (id)").await?;
        db.execute_unprepared("CREATE INDEX ix_layer_layer_name ON layer (layer_name)").await?;
        db.execute_unprepared("CREATE INDEX ix_style_id ON style (id)").await?;
        db.execute_unprepared("CREATE INDEX ix_style_name ON style (name)").await?;
        db.execute_unprepared("CREATE INDEX ix_project_id ON project (id)").await?;
        db.execute_unprepared("CREATE INDEX ix_project_slug ON project (slug)").await?;
        db.execute_unprepared("CREATE INDEX ix_showcase_item_id ON showcase_item (id)").await?;

        // Drop fulltext indexes
        db.execute_unprepared("DROP INDEX IF EXISTS idx_showcase_item_title_trgm").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_project_title_trgm").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_variable_name_trgm").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_layer_name_trgm").await?;

        // Drop compound index
        db.execute_unprepared("DROP INDEX IF EXISTS idx_layer_project_enabled").await?;

        // Drop reference table sort_order indexes
        db.execute_unprepared("DROP INDEX IF EXISTS idx_variable_sort_order").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_scenario_sort_order").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_climate_model_sort_order").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_water_model_sort_order").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_crop_sort_order").await?;

        // Drop junction table sort_order indexes
        db.execute_unprepared("DROP INDEX IF EXISTS idx_project_variable_sort").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_project_scenario_sort").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_project_climate_model_sort").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_project_water_model_sort").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_project_crop_sort").await?;

        // Drop boolean filter indexes
        db.execute_unprepared("DROP INDEX IF EXISTS idx_showcase_item_enabled").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_layer_enabled").await?;

        // Drop FK indexes
        db.execute_unprepared("DROP INDEX IF EXISTS idx_project_card_style_id").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_project_card_layer_id").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_showcase_item_layer_id").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS idx_layer_style_id").await?;

        // Restore NOT NULL on variable fields (backfill nulls first)
        db.execute_unprepared(
            "UPDATE variable SET name = slug WHERE name IS NULL",
        )
        .await?;
        db.execute_unprepared(
            "UPDATE variable SET abbreviation = '' WHERE abbreviation IS NULL",
        )
        .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Variable::Table)
                    .modify_column(
                        ColumnDef::new(Variable::Abbreviation).string().not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Variable::Table)
                    .modify_column(ColumnDef::new(Variable::Name).string().not_null())
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Variable {
    Table,
    Name,
    Abbreviation,
}
