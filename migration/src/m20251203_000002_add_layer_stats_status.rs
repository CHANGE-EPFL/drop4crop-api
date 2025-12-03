use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Add stats_status JSON field to track recalculation status
        manager
            .alter_table(
                Table::alter()
                    .table(Layer::Table)
                    .add_column(
                        ColumnDef::new(Layer::StatsStatus)
                            .json()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Add file_size field to track the size of the file in S3
        manager
            .alter_table(
                Table::alter()
                    .table(Layer::Table)
                    .add_column(
                        ColumnDef::new(Layer::FileSize)
                            .big_integer()
                            .null(),
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
                    .table(Layer::Table)
                    .drop_column(Layer::StatsStatus)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Layer::Table)
                    .drop_column(Layer::FileSize)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Layer {
    Table,
    StatsStatus,
    FileSize,
}
