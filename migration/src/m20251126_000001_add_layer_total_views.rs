use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. Add total_views column to layer table
        manager
            .alter_table(
                Table::alter()
                    .table(Layer::Table)
                    .add_column(
                        ColumnDef::new(Layer::TotalViews)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;

        // 2. Create trigger function to update total_views
        let create_function = r#"
            CREATE OR REPLACE FUNCTION update_layer_total_views()
            RETURNS TRIGGER AS $$
            BEGIN
                -- Update the layer's total_views based on the affected layer_id
                IF TG_OP = 'DELETE' THEN
                    UPDATE layer
                    SET total_views = COALESCE((
                        SELECT SUM(xyz_tile_count + cog_download_count + pixel_query_count + stac_request_count + other_request_count)
                        FROM layer_statistics
                        WHERE layer_id = OLD.layer_id
                    ), 0)
                    WHERE id = OLD.layer_id;
                    RETURN OLD;
                ELSE
                    UPDATE layer
                    SET total_views = COALESCE((
                        SELECT SUM(xyz_tile_count + cog_download_count + pixel_query_count + stac_request_count + other_request_count)
                        FROM layer_statistics
                        WHERE layer_id = NEW.layer_id
                    ), 0)
                    WHERE id = NEW.layer_id;
                    RETURN NEW;
                END IF;
            END;
            $$ LANGUAGE plpgsql;
        "#;
        manager.get_connection().execute_unprepared(create_function).await?;

        // 3. Create triggers on layer_statistics table
        let create_triggers = r#"
            CREATE TRIGGER trigger_layer_stats_insert
                AFTER INSERT ON layer_statistics
                FOR EACH ROW
                EXECUTE FUNCTION update_layer_total_views();

            CREATE TRIGGER trigger_layer_stats_update
                AFTER UPDATE ON layer_statistics
                FOR EACH ROW
                EXECUTE FUNCTION update_layer_total_views();

            CREATE TRIGGER trigger_layer_stats_delete
                AFTER DELETE ON layer_statistics
                FOR EACH ROW
                EXECUTE FUNCTION update_layer_total_views();
        "#;
        manager.get_connection().execute_unprepared(create_triggers).await?;

        // 4. Backfill existing data
        let backfill = r#"
            UPDATE layer
            SET total_views = COALESCE((
                SELECT SUM(xyz_tile_count + cog_download_count + pixel_query_count + stac_request_count + other_request_count)
                FROM layer_statistics
                WHERE layer_statistics.layer_id = layer.id
            ), 0);
        "#;
        manager.get_connection().execute_unprepared(backfill).await?;

        // 5. Add index for sorting performance
        manager
            .create_index(
                Index::create()
                    .name("idx_layer_total_views")
                    .table(Layer::Table)
                    .col(Layer::TotalViews)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop triggers
        let drop_triggers = r#"
            DROP TRIGGER IF EXISTS trigger_layer_stats_insert ON layer_statistics;
            DROP TRIGGER IF EXISTS trigger_layer_stats_update ON layer_statistics;
            DROP TRIGGER IF EXISTS trigger_layer_stats_delete ON layer_statistics;
        "#;
        manager.get_connection().execute_unprepared(drop_triggers).await?;

        // Drop function
        let drop_function = "DROP FUNCTION IF EXISTS update_layer_total_views();";
        manager.get_connection().execute_unprepared(drop_function).await?;

        // Drop index
        manager
            .drop_index(Index::drop().name("idx_layer_total_views").to_owned())
            .await?;

        // Drop column
        manager
            .alter_table(
                Table::alter()
                    .table(Layer::Table)
                    .drop_column(Layer::TotalViews)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Layer {
    Table,
    TotalViews,
}
