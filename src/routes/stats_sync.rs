use anyhow::Result;
use redis::AsyncCommands;
use sea_orm::{DatabaseConnection, EntityTrait, Set};
use std::collections::HashMap;
use tokio::time::{Duration, interval};
use tracing::{error, info, warn};
use crate::config::Config;

/// Spawns a background task that syncs statistics from Redis to PostgreSQL every 30 seconds.
/// Uses distributed locking to ensure only one instance runs the sync at a time.
pub fn spawn_stats_sync_task(db: DatabaseConnection, config: Config) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(30)); // 30 seconds
        let instance_id = uuid::Uuid::new_v4().to_string();

        loop {
            ticker.tick().await;

            match sync_stats_to_db(&db, &config, &instance_id).await {
                Ok(synced_count) => {
                    if synced_count > 0 {
                        info!(synced_count, "Synced statistics to PostgreSQL");
                    }
                }
                Err(e) => {
                    error!(
                        error = %e,
                        "Stats sync failed, will retry in 30 seconds"
                    );
                }
            }
        }
    });
}

/// Attempts to sync statistics from Redis to PostgreSQL with distributed locking.
async fn sync_stats_to_db(db: &DatabaseConnection, config: &Config, instance_id: &str) -> Result<usize> {
    let redis_client = super::tiles::cache::get_redis_client(config);
    let mut con = redis_client.get_multiplexed_async_connection().await?;

    // Try to acquire distributed lock
    let lock_key = format!("{}-{}/stats:sync_lock", config.app_name, config.deployment);
    let lock_acquired: bool = redis::cmd("SET")
        .arg(&lock_key)
        .arg(instance_id)
        .arg("NX")
        .arg("EX")
        .arg(60) // 60 second TTL (longer than sync interval)
        .query_async(&mut con)
        .await
        .unwrap_or(false);

    if !lock_acquired {
        // Another instance is handling the sync
        return Ok(0);
    }

    // Check if it's been at least 30 seconds since last sync
    let last_sync_key = format!(
        "{}-{}/stats:last_sync_time",
        config.app_name, config.deployment
    );
    let last_sync: Option<String> = con.get(&last_sync_key).await?;

    if let Some(last_sync_str) = last_sync
        && let Ok(last_sync_time) = chrono::DateTime::parse_from_rfc3339(&last_sync_str)
    {
        let elapsed =
            chrono::Utc::now().signed_duration_since(last_sync_time.with_timezone(&chrono::Utc));
        if elapsed < chrono::Duration::seconds(30) {
            // Too soon, skip this sync
            return Ok(0);
        }
    }

    // Scan for all stats keys
    let stats_pattern = format!("{}-{}/stats:*", config.app_name, config.deployment);
    let keys = counter_keys(scan_keys(&mut con, &stats_pattern).await?, config);

    if keys.is_empty() {
        // No stats to sync
        let _: () = con
            .set(&last_sync_key, chrono::Utc::now().to_rfc3339())
            .await?;
        return Ok(0);
    }

    // Parse keys and aggregate statistics
    let mut stats_map: HashMap<(String, String), StatsCounter> = HashMap::new();

    for key in &keys {
        if let Some((date, layer_id, stat_type)) = parse_stats_key(key, config) {
            // GETDEL so a request incrementing this counter after the read starts a fresh
            // key for the next tick rather than landing inside a set about to be deleted.
            let count: i64 = redis::cmd("GETDEL")
                .arg(key)
                .query_async(&mut con)
                .await
                .unwrap_or(0);
            let entry = stats_map
                .entry((layer_id.clone(), date.clone()))
                .or_insert_with(|| StatsCounter::new(layer_id.clone(), date.clone()));

            match stat_type.as_str() {
                "xyz" => entry.xyz_tile_count = add_count(entry.xyz_tile_count, count),
                "cog" => entry.cog_download_count = add_count(entry.cog_download_count, count),
                "pixel" => entry.pixel_query_count = add_count(entry.pixel_query_count, count),
                "stac" => entry.stac_request_count = add_count(entry.stac_request_count, count),
                "hit" => entry.cache_hit_count = add_count(entry.cache_hit_count, count),
                "miss" => entry.cache_miss_count = add_count(entry.cache_miss_count, count),
                _ => {}
            }
        }
    }

    // Write to database with UPSERT. The counts are already out of Redis, so anything the
    // write could not apply has to go back or it is lost.
    let (synced_count, unwritten) = write_stats_to_db(db, stats_map).await;

    for (key, count) in restore_increments(&unwritten, config) {
        if let Err(e) = redis::cmd("INCRBY")
            .arg(&key)
            .arg(count)
            .query_async::<i64>(&mut con)
            .await
        {
            error!(key, count, error = %e, "Failed to restore unwritten stats count");
        }
    }

    // Update last sync time
    let _: () = con
        .set(&last_sync_key, chrono::Utc::now().to_rfc3339())
        .await?;

    // Release the distributed lock (TTL is just a safety net for crashes)
    let _: () = redis::cmd("DEL")
        .arg(&lock_key)
        .query_async(&mut con)
        .await
        .unwrap_or(()); // Ignore errors on lock release

    Ok(synced_count)
}

/// Scans Redis for keys matching the pattern.
/// Adds a counter to a stored total, holding at the ceiling rather than
/// wrapping. The statistics columns are `i32` and the Redis counters are `i64`.
fn add_count(stored: i32, counted: i64) -> i32 {
    let counted = counted.clamp(0, i64::from(i32::MAX)) as i32;
    let total = stored.saturating_add(counted);
    if total == i32::MAX && stored != i32::MAX {
        warn!(stored, counted, "Statistics counter held at the column ceiling");
    }
    total
}

async fn scan_keys(
    con: &mut redis::aio::MultiplexedConnection,
    pattern: &str,
) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    let mut cursor = 0u64;

    loop {
        let (new_cursor, batch): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(100)
            .query_async(con)
            .await?;

        keys.extend(batch);
        cursor = new_cursor;

        if cursor == 0 {
            break;
        }
    }

    Ok(keys)
}

/// The scanned keys that hold counters. The counter pattern also matches the keys the
/// sync manages itself, and those must survive the sync that found them.
fn counter_keys(keys: Vec<String>, config: &Config) -> Vec<String> {
    keys.into_iter()
        .filter(|key| parse_stats_key(key, config).is_some())
        .collect()
}

/// Redis increments that put back the counts a write did not apply.
fn restore_increments(unwritten: &[StatsCounter], config: &Config) -> Vec<(String, i64)> {
    let prefix = format!("{}-{}", config.app_name, config.deployment);
    let mut increments = Vec::new();

    for counter in unwritten {
        for (stat_type, count) in [
            ("xyz", counter.xyz_tile_count),
            ("cog", counter.cog_download_count),
            ("pixel", counter.pixel_query_count),
            ("stac", counter.stac_request_count),
            ("hit", counter.cache_hit_count),
            ("miss", counter.cache_miss_count),
        ] {
            if count != 0 {
                increments.push((
                    format!("{}/stats:{}:{}:{}", prefix, counter.date, counter.layer_id, stat_type),
                    i64::from(count),
                ));
            }
        }
    }

    increments
}

/// Parses a stats key and extracts the date, layer_id, and stat_type.
/// Format: {app}-{deploy}/stats:{YYYY-MM-DD}:{layer_id}:{type}
fn parse_stats_key(key: &str, config: &Config) -> Option<(String, String, String)> {
    let prefix = format!("{}-{}/stats:", config.app_name, config.deployment);
    let rest = key.strip_prefix(&prefix)?;
    let parts: Vec<&str> = rest.splitn(3, ':').collect();

    if parts.len() == 3 {
        Some((
            parts[0].to_string(),
            parts[1].to_string(),
            parts[2].to_string(),
        ))
    } else {
        None
    }
}

/// Writes aggregated statistics to the database using UPSERT.
/// Returns how many layer-days were written, and the counters that were not.
async fn write_stats_to_db(
    db: &DatabaseConnection,
    stats_map: HashMap<(String, String), StatsCounter>,
) -> (usize, Vec<StatsCounter>) {
    use sea_orm::{ColumnTrait, QueryFilter};

    let mut synced_count = 0;
    let mut unwritten = Vec::new();

    for ((layer_identifier, date_str), stats) in stats_map {
        let layer_record = find_layer(db, &parse_layer_identifier(&layer_identifier)).await;

        let layer_record = match layer_record {
            Ok(Some(record)) => record,
            Ok(None) => {
                error!(layer_identifier, "Layer not found during stats sync");
                unwritten.push(stats);
                continue;
            }
            Err(e) => {
                error!(layer_identifier, error = %e, "Failed to resolve layer during stats sync");
                unwritten.push(stats);
                continue;
            }
        };

        let layer_id = layer_record.id;
        let Ok(stat_date) = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") else {
            error!(date_str, "Unparseable stat date during stats sync");
            unwritten.push(stats);
            continue;
        };

        // Check if record exists
        use crate::routes::admin::db::layer_statistics as stats_entity;

        let existing = match stats_entity::Entity::find()
            .filter(stats_entity::Column::LayerId.eq(layer_id))
            .filter(stats_entity::Column::StatDate.eq(stat_date))
            .one(db)
            .await
        {
            Ok(existing) => existing,
            Err(e) => {
                error!(%layer_id, error = %e, "Failed to read existing stats row");
                unwritten.push(stats);
                continue;
            }
        };

        let write = if let Some(existing_record) = existing {
            // Update existing record
            let mut active_model: stats_entity::ActiveModel = existing_record.into();
            active_model.xyz_tile_count = Set(add_count(
                active_model.xyz_tile_count.unwrap(),
                i64::from(stats.xyz_tile_count),
            ));
            active_model.cog_download_count = Set(add_count(
                active_model.cog_download_count.unwrap(),
                i64::from(stats.cog_download_count),
            ));
            active_model.pixel_query_count = Set(add_count(
                active_model.pixel_query_count.unwrap(),
                i64::from(stats.pixel_query_count),
            ));
            active_model.stac_request_count = Set(add_count(
                active_model.stac_request_count.unwrap(),
                i64::from(stats.stac_request_count),
            ));
            active_model.cache_hit_count = Set(add_count(
                active_model.cache_hit_count.unwrap(),
                i64::from(stats.cache_hit_count),
            ));
            active_model.cache_miss_count = Set(add_count(
                active_model.cache_miss_count.unwrap(),
                i64::from(stats.cache_miss_count),
            ));
            active_model.last_accessed_at = Set(chrono::Utc::now());

            stats_entity::Entity::update(active_model)
                .exec(db)
                .await
                .map(|_| ())
        } else {
            // Insert new record
            let new_record = stats_entity::ActiveModel {
                id: Set(uuid::Uuid::new_v4()),
                layer_id: Set(layer_id),
                stat_date: Set(stat_date),
                last_accessed_at: Set(chrono::Utc::now()),
                xyz_tile_count: Set(stats.xyz_tile_count),
                cog_download_count: Set(stats.cog_download_count),
                pixel_query_count: Set(stats.pixel_query_count),
                stac_request_count: Set(stats.stac_request_count),
                cache_hit_count: Set(stats.cache_hit_count),
                cache_miss_count: Set(stats.cache_miss_count),
            };

            stats_entity::Entity::insert(new_record)
                .exec(db)
                .await
                .map(|_| ())
        };

        if let Err(e) = write {
            error!(%layer_id, error = %e, "Failed to write stats row");
            unwritten.push(stats);
            continue;
        }

        synced_count += 1;
    }

    (synced_count, unwritten)
}

#[derive(Debug)]
struct StatsCounter {
    layer_id: String,
    date: String,
    xyz_tile_count: i32,
    cog_download_count: i32,
    pixel_query_count: i32,
    stac_request_count: i32,
    cache_hit_count: i32,
    cache_miss_count: i32,
}

impl StatsCounter {
    fn new(layer_id: String, date: String) -> Self {
        Self {
            layer_id,
            date,
            xyz_tile_count: 0,
            cog_download_count: 0,
            pixel_query_count: 0,
            stac_request_count: 0,
            cache_hit_count: 0,
            cache_miss_count: 0,
        }
    }
}

/// How a statistics key names its layer. Increments key on the layer UUID for
/// pixel reads, and on the layer name for tiles, COG downloads and STAC.
#[derive(Debug, PartialEq)]
pub enum LayerIdentifier {
    Id(uuid::Uuid),
    Name(String),
}

pub fn parse_layer_identifier(identifier: &str) -> LayerIdentifier {
    match uuid::Uuid::parse_str(identifier) {
        Ok(id) => LayerIdentifier::Id(id),
        Err(_) => LayerIdentifier::Name(identifier.to_string()),
    }
}

/// Resolves a statistics key's identifier to the layer it names.
pub async fn find_layer(
    db: &DatabaseConnection,
    identifier: &LayerIdentifier,
) -> Result<Option<crate::routes::layers::db::Model>, sea_orm::DbErr> {
    use crate::routes::layers::db as layer;
    use sea_orm::{ColumnTrait, QueryFilter};

    match identifier {
        LayerIdentifier::Id(id) => layer::Entity::find_by_id(*id).one(db).await,
        LayerIdentifier::Name(name) => layer::Entity::find()
            .filter(layer::Column::LayerName.eq(name))
            .one(db)
            .await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        let mut config = Config::for_tests();
        config.app_name = "drop4crop".to_string();
        config.deployment = "prod".to_string();
        config
    }

    fn key(suffix: &str) -> String {
        format!("drop4crop-prod/stats:{suffix}")
    }

    #[test]
    fn test_counter_keys_drops_the_sync_lock_and_last_sync_marker() {
        let scanned = vec![
            key("sync_lock"),
            key("2026-09-09:maize_mirca_area_total:xyz"),
            key("last_sync_time"),
        ];
        assert_eq!(
            counter_keys(scanned, &config()),
            vec![key("2026-09-09:maize_mirca_area_total:xyz")]
        );
    }

    #[test]
    fn test_counter_keys_keeps_every_counter() {
        let scanned = vec![
            key("2026-09-09:a:xyz"),
            key("2026-09-09:a:cog"),
            key("2026-09-08:b:stac"),
        ];
        assert_eq!(counter_keys(scanned.clone(), &config()), scanned);
    }

    #[test]
    fn test_counter_keys_on_only_control_keys_is_empty() {
        let scanned = vec![key("sync_lock"), key("last_sync_time")];
        assert!(counter_keys(scanned, &config()).is_empty());
    }

    #[test]
    fn test_restore_increments_puts_back_each_nonzero_counter() {
        let mut counter = StatsCounter::new("maize".to_string(), "2026-09-09".to_string());
        counter.xyz_tile_count = 4;

        let mut restored = restore_increments(&[counter], &config());
        restored.sort();
        assert_eq!(
            restored,
            vec![
                (key("2026-09-09:maize:xyz"), 4),
            ]
        );
    }

    #[test]
    fn test_restore_increments_skips_zero_counters() {
        let counter = StatsCounter::new("maize".to_string(), "2026-09-09".to_string());
        assert!(restore_increments(&[counter], &config()).is_empty());
    }

    #[test]
    fn test_restore_increments_of_nothing_is_nothing() {
        assert!(restore_increments(&[], &config()).is_empty());
    }

    // A cache outcome is restored like any other counter, so an unwritten sync
    // does not lose it.
    #[test]
    fn test_restore_increments_puts_back_cache_outcomes() {
        let mut counter = StatsCounter::new("maize".to_string(), "2026-09-09".to_string());
        counter.cache_hit_count = 7;
        counter.cache_miss_count = 3;

        let mut restored = restore_increments(&[counter], &config());
        restored.sort();
        assert_eq!(
            restored,
            vec![
                (key("2026-09-09:maize:hit"), 7),
                (key("2026-09-09:maize:miss"), 3),
            ]
        );
    }

    #[test]
    fn test_restore_increments_covers_all_four_counters() {
        let mut counter = StatsCounter::new("maize".to_string(), "2026-09-09".to_string());
        counter.xyz_tile_count = 1;
        counter.cog_download_count = 2;
        counter.pixel_query_count = 3;
        counter.stac_request_count = 4;

        let restored = restore_increments(&[counter], &config());
        assert_eq!(restored.len(), 4);
        assert_eq!(restored.iter().map(|(_, n)| n).sum::<i64>(), 10);
    }

    #[test]
    fn test_parse_layer_identifier_uuid() {
        let id = uuid::Uuid::new_v4();
        assert_eq!(
            parse_layer_identifier(&id.to_string()),
            LayerIdentifier::Id(id)
        );
    }

    #[test]
    fn test_parse_layer_identifier_name() {
        assert_eq!(
            parse_layer_identifier("barley_production"),
            LayerIdentifier::Name("barley_production".to_string())
        );
    }

    #[test]
    fn test_add_count_adds() {
        assert_eq!(add_count(0, 5), 5);
        assert_eq!(add_count(7, 0), 7);
        assert_eq!(add_count(1000, 4605), 5605);
    }

    #[test]
    fn test_add_count_holds_at_the_column_ceiling() {
        assert_eq!(add_count(i32::MAX - 1, 5), i32::MAX);
        assert_eq!(add_count(i32::MAX, 1), i32::MAX);
    }

    #[test]
    fn test_add_count_counter_larger_than_the_column() {
        assert_eq!(add_count(0, i64::from(i32::MAX) + 1), i32::MAX);
        assert_eq!(add_count(0, i64::MAX), i32::MAX);
    }

    #[test]
    fn test_add_count_negative_counter_adds_nothing() {
        assert_eq!(add_count(10, -1), 10);
        assert_eq!(add_count(10, i64::MIN), 10);
    }
}
