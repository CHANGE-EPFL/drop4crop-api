use sea_orm_migration::prelude::*;
use tracing::info;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let is_postgres = manager.get_database_backend() == sea_orm::DatabaseBackend::Postgres;

        // ── 1. Add sort_order column to each of the five junction tables ─

        for (table_name, ref_column) in [
            ("project_crop", "crop_id"),
            ("project_water_model", "water_model_id"),
            ("project_climate_model", "climate_model_id"),
            ("project_scenario", "scenario_id"),
            ("project_variable", "variable_id"),
        ] {
            db.execute_unprepared(&format!(
                "ALTER TABLE {table_name} ADD COLUMN IF NOT EXISTS sort_order INTEGER NOT NULL DEFAULT 0"
            ))
            .await?;
            info!(
                "Added sort_order column to {table_name} (ref column: {ref_column})"
            );
        }

        // ── 2. Backfill existing rows ──────────────────────────────────
        // Seed each (project, ref) pair's sort_order from ROW_NUMBER() over
        // the reference entity's global sort_order, so existing projects keep
        // the ordering they had before this migration landed.

        if is_postgres {
            for (junction, junction_ref_col, ref_table, ref_pk) in [
                ("project_crop", "crop_id", "crop", "id"),
                (
                    "project_water_model",
                    "water_model_id",
                    "water_model",
                    "id",
                ),
                (
                    "project_climate_model",
                    "climate_model_id",
                    "climate_model",
                    "id",
                ),
                ("project_scenario", "scenario_id", "scenario", "id"),
                ("project_variable", "variable_id", "variable", "id"),
            ] {
                let sql = format!(
                    "UPDATE {junction} AS j \
                     SET sort_order = sub.rn - 1 \
                     FROM ( \
                        SELECT j2.project_id, j2.{junction_ref_col}, \
                               ROW_NUMBER() OVER ( \
                                   PARTITION BY j2.project_id \
                                   ORDER BY r.sort_order NULLS LAST, r.name \
                               ) AS rn \
                        FROM {junction} j2 \
                        JOIN {ref_table} r ON r.{ref_pk} = j2.{junction_ref_col} \
                     ) AS sub \
                     WHERE sub.project_id = j.project_id \
                       AND sub.{junction_ref_col} = j.{junction_ref_col}"
                );
                db.execute_unprepared(&sql).await?;
                info!("Backfilled sort_order in {junction}");
            }
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for table_name in [
            "project_crop",
            "project_water_model",
            "project_climate_model",
            "project_scenario",
            "project_variable",
        ] {
            db.execute_unprepared(&format!(
                "ALTER TABLE {table_name} DROP COLUMN IF EXISTS sort_order"
            ))
            .await?;
        }
        Ok(())
    }
}
