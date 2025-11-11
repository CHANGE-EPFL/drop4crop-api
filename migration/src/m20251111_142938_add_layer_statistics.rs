use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(LayerStatistics::Table)
                    .if_not_exists()
                    .col(uuid(LayerStatistics::Id).primary_key())
                    .col(uuid(LayerStatistics::LayerId).not_null())
                    .col(date(LayerStatistics::StatDate).not_null())
                    .col(timestamp_with_time_zone(LayerStatistics::LastAccessedAt).not_null())
                    .col(integer(LayerStatistics::XyzTileCount).default(0).not_null())
                    .col(integer(LayerStatistics::CogDownloadCount).default(0).not_null())
                    .col(integer(LayerStatistics::PixelQueryCount).default(0).not_null())
                    .col(integer(LayerStatistics::StacRequestCount).default(0).not_null())
                    .col(integer(LayerStatistics::OtherRequestCount).default(0).not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_layer_statistics_layer")
                            .from(LayerStatistics::Table, LayerStatistics::LayerId)
                            .to(Layer::Table, Layer::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Create composite unique index on layer_id and stat_date
        manager
            .create_index(
                Index::create()
                    .name("idx_layer_statistics_layer_date")
                    .table(LayerStatistics::Table)
                    .col(LayerStatistics::LayerId)
                    .col(LayerStatistics::StatDate)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Create index on layer_id for foreign key lookups
        manager
            .create_index(
                Index::create()
                    .name("idx_layer_statistics_layer_id")
                    .table(LayerStatistics::Table)
                    .col(LayerStatistics::LayerId)
                    .to_owned(),
            )
            .await?;

        // Create index on stat_date for date range queries
        manager
            .create_index(
                Index::create()
                    .name("idx_layer_statistics_stat_date")
                    .table(LayerStatistics::Table)
                    .col(LayerStatistics::StatDate)
                    .to_owned(),
            )
            .await?;

        // Create index on last_accessed_at for sorting by recency
        manager
            .create_index(
                Index::create()
                    .name("idx_layer_statistics_last_accessed")
                    .table(LayerStatistics::Table)
                    .col(LayerStatistics::LastAccessedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(LayerStatistics::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum LayerStatistics {
    Table,
    Id,
    LayerId,
    StatDate,
    LastAccessedAt,
    XyzTileCount,
    CogDownloadCount,
    PixelQueryCount,
    StacRequestCount,
    OtherRequestCount,
}

#[derive(DeriveIden)]
enum Layer {
    Table,
    Id,
}
