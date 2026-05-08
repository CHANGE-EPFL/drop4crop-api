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
                    .add_column(ColumnDef::new(Project::Extent).json_binary())
                    .drop_column(Project::UseCardAsExtent)
                    .drop_column(Project::Latitude)
                    .drop_column(Project::Longitude)
                    .drop_column(Project::ZoomLevel)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Project::Table)
                    .add_column(
                        ColumnDef::new(Project::UseCardAsExtent)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .add_column(
                        ColumnDef::new(Project::Latitude)
                            .double()
                            .not_null()
                            .default(0.0),
                    )
                    .add_column(
                        ColumnDef::new(Project::Longitude)
                            .double()
                            .not_null()
                            .default(0.0),
                    )
                    .add_column(
                        ColumnDef::new(Project::ZoomLevel)
                            .integer()
                            .not_null()
                            .default(2),
                    )
                    .drop_column(Project::Extent)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Project {
    Table,
    Extent,
    UseCardAsExtent,
    Latitude,
    Longitude,
    ZoomLevel,
}
