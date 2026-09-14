use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Read from the GeoTIFF when it is uploaded or its statistics are
        // recalculated, and served as proj:shape, spatial_resolution and
        // data_type in STAC. Null until the layer is next processed.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE layer
                    ADD COLUMN raster_width INTEGER,
                    ADD COLUMN raster_height INTEGER,
                    ADD COLUMN raster_resolution DOUBLE PRECISION,
                    ADD COLUMN raster_data_type VARCHAR;",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE layer
                    DROP COLUMN raster_width,
                    DROP COLUMN raster_height,
                    DROP COLUMN raster_resolution,
                    DROP COLUMN raster_data_type;",
            )
            .await?;

        Ok(())
    }
}
