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
                    .add_column(ColumnDef::new(Project::License).text())
                    .add_column(ColumnDef::new(Project::Providers).json_binary())
                    .add_column(ColumnDef::new(Project::Keywords).json_binary())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Project::Table)
                    .drop_column(Project::License)
                    .drop_column(Project::Providers)
                    .drop_column(Project::Keywords)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Project {
    Table,
    License,
    Providers,
    Keywords,
}
