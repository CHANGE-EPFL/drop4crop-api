use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Project::Table)
                    .add_column(ColumnDef::new(Project::Citation).json_binary())
                    .add_column(ColumnDef::new(Project::UnavailableMessage).text())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Project::Table)
                    .drop_column(Project::Citation)
                    .drop_column(Project::UnavailableMessage)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Project {
    Table,
    Citation,
    UnavailableMessage,
}
