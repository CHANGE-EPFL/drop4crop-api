use sea_orm_migration::prelude::*;
use tracing::info;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let is_postgres = manager.get_database_backend() == sea_orm::DatabaseBackend::Postgres;

        // ── 1. Add project_id FK to layer table ─────────────────────────

        manager.alter_table(
            Table::alter()
                .table(Layer::Table)
                .add_column(ColumnDef::new(Layer::ProjectId).uuid())
                .to_owned()
        ).await?;

        manager.create_foreign_key(
            ForeignKey::create()
                .name("layer_project_id_fkey")
                .from(Layer::Table, Layer::ProjectId)
                .to(Project::Table, Project::Id)
                .on_delete(ForeignKeyAction::SetNull)
                .to_owned()
        ).await?;

        manager.create_index(
            Index::create()
                .name("ix_layer_project_id")
                .table(Layer::Table)
                .col(Layer::ProjectId)
                .to_owned()
        ).await?;

        info!("Added project_id FK to layer table");

        // ── 2. Create junction tables ───────────────────────────────────

        // project_crop
        manager.create_table(
            Table::create()
                .table(ProjectCrop::Table)
                .if_not_exists()
                .col(ColumnDef::new(ProjectCrop::ProjectId).uuid().not_null())
                .col(ColumnDef::new(ProjectCrop::CropId).uuid().not_null())
                .primary_key(
                    Index::create()
                        .col(ProjectCrop::ProjectId)
                        .col(ProjectCrop::CropId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("project_crop_project_id_fkey")
                        .from(ProjectCrop::Table, ProjectCrop::ProjectId)
                        .to(Project::Table, Project::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("project_crop_crop_id_fkey")
                        .from(ProjectCrop::Table, ProjectCrop::CropId)
                        .to(Crop::Table, Crop::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned()
        ).await?;

        // project_water_model
        manager.create_table(
            Table::create()
                .table(ProjectWaterModel::Table)
                .if_not_exists()
                .col(ColumnDef::new(ProjectWaterModel::ProjectId).uuid().not_null())
                .col(ColumnDef::new(ProjectWaterModel::WaterModelId).uuid().not_null())
                .primary_key(
                    Index::create()
                        .col(ProjectWaterModel::ProjectId)
                        .col(ProjectWaterModel::WaterModelId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("project_water_model_project_id_fkey")
                        .from(ProjectWaterModel::Table, ProjectWaterModel::ProjectId)
                        .to(Project::Table, Project::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("project_water_model_water_model_id_fkey")
                        .from(ProjectWaterModel::Table, ProjectWaterModel::WaterModelId)
                        .to(WaterModel::Table, WaterModel::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned()
        ).await?;

        // project_climate_model
        manager.create_table(
            Table::create()
                .table(ProjectClimateModel::Table)
                .if_not_exists()
                .col(ColumnDef::new(ProjectClimateModel::ProjectId).uuid().not_null())
                .col(ColumnDef::new(ProjectClimateModel::ClimateModelId).uuid().not_null())
                .primary_key(
                    Index::create()
                        .col(ProjectClimateModel::ProjectId)
                        .col(ProjectClimateModel::ClimateModelId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("project_climate_model_project_id_fkey")
                        .from(ProjectClimateModel::Table, ProjectClimateModel::ProjectId)
                        .to(Project::Table, Project::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("project_climate_model_climate_model_id_fkey")
                        .from(ProjectClimateModel::Table, ProjectClimateModel::ClimateModelId)
                        .to(ClimateModel::Table, ClimateModel::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned()
        ).await?;

        // project_scenario
        manager.create_table(
            Table::create()
                .table(ProjectScenario::Table)
                .if_not_exists()
                .col(ColumnDef::new(ProjectScenario::ProjectId).uuid().not_null())
                .col(ColumnDef::new(ProjectScenario::ScenarioId).uuid().not_null())
                .primary_key(
                    Index::create()
                        .col(ProjectScenario::ProjectId)
                        .col(ProjectScenario::ScenarioId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("project_scenario_project_id_fkey")
                        .from(ProjectScenario::Table, ProjectScenario::ProjectId)
                        .to(Project::Table, Project::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("project_scenario_scenario_id_fkey")
                        .from(ProjectScenario::Table, ProjectScenario::ScenarioId)
                        .to(Scenario::Table, Scenario::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned()
        ).await?;

        // project_variable
        manager.create_table(
            Table::create()
                .table(ProjectVariable::Table)
                .if_not_exists()
                .col(ColumnDef::new(ProjectVariable::ProjectId).uuid().not_null())
                .col(ColumnDef::new(ProjectVariable::VariableId).uuid().not_null())
                .primary_key(
                    Index::create()
                        .col(ProjectVariable::ProjectId)
                        .col(ProjectVariable::VariableId),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("project_variable_project_id_fkey")
                        .from(ProjectVariable::Table, ProjectVariable::ProjectId)
                        .to(Project::Table, Project::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("project_variable_variable_id_fkey")
                        .from(ProjectVariable::Table, ProjectVariable::VariableId)
                        .to(Variable::Table, Variable::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned()
        ).await?;

        info!("Created junction tables: project_crop, project_water_model, project_climate_model, project_scenario, project_variable");

        // ── 3. Seed junction tables from existing layer data ────────────

        if is_postgres {
            // Auto-populate junction tables from layers that have a project_id
            db.execute_unprepared(
                "INSERT INTO project_crop (project_id, crop_id) \
                 SELECT DISTINCT l.project_id, l.crop_id FROM layer l \
                 WHERE l.project_id IS NOT NULL AND l.crop_id IS NOT NULL \
                 ON CONFLICT DO NOTHING"
            ).await?;
            db.execute_unprepared(
                "INSERT INTO project_water_model (project_id, water_model_id) \
                 SELECT DISTINCT l.project_id, l.water_model_id FROM layer l \
                 WHERE l.project_id IS NOT NULL AND l.water_model_id IS NOT NULL \
                 ON CONFLICT DO NOTHING"
            ).await?;
            db.execute_unprepared(
                "INSERT INTO project_climate_model (project_id, climate_model_id) \
                 SELECT DISTINCT l.project_id, l.climate_model_id FROM layer l \
                 WHERE l.project_id IS NOT NULL AND l.climate_model_id IS NOT NULL \
                 ON CONFLICT DO NOTHING"
            ).await?;
            db.execute_unprepared(
                "INSERT INTO project_scenario (project_id, scenario_id) \
                 SELECT DISTINCT l.project_id, l.scenario_id FROM layer l \
                 WHERE l.project_id IS NOT NULL AND l.scenario_id IS NOT NULL \
                 ON CONFLICT DO NOTHING"
            ).await?;
            db.execute_unprepared(
                "INSERT INTO project_variable (project_id, variable_id) \
                 SELECT DISTINCT l.project_id, l.variable_id FROM layer l \
                 WHERE l.project_id IS NOT NULL AND l.variable_id IS NOT NULL \
                 ON CONFLICT DO NOTHING"
            ).await?;

            info!("Seeded junction tables from existing layer data");
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop junction tables
        manager.drop_table(Table::drop().table(ProjectVariable::Table).if_exists().to_owned()).await?;
        manager.drop_table(Table::drop().table(ProjectScenario::Table).if_exists().to_owned()).await?;
        manager.drop_table(Table::drop().table(ProjectClimateModel::Table).if_exists().to_owned()).await?;
        manager.drop_table(Table::drop().table(ProjectWaterModel::Table).if_exists().to_owned()).await?;
        manager.drop_table(Table::drop().table(ProjectCrop::Table).if_exists().to_owned()).await?;

        // Drop project_id FK from layer
        let db = manager.get_connection();
        db.execute_unprepared("ALTER TABLE layer DROP CONSTRAINT IF EXISTS layer_project_id_fkey").await?;
        db.execute_unprepared("DROP INDEX IF EXISTS ix_layer_project_id").await?;

        manager.alter_table(
            Table::alter()
                .table(Layer::Table)
                .drop_column(Layer::ProjectId)
                .to_owned()
        ).await?;

        Ok(())
    }
}

// ── Table identifiers ───────────────────────────────────────────────────

#[derive(DeriveIden)]
enum Layer {
    Table,
    ProjectId,
}

#[derive(DeriveIden)]
enum Project {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Crop {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum WaterModel {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum ClimateModel {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Scenario {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Variable {
    Table,
    Id,
}

#[derive(DeriveIden)]
#[sea_orm(iden = "project_crop")]
enum ProjectCrop {
    Table,
    ProjectId,
    CropId,
}

#[derive(DeriveIden)]
#[sea_orm(iden = "project_water_model")]
enum ProjectWaterModel {
    Table,
    ProjectId,
    WaterModelId,
}

#[derive(DeriveIden)]
#[sea_orm(iden = "project_climate_model")]
enum ProjectClimateModel {
    Table,
    ProjectId,
    ClimateModelId,
}

#[derive(DeriveIden)]
#[sea_orm(iden = "project_scenario")]
enum ProjectScenario {
    Table,
    ProjectId,
    ScenarioId,
}

#[derive(DeriveIden)]
#[sea_orm(iden = "project_variable")]
enum ProjectVariable {
    Table,
    ProjectId,
    VariableId,
}
