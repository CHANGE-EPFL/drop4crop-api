use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut table = Table::create()
            .table(ShowcaseItem::Table)
            .if_not_exists()
            .col(ColumnDef::new(ShowcaseItem::ProjectId).uuid().not_null())
            .col(ColumnDef::new(ShowcaseItem::LayerId).uuid().not_null())
            .col(ColumnDef::new(ShowcaseItem::Title).string().not_null())
            .col(ColumnDef::new(ShowcaseItem::Description).text())
            .col(ColumnDef::new(ShowcaseItem::SortOrder).integer().not_null().default(0))
            .col(ColumnDef::new(ShowcaseItem::Enabled).boolean().not_null().default(true))
            .foreign_key(
                ForeignKey::create()
                    .name("showcase_item_project_id_fkey")
                    .from(ShowcaseItem::Table, ShowcaseItem::ProjectId)
                    .to(Project::Table, Project::Id)
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .foreign_key(
                ForeignKey::create()
                    .name("showcase_item_layer_id_fkey")
                    .from(ShowcaseItem::Table, ShowcaseItem::LayerId)
                    .to(Layer::Table, Layer::Id)
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .to_owned();

        match manager.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres | sea_orm::DatabaseBackend::Sqlite => {
                table.col(ColumnDef::new(ShowcaseItem::Id).uuid().not_null().unique_key());
            }
            _ => return Err(DbErr::Custom("Unsupported database backend".to_string())),
        }

        table.col(
            ColumnDef::new(ShowcaseItem::Iterator)
                .integer()
                .not_null()
                .auto_increment()
                .primary_key(),
        );

        manager.create_table(table).await?;

        // Indexes
        manager.create_index(
            Index::create()
                .name("ix_showcase_item_project_id")
                .table(ShowcaseItem::Table)
                .col(ShowcaseItem::ProjectId)
                .to_owned(),
        ).await?;

        manager.create_index(
            Index::create()
                .name("ix_showcase_item_id")
                .table(ShowcaseItem::Table)
                .col(ShowcaseItem::Id)
                .to_owned(),
        ).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ShowcaseItem::Table).if_exists().to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum ShowcaseItem {
    Table,
    Id,
    ProjectId,
    LayerId,
    Title,
    Description,
    SortOrder,
    Enabled,
    Iterator,
}

#[derive(DeriveIden)]
enum Project {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Layer {
    Table,
    Id,
}
