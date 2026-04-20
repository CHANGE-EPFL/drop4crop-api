use sea_orm_migration::prelude::*;
use tracing::info;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let is_postgres = manager.get_database_backend() == sea_orm::DatabaseBackend::Postgres;

        // ── 1. Create reference tables ──────────────────────────────────

        // crop
        let mut crop_table = Table::create()
            .table(Crop::Table)
            .if_not_exists()
            .col(ColumnDef::new(Crop::Slug).string().not_null().unique_key())
            .col(ColumnDef::new(Crop::Name).string().not_null())
            .col(ColumnDef::new(Crop::SortOrder).integer().not_null().default(0))
            .to_owned();
        match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres | sea_orm::DatabaseBackend::Sqlite => {
                crop_table.col(ColumnDef::new(Crop::Id).uuid().not_null().unique_key());
            }
            _ => return Err(DbErr::Custom("Unsupported database backend".to_string())),
        }
        crop_table.col(ColumnDef::new(Crop::Iterator).integer().not_null().auto_increment().primary_key());
        manager.create_table(crop_table).await?;

        // water_model
        let mut wm_table = Table::create()
            .table(WaterModel::Table)
            .if_not_exists()
            .col(ColumnDef::new(WaterModel::Slug).string().not_null().unique_key())
            .col(ColumnDef::new(WaterModel::Name).string().not_null())
            .col(ColumnDef::new(WaterModel::SortOrder).integer().not_null().default(0))
            .to_owned();
        match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres | sea_orm::DatabaseBackend::Sqlite => {
                wm_table.col(ColumnDef::new(WaterModel::Id).uuid().not_null().unique_key());
            }
            _ => return Err(DbErr::Custom("Unsupported database backend".to_string())),
        }
        wm_table.col(ColumnDef::new(WaterModel::Iterator).integer().not_null().auto_increment().primary_key());
        manager.create_table(wm_table).await?;

        // climate_model
        let mut cm_table = Table::create()
            .table(ClimateModel::Table)
            .if_not_exists()
            .col(ColumnDef::new(ClimateModel::Slug).string().not_null().unique_key())
            .col(ColumnDef::new(ClimateModel::Name).string().not_null())
            .col(ColumnDef::new(ClimateModel::SortOrder).integer().not_null().default(0))
            .to_owned();
        match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres | sea_orm::DatabaseBackend::Sqlite => {
                cm_table.col(ColumnDef::new(ClimateModel::Id).uuid().not_null().unique_key());
            }
            _ => return Err(DbErr::Custom("Unsupported database backend".to_string())),
        }
        cm_table.col(ColumnDef::new(ClimateModel::Iterator).integer().not_null().auto_increment().primary_key());
        manager.create_table(cm_table).await?;

        // scenario
        let mut sc_table = Table::create()
            .table(Scenario::Table)
            .if_not_exists()
            .col(ColumnDef::new(Scenario::Slug).string().not_null().unique_key())
            .col(ColumnDef::new(Scenario::Name).string().not_null())
            .col(ColumnDef::new(Scenario::SortOrder).integer().not_null().default(0))
            .to_owned();
        match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres | sea_orm::DatabaseBackend::Sqlite => {
                sc_table.col(ColumnDef::new(Scenario::Id).uuid().not_null().unique_key());
            }
            _ => return Err(DbErr::Custom("Unsupported database backend".to_string())),
        }
        sc_table.col(ColumnDef::new(Scenario::Iterator).integer().not_null().auto_increment().primary_key());
        manager.create_table(sc_table).await?;

        // variable
        let mut var_table = Table::create()
            .table(Variable::Table)
            .if_not_exists()
            .col(ColumnDef::new(Variable::Slug).string().not_null().unique_key())
            .col(ColumnDef::new(Variable::Name).string().not_null())
            .col(ColumnDef::new(Variable::Abbreviation).string().not_null().default(""))
            .col(ColumnDef::new(Variable::Subscript).string())
            .col(ColumnDef::new(Variable::Unit).string().not_null().default(""))
            .col(ColumnDef::new(Variable::IsCropSpecific).boolean().not_null().default(false))
            .col(ColumnDef::new(Variable::GroupName).string())
            .col(ColumnDef::new(Variable::SortOrder).integer().not_null().default(0))
            .to_owned();
        match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres | sea_orm::DatabaseBackend::Sqlite => {
                var_table.col(ColumnDef::new(Variable::Id).uuid().not_null().unique_key());
            }
            _ => return Err(DbErr::Custom("Unsupported database backend".to_string())),
        }
        var_table.col(ColumnDef::new(Variable::Iterator).integer().not_null().auto_increment().primary_key());
        manager.create_table(var_table).await?;

        info!("Created reference tables: crop, water_model, climate_model, scenario, variable");

        // ── 2. Seed reference data (PostgreSQL) ─────────────────────────

        if is_postgres {
            // The consolidated schema migration only enables uuid-ossp on a
            // fresh DB; deployments that were migrated over from the old
            // Alembic state took the skip-setup branch and never got it.
            // Declare the dependency explicitly here so the migration is
            // self-contained.
            db.execute_unprepared("CREATE EXTENSION IF NOT EXISTS \"uuid-ossp\"")
                .await?;

            // Crops
            let crops = [
                ("barley", "Barley", 0),
                ("maize", "Maize", 1),
                ("potato", "Potato", 2),
                ("rice", "Rice", 3),
                ("sorghum", "Sorghum", 4),
                ("soy", "Soy", 5),
                ("sugarcane", "Sugar Cane", 6),
                ("wheat", "Wheat", 7),
            ];
            for (slug, name, sort) in &crops {
                db.execute_unprepared(&format!(
                    "INSERT INTO crop (id, slug, name, sort_order) VALUES (uuid_generate_v4(), '{}', '{}', {}) ON CONFLICT (slug) DO NOTHING",
                    slug, name, sort
                )).await?;
            }

            // Water models
            let water_models = [
                ("cwatm", "CWatM", 0),
                ("h08", "H08", 1),
                ("lpjml", "LPJmL", 2),
                ("matsiro", "MATSIRO", 3),
                ("pcr-globwb", "PCR-GLOBWB", 4),
                ("watergap2", "WaterGAP2", 5),
            ];
            for (slug, name, sort) in &water_models {
                db.execute_unprepared(&format!(
                    "INSERT INTO water_model (id, slug, name, sort_order) VALUES (uuid_generate_v4(), '{}', '{}', {}) ON CONFLICT (slug) DO NOTHING",
                    slug, name, sort
                )).await?;
            }

            // Climate models
            let climate_models = [
                ("gfdl-esm2m", "GFDL-ESM2M", 0),
                ("hadgem2-es", "HadGEM2-ES", 1),
                ("ipsl-cm5a-lr", "IPSL-CM5A-LR", 2),
                ("miroc5", "MIROC5", 3),
            ];
            for (slug, name, sort) in &climate_models {
                db.execute_unprepared(&format!(
                    "INSERT INTO climate_model (id, slug, name, sort_order) VALUES (uuid_generate_v4(), '{}', '{}', {}) ON CONFLICT (slug) DO NOTHING",
                    slug, name, sort
                )).await?;
            }

            // Scenarios
            let scenarios = [
                ("rcp26", "RCP 2.6", 0),
                ("rcp60", "RCP 6.0", 1),
                ("rcp85", "RCP 8.5", 2),
            ];
            for (slug, name, sort) in &scenarios {
                db.execute_unprepared(&format!(
                    "INSERT INTO scenario (id, slug, name, sort_order) VALUES (uuid_generate_v4(), '{}', '{}', {}) ON CONFLICT (slug) DO NOTHING",
                    slug, name, sort
                )).await?;
            }

            // Variables (time-based)
            // (slug, name, abbreviation, subscript_or_empty, unit, group_name, sort_order)
            let time_variables: Vec<(&str, &str, &str, &str, &str, &str, i32)> = vec![
                ("vwc",       "Total",  "VWC", "",  "m³ ton⁻¹", "Virtual Water Content", 0),
                ("vwcb",      "Blue",   "VWC", "b", "m³ ton⁻¹", "Virtual Water Content", 1),
                ("vwcg",      "Green",  "VWC", "g", "m³ ton⁻¹", "Virtual Water Content", 2),
                ("vwcg_perc", "Green",  "VWC", "g", "%",         "Virtual Water Content", 3),
                ("vwcb_perc", "Blue",   "VWC", "b", "%",         "Virtual Water Content", 4),
                ("wf",        "Total",  "WF",  "",  "m³",        "Water Footprint",       5),
                ("wfb",       "Blue",   "WF",  "b", "m³",        "Water Footprint",       6),
                ("wfg",       "Green",  "WF",  "g", "m³",        "Water Footprint",       7),
                ("etb",       "Blue",   "ET",  "b", "mm",        "Evapotranspiration",    8),
                ("etg",       "Green",  "ET",  "g", "mm",        "Evapotranspiration",    9),
                ("rb",        "Blue",   "R",   "b", "mm",        "Renewability Rates",    10),
                ("rg",        "Green",  "R",   "g", "mm",        "Renewability Rates",    11),
                ("wdb",       "Blue",   "WD",  "b", "years",     "Water Debt",            12),
                ("wdg",       "Green",  "WD",  "g", "years",     "Water Debt",            13),
            ];
            for (slug, name, abbr, sub, unit, group, sort) in &time_variables {
                let subscript_sql = if sub.is_empty() { "NULL".to_string() } else { format!("'{}'", sub) };
                db.execute_unprepared(&format!(
                    "INSERT INTO variable (id, slug, name, abbreviation, subscript, unit, is_crop_specific, group_name, sort_order) \
                     VALUES (uuid_generate_v4(), '{}', '{}', '{}', {}, '{}', false, '{}', {}) ON CONFLICT (slug) DO NOTHING",
                    slug, name, abbr, subscript_sql, unit, group, sort
                )).await?;
            }

            // Variables (crop-specific)
            let crop_variables: Vec<(&str, &str, &str, &str, &str, i32)> = vec![
                ("mirca_area_irrigated", "Irrigated Area",  "MircaAreaIrrigated", "ha",       "Crop Specific", 20),
                ("mirca_area_total",     "Total Area",      "MircaAreaTotal",     "ha",       "Crop Specific", 21),
                ("mirca_rainfed",        "Rainfed Area",    "MircaRainfed",       "ha",       "Crop Specific", 22),
                ("yield",                "Yield",           "Yield",              "ton ha⁻¹", "Crop Specific", 23),
                ("production",           "Production",      "Production",         "ton",      "Crop Specific", 24),
            ];
            for (slug, name, abbr, unit, group, sort) in &crop_variables {
                db.execute_unprepared(&format!(
                    "INSERT INTO variable (id, slug, name, abbreviation, subscript, unit, is_crop_specific, group_name, sort_order) \
                     VALUES (uuid_generate_v4(), '{}', '{}', '{}', NULL, '{}', true, '{}', {}) ON CONFLICT (slug) DO NOTHING",
                    slug, name, abbr, unit, group, sort
                )).await?;
            }

            info!("Seeded reference tables");

            // ── Pre-flight safeguard ────────────────────────────────────────
            // Before dropping the string columns, verify that every distinct
            // non-null value in layer.<col> has a matching slug in its reference
            // table. If any value is unmatched, abort the migration with a
            // detailed error so the operator can add the missing seed entry
            // rather than silently losing data when the string column is dropped.

            let checks: [(&str, &str); 5] = [
                ("crop",          "crop"),
                ("water_model",   "water_model"),
                ("climate_model", "climate_model"),
                ("scenario",      "scenario"),
                ("variable",      "variable"),
            ];

            let mut missing: Vec<(&str, Vec<String>)> = Vec::new();
            for (col, ref_table) in checks {
                let sql = format!(
                    "SELECT DISTINCT l.{col} AS val \
                     FROM layer l \
                     WHERE l.{col} IS NOT NULL \
                       AND NOT EXISTS (SELECT 1 FROM {ref_table} r WHERE r.slug = l.{col}) \
                     ORDER BY l.{col}"
                );
                let rows = db.query_all(sea_orm::Statement::from_string(
                    sea_orm::DatabaseBackend::Postgres,
                    sql,
                )).await?;
                if !rows.is_empty() {
                    let values: Vec<String> = rows.iter()
                        .filter_map(|r| r.try_get::<String>("", "val").ok())
                        .collect();
                    missing.push((col, values));
                }
            }

            if !missing.is_empty() {
                let mut msg = String::from(
                    "Aborting migration: layer table contains values that do not \
                     exist in the corresponding reference tables. No data has been \
                     altered. Add the missing entries to the seed list in \
                     m20260413_000002_add_reference_tables.rs (`time_variables` / \
                     `crop_variables` / the `crops`/`water_models`/`climate_models`/\
                     `scenarios` arrays), or manually insert them into the reference \
                     table, then re-run the migration.\n\n\
                     Unmatched values:\n"
                );
                for (col, vals) in &missing {
                    msg.push_str(&format!(
                        "  layer.{} ({} unknown value{}):\n",
                        col,
                        vals.len(),
                        if vals.len() == 1 { "" } else { "s" }
                    ));
                    for v in vals {
                        msg.push_str(&format!("    - {}\n", v));
                    }
                }
                return Err(DbErr::Custom(msg));
            }

            info!("Pre-flight check passed: every layer string maps to a reference row");

            // ── 3. Add FK columns to layer table ────────────────────────

            manager.alter_table(
                Table::alter().table(Layer::Table)
                    .add_column(ColumnDef::new(Layer::CropId).uuid())
                    .to_owned()
            ).await?;
            manager.alter_table(
                Table::alter().table(Layer::Table)
                    .add_column(ColumnDef::new(Layer::WaterModelId).uuid())
                    .to_owned()
            ).await?;
            manager.alter_table(
                Table::alter().table(Layer::Table)
                    .add_column(ColumnDef::new(Layer::ClimateModelId).uuid())
                    .to_owned()
            ).await?;
            manager.alter_table(
                Table::alter().table(Layer::Table)
                    .add_column(ColumnDef::new(Layer::ScenarioId).uuid())
                    .to_owned()
            ).await?;
            manager.alter_table(
                Table::alter().table(Layer::Table)
                    .add_column(ColumnDef::new(Layer::VariableId).uuid())
                    .to_owned()
            ).await?;

            // ── 4. Populate FK columns from string values ───────────────

            db.execute_unprepared(
                "UPDATE layer SET crop_id = (SELECT id FROM crop WHERE slug = layer.crop) WHERE crop IS NOT NULL"
            ).await?;
            db.execute_unprepared(
                "UPDATE layer SET water_model_id = (SELECT id FROM water_model WHERE slug = layer.water_model) WHERE water_model IS NOT NULL"
            ).await?;
            db.execute_unprepared(
                "UPDATE layer SET climate_model_id = (SELECT id FROM climate_model WHERE slug = layer.climate_model) WHERE climate_model IS NOT NULL"
            ).await?;
            db.execute_unprepared(
                "UPDATE layer SET scenario_id = (SELECT id FROM scenario WHERE slug = layer.scenario) WHERE scenario IS NOT NULL"
            ).await?;
            db.execute_unprepared(
                "UPDATE layer SET variable_id = (SELECT id FROM variable WHERE slug = layer.variable) WHERE variable IS NOT NULL"
            ).await?;

            info!("Populated FK columns on layer table");

            // ── 5. Drop old unique constraint and string columns ────────

            // Drop the old unique constraint on string columns
            db.execute_unprepared(
                "ALTER TABLE layer DROP CONSTRAINT IF EXISTS layer_crop_year_variable_scenario_climate_model_water_model_key"
            ).await?;

            manager.alter_table(
                Table::alter().table(Layer::Table).drop_column(Layer::CropStr).to_owned()
            ).await?;
            manager.alter_table(
                Table::alter().table(Layer::Table).drop_column(Layer::WaterModelStr).to_owned()
            ).await?;
            manager.alter_table(
                Table::alter().table(Layer::Table).drop_column(Layer::ClimateModelStr).to_owned()
            ).await?;
            manager.alter_table(
                Table::alter().table(Layer::Table).drop_column(Layer::ScenarioStr).to_owned()
            ).await?;
            manager.alter_table(
                Table::alter().table(Layer::Table).drop_column(Layer::VariableStr).to_owned()
            ).await?;
            manager.alter_table(
                Table::alter().table(Layer::Table).drop_column(Layer::IsCropSpecific).to_owned()
            ).await?;

            // Add foreign keys
            manager.create_foreign_key(
                ForeignKey::create()
                    .name("layer_crop_id_fkey")
                    .from(Layer::Table, Layer::CropId)
                    .to(Crop::Table, Crop::Id)
                    .on_delete(ForeignKeyAction::SetNull)
                    .to_owned()
            ).await?;
            manager.create_foreign_key(
                ForeignKey::create()
                    .name("layer_water_model_id_fkey")
                    .from(Layer::Table, Layer::WaterModelId)
                    .to(WaterModel::Table, WaterModel::Id)
                    .on_delete(ForeignKeyAction::SetNull)
                    .to_owned()
            ).await?;
            manager.create_foreign_key(
                ForeignKey::create()
                    .name("layer_climate_model_id_fkey")
                    .from(Layer::Table, Layer::ClimateModelId)
                    .to(ClimateModel::Table, ClimateModel::Id)
                    .on_delete(ForeignKeyAction::SetNull)
                    .to_owned()
            ).await?;
            manager.create_foreign_key(
                ForeignKey::create()
                    .name("layer_scenario_id_fkey")
                    .from(Layer::Table, Layer::ScenarioId)
                    .to(Scenario::Table, Scenario::Id)
                    .on_delete(ForeignKeyAction::SetNull)
                    .to_owned()
            ).await?;
            manager.create_foreign_key(
                ForeignKey::create()
                    .name("layer_variable_id_fkey")
                    .from(Layer::Table, Layer::VariableId)
                    .to(Variable::Table, Variable::Id)
                    .on_delete(ForeignKeyAction::SetNull)
                    .to_owned()
            ).await?;

            // Add new unique constraint on FK columns
            manager.create_index(
                Index::create()
                    .name("layer_crop_year_variable_scenario_climate_water_fk_key")
                    .table(Layer::Table)
                    .col(Layer::CropId)
                    .col(Layer::Year)
                    .col(Layer::VariableId)
                    .col(Layer::ScenarioId)
                    .col(Layer::ClimateModelId)
                    .col(Layer::WaterModelId)
                    .unique()
                    .to_owned()
            ).await?;

            // Add indexes on FK columns
            let fk_indexes = [
                ("ix_layer_crop_id", Layer::CropId),
                ("ix_layer_water_model_id", Layer::WaterModelId),
                ("ix_layer_climate_model_id", Layer::ClimateModelId),
                ("ix_layer_scenario_id", Layer::ScenarioId),
                ("ix_layer_variable_id", Layer::VariableId),
            ];
            for (name, col) in fk_indexes {
                manager.create_index(
                    Index::create().name(name).table(Layer::Table).col(col).to_owned()
                ).await?;
            }

            // Drop old indexes on string columns
            let old_indexes = [
                "ix_layer_crop",
                "ix_layer_water_model",
                "ix_layer_climate_model",
                "ix_layer_scenario",
                "ix_layer_variable",
            ];
            for name in old_indexes {
                db.execute_unprepared(&format!("DROP INDEX IF EXISTS {}", name)).await?;
            }

            info!("Migrated layer table from string columns to FK columns");
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // This is a destructive migration — down adds back string columns but can't restore data
        let db = manager.get_connection();
        let is_postgres = manager.get_database_backend() == sea_orm::DatabaseBackend::Postgres;

        if is_postgres {
            // Drop FK constraints
            db.execute_unprepared("ALTER TABLE layer DROP CONSTRAINT IF EXISTS layer_crop_id_fkey").await?;
            db.execute_unprepared("ALTER TABLE layer DROP CONSTRAINT IF EXISTS layer_water_model_id_fkey").await?;
            db.execute_unprepared("ALTER TABLE layer DROP CONSTRAINT IF EXISTS layer_climate_model_id_fkey").await?;
            db.execute_unprepared("ALTER TABLE layer DROP CONSTRAINT IF EXISTS layer_scenario_id_fkey").await?;
            db.execute_unprepared("ALTER TABLE layer DROP CONSTRAINT IF EXISTS layer_variable_id_fkey").await?;
            db.execute_unprepared("DROP INDEX IF EXISTS layer_crop_year_variable_scenario_climate_water_fk_key").await?;

            // Add back string columns
            db.execute_unprepared("ALTER TABLE layer ADD COLUMN crop VARCHAR, ADD COLUMN water_model VARCHAR, ADD COLUMN climate_model VARCHAR, ADD COLUMN scenario VARCHAR, ADD COLUMN variable VARCHAR, ADD COLUMN is_crop_specific BOOLEAN NOT NULL DEFAULT false").await?;

            // Populate string columns from FKs
            db.execute_unprepared("UPDATE layer SET crop = (SELECT slug FROM crop WHERE id = layer.crop_id)").await?;
            db.execute_unprepared("UPDATE layer SET water_model = (SELECT slug FROM water_model WHERE id = layer.water_model_id)").await?;
            db.execute_unprepared("UPDATE layer SET climate_model = (SELECT slug FROM climate_model WHERE id = layer.climate_model_id)").await?;
            db.execute_unprepared("UPDATE layer SET scenario = (SELECT slug FROM scenario WHERE id = layer.scenario_id)").await?;
            db.execute_unprepared("UPDATE layer SET variable = (SELECT slug FROM variable WHERE id = layer.variable_id)").await?;
            db.execute_unprepared("UPDATE layer SET is_crop_specific = COALESCE((SELECT is_crop_specific FROM variable WHERE id = layer.variable_id), false)").await?;

            // Drop FK columns
            db.execute_unprepared("ALTER TABLE layer DROP COLUMN IF EXISTS crop_id, DROP COLUMN IF EXISTS water_model_id, DROP COLUMN IF EXISTS climate_model_id, DROP COLUMN IF EXISTS scenario_id, DROP COLUMN IF EXISTS variable_id").await?;
        }

        // Drop reference tables
        manager.drop_table(Table::drop().table(Variable::Table).if_exists().to_owned()).await?;
        manager.drop_table(Table::drop().table(Scenario::Table).if_exists().to_owned()).await?;
        manager.drop_table(Table::drop().table(ClimateModel::Table).if_exists().to_owned()).await?;
        manager.drop_table(Table::drop().table(WaterModel::Table).if_exists().to_owned()).await?;
        manager.drop_table(Table::drop().table(Crop::Table).if_exists().to_owned()).await?;

        Ok(())
    }
}

// ── Table identifiers ───────────────────────────────────────────────────

#[derive(DeriveIden)]
pub enum Crop {
    Table,
    Id,
    Slug,
    Name,
    SortOrder,
    Iterator,
}

#[derive(DeriveIden)]
pub enum WaterModel {
    Table,
    Id,
    Slug,
    Name,
    SortOrder,
    Iterator,
}

#[derive(DeriveIden)]
pub enum ClimateModel {
    Table,
    Id,
    Slug,
    Name,
    SortOrder,
    Iterator,
}

#[derive(DeriveIden)]
pub enum Scenario {
    Table,
    Id,
    Slug,
    Name,
    SortOrder,
    Iterator,
}

#[derive(DeriveIden)]
pub enum Variable {
    Table,
    Id,
    Slug,
    Name,
    Abbreviation,
    Subscript,
    Unit,
    IsCropSpecific,
    GroupName,
    SortOrder,
    Iterator,
}

#[derive(DeriveIden)]
pub enum Layer {
    Table,
    CropId,
    WaterModelId,
    ClimateModelId,
    ScenarioId,
    VariableId,
    Year,
    // Old string columns (for dropping)
    #[sea_orm(iden = "crop")]
    CropStr,
    #[sea_orm(iden = "water_model")]
    WaterModelStr,
    #[sea_orm(iden = "climate_model")]
    ClimateModelStr,
    #[sea_orm(iden = "scenario")]
    ScenarioStr,
    #[sea_orm(iden = "variable")]
    VariableStr,
    IsCropSpecific,
}
