use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut project_table = Table::create()
            .table(Project::Table)
            .if_not_exists()
            .col(ColumnDef::new(Project::Slug).string().not_null().unique_key())
            .col(ColumnDef::new(Project::Title).string().not_null())
            .col(ColumnDef::new(Project::Description).text())
            .col(
                ColumnDef::new(Project::Latitude)
                    .double()
                    .not_null()
                    .default(20.0),
            )
            .col(
                ColumnDef::new(Project::Longitude)
                    .double()
                    .not_null()
                    .default(0.0),
            )
            .col(
                ColumnDef::new(Project::ZoomLevel)
                    .integer()
                    .not_null()
                    .default(2),
            )
            .col(
                ColumnDef::new(Project::Enabled)
                    .boolean()
                    .not_null()
                    .default(true),
            )
            .col(
                ColumnDef::new(Project::SortOrder)
                    .integer()
                    .not_null()
                    .default(0),
            )
            .to_owned();

        // Add ID column based on database backend
        match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres => {
                project_table.col(ColumnDef::new(Project::Id).uuid().not_null().unique_key());
            }
            sea_orm::DatabaseBackend::Sqlite => {
                project_table.col(ColumnDef::new(Project::Id).uuid().not_null().unique_key());
            }
            _ => {
                return Err(DbErr::Custom("Unsupported database backend".to_string()));
            }
        }

        // Add iterator column for primary key
        project_table.col(
            ColumnDef::new(Project::Iterator)
                .integer()
                .not_null()
                .auto_increment()
                .primary_key(),
        );

        manager.create_table(project_table).await?;

        // Create indexes
        manager
            .create_index(
                Index::create()
                    .name("ix_project_slug")
                    .table(Project::Table)
                    .col(Project::Slug)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("ix_project_id")
                    .table(Project::Table)
                    .col(Project::Id)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Project::Table).if_exists().to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
pub enum Project {
    Table,
    Id,
    Slug,
    Title,
    Description,
    Latitude,
    Longitude,
    ZoomLevel,
    Enabled,
    SortOrder,
    Iterator,
}
