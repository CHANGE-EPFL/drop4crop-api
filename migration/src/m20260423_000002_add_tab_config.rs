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
                    .add_column(ColumnDef::new(Project::TabConfig).json_binary())
                    .to_owned(),
            )
            .await?;

        // Change the default for variable.has_time from true to false so new
        // variables are timeless unless explicitly marked otherwise.
        manager
            .alter_table(
                Table::alter()
                    .table(Variable::Table)
                    .modify_column(
                        ColumnDef::new(Variable::HasTime)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Variable::Table)
                    .modify_column(
                        ColumnDef::new(Variable::HasTime)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Project::Table)
                    .drop_column(Project::TabConfig)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Project {
    Table,
    TabConfig,
}

#[derive(DeriveIden)]
enum Variable {
    Table,
    HasTime,
}
