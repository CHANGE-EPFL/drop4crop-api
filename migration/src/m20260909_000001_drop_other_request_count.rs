use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const FOUR_COUNTER_TOTALS: &str = r#"
    CREATE OR REPLACE FUNCTION update_layer_total_views()
    RETURNS TRIGGER AS $$
    BEGIN
        IF TG_OP = 'DELETE' THEN
            UPDATE layer
            SET total_views = COALESCE((
                SELECT SUM(xyz_tile_count + cog_download_count + pixel_query_count + stac_request_count)
                FROM layer_statistics
                WHERE layer_id = OLD.layer_id
            ), 0)
            WHERE id = OLD.layer_id;
            RETURN OLD;
        ELSE
            UPDATE layer
            SET total_views = COALESCE((
                SELECT SUM(xyz_tile_count + cog_download_count + pixel_query_count + stac_request_count)
                FROM layer_statistics
                WHERE layer_id = NEW.layer_id
            ), 0)
            WHERE id = NEW.layer_id;
            RETURN NEW;
        END IF;
    END;
    $$ LANGUAGE plpgsql;
"#;

const FIVE_COUNTER_TOTALS: &str = r#"
    CREATE OR REPLACE FUNCTION update_layer_total_views()
    RETURNS TRIGGER AS $$
    BEGIN
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_trigger_excludes_other_requests() {
        assert!(!FOUR_COUNTER_TOTALS.contains("other_request_count"));
        assert!(FOUR_COUNTER_TOTALS.contains("stac_request_count"));
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(FOUR_COUNTER_TOTALS).await?;

        // Counts classified as "other" are intentionally discarded.
        db.execute_unprepared("ALTER TABLE layer_statistics DROP COLUMN other_request_count;")
            .await?;
        db.execute_unprepared(
            r#"
            UPDATE layer
            SET total_views = COALESCE((
                SELECT SUM(xyz_tile_count + cog_download_count + pixel_query_count + stac_request_count)
                FROM layer_statistics
                WHERE layer_statistics.layer_id = layer.id
            ), 0);
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "ALTER TABLE layer_statistics ADD COLUMN other_request_count INTEGER NOT NULL DEFAULT 0;",
        )
        .await?;
        db.execute_unprepared(FIVE_COUNTER_TOTALS).await?;

        Ok(())
    }
}
