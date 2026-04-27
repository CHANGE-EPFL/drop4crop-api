use sea_orm_migration::prelude::*;
use tracing::info;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Project::Table)
                    .add_column(ColumnDef::new(Project::CardLayerId).uuid())
                    .add_column(ColumnDef::new(Project::CardStyleId).uuid())
                    .to_owned(),
            )
            .await?;

        manager
            .create_foreign_key(
                ForeignKey::create()
                    .name("project_card_layer_id_fkey")
                    .from(Project::Table, Project::CardLayerId)
                    .to(Layer::Table, Layer::Id)
                    .on_delete(ForeignKeyAction::SetNull)
                    .to_owned(),
            )
            .await?;

        manager
            .create_foreign_key(
                ForeignKey::create()
                    .name("project_card_style_id_fkey")
                    .from(Project::Table, Project::CardStyleId)
                    .to(Style::Table, Style::Id)
                    .on_delete(ForeignKeyAction::SetNull)
                    .to_owned(),
            )
            .await?;

        info!("Added card_layer_id and card_style_id columns to project table");
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "ALTER TABLE project DROP CONSTRAINT IF EXISTS project_card_layer_id_fkey",
        )
        .await?;
        db.execute_unprepared(
            "ALTER TABLE project DROP CONSTRAINT IF EXISTS project_card_style_id_fkey",
        )
        .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Project::Table)
                    .drop_column(Project::CardLayerId)
                    .drop_column(Project::CardStyleId)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Project {
    Table,
    CardLayerId,
    CardStyleId,
}

#[derive(DeriveIden)]
enum Layer {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Style {
    Table,
    Id,
}
