use sea_orm_migration::prelude::*;
use tracing::info;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SiteSettings::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SiteSettings::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(SiteSettings::GlobeLayerId).uuid())
                    .col(ColumnDef::new(SiteSettings::GlobeStyleId).uuid())
                    .to_owned(),
            )
            .await?;

        manager
            .create_foreign_key(
                ForeignKey::create()
                    .name("site_settings_globe_layer_id_fkey")
                    .from(SiteSettings::Table, SiteSettings::GlobeLayerId)
                    .to(Layer::Table, Layer::Id)
                    .on_delete(ForeignKeyAction::SetNull)
                    .to_owned(),
            )
            .await?;

        manager
            .create_foreign_key(
                ForeignKey::create()
                    .name("site_settings_globe_style_id_fkey")
                    .from(SiteSettings::Table, SiteSettings::GlobeStyleId)
                    .to(Style::Table, Style::Id)
                    .on_delete(ForeignKeyAction::SetNull)
                    .to_owned(),
            )
            .await?;

        // Seed a single row so the singleton always exists.
        let db = manager.get_connection();
        db.execute_unprepared(
            "INSERT INTO site_settings (id) VALUES ('00000000-0000-0000-0000-000000000001')",
        )
        .await?;

        info!("Created site_settings table with globe overlay columns");
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SiteSettings::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum SiteSettings {
    Table,
    Id,
    GlobeLayerId,
    GlobeStyleId,
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
