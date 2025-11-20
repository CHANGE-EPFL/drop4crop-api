use anyhow::Result;
use redis::AsyncCommands;
use sea_orm::{DatabaseConnection, EntityTrait, Set};
use std::collections::HashMap;
use tokio::time::{Duration, interval};
use tracing::{error, info};

/// Spawns a background task that syncs statistics from Redis to PostgreSQL every 5 minutes.
/// Uses distributed locking to ensure only one instance runs the sync at a time.
pub fn spawn_stats_sync_task(db: DatabaseConnection) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(300)); // 5 minutes
        let instance_id = uuid::Uuid::new_v4().to_string();

        loop {
            ticker.tick().await;

            match sync_stats_to_db(&db, &instance_id).await {
                Ok(synced_count) => {
                    if synced_count > 0 {
                        info!(synced_count, "Synced statistics to PostgreSQL");
                    }
                }
                Err(e) => {
                    error!(
                        error = %e,
                        "Stats sync failed, will retry in 5 minutes"
                    );
                }
            }
        }
    });
}

/// Attempts to sync statistics from Redis to PostgreSQL with distributed locking.
async fn sync_stats_to_db(db: &DatabaseConnection, instance_id: &str) -> Result<usize> {
    let config = crate::config::Config::from_env();
    let redis_client = super::tiles::cache::get_redis_client(&config);
    let mut con = redis_client.get_multiplexed_async_connection().await?;

    // Try to acquire distributed lock
    let lock_key = format!("{}-{}/stats:sync_lock", config.app_name, config.deployment);
    let lock_acquired: bool = redis::cmd("SET")
        .arg(&lock_key)
        .arg(instance_id)
        .arg("NX")
        .arg("EX")
        .arg(360) // 6 minute TTL (longer than sync interval)
        .query_async(&mut con)
        .await
        .unwrap_or(false);

    if !lock_acquired {
        // Another instance is handling the sync
        return Ok(0);
    }

    // Check if it's been at least 5 minutes since last sync
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
        if elapsed < chrono::Duration::minutes(5) {
            // Too soon, skip this sync
            return Ok(0);
        }
    }

    // Scan for all stats keys
    let stats_pattern = format!("{}-{}/stats:*", config.app_name, config.deployment);
    let keys: Vec<String> = scan_keys(&mut con, &stats_pattern).await?;

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
        if let Some((date, layer_id, stat_type)) = parse_stats_key(key, &config) {
            let count: i64 = con.get(key).await.unwrap_or(0);
            let entry = stats_map
                .entry((layer_id.clone(), date.clone()))
                .or_insert_with(|| StatsCounter::new(layer_id.clone(), date.clone()));

            match stat_type.as_str() {
                "xyz" => entry.xyz_tile_count += count as i32,
                "cog" => entry.cog_download_count += count as i32,
                "pixel" => entry.pixel_query_count += count as i32,
                "stac" => entry.stac_request_count += count as i32,
                "other" => entry.other_request_count += count as i32,
                _ => {}
            }
        }
    }

    // Write to database with UPSERT
    let synced_count = write_stats_to_db(db, stats_map).await?;

    // Delete processed keys
    if !keys.is_empty() {
        let _: () = redis::cmd("DEL").arg(&keys).query_async(&mut con).await?;
    }

    // Update last sync time
    let _: () = con
        .set(&last_sync_key, chrono::Utc::now().to_rfc3339())
        .await?;

    Ok(synced_count)
}

/// Scans Redis for keys matching the pattern.
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

/// Parses a stats key and extracts the date, layer_id, and stat_type.
/// Format: {app}-{deploy}/stats:{YYYY-MM-DD}:{layer_id}:{type}
fn parse_stats_key(key: &str, config: &crate::config::Config) -> Option<(String, String, String)> {
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
async fn write_stats_to_db(
    db: &DatabaseConnection,
    stats_map: HashMap<(String, String), StatsCounter>,
) -> Result<usize> {
    use crate::routes::layers::db as layer;
    use sea_orm::{ColumnTrait, QueryFilter};

    let mut synced_count = 0;

    for ((layer_name, date_str), stats) in stats_map {
        // Find layer by name
        let layer_record = layer::Entity::find()
            .filter(layer::Column::LayerName.eq(&layer_name))
            .one(db)
            .await?;

        if layer_record.is_none() {
            error!(layer_name, "Layer not found during stats sync");
            continue;
        }

        let layer_id = layer_record.unwrap().id;
        let stat_date = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")?;

        // Check if record exists
        use crate::routes::admin::db::layer_statistics as stats_entity;

        let existing = stats_entity::Entity::find()
            .filter(stats_entity::Column::LayerId.eq(layer_id))
            .filter(stats_entity::Column::StatDate.eq(stat_date))
            .one(db)
            .await?;

        if let Some(existing_record) = existing {
            // Update existing record
            let mut active_model: stats_entity::ActiveModel = existing_record.into();
            active_model.xyz_tile_count =
                Set(active_model.xyz_tile_count.unwrap() + stats.xyz_tile_count);
            active_model.cog_download_count =
                Set(active_model.cog_download_count.unwrap() + stats.cog_download_count);
            active_model.pixel_query_count =
                Set(active_model.pixel_query_count.unwrap() + stats.pixel_query_count);
            active_model.stac_request_count =
                Set(active_model.stac_request_count.unwrap() + stats.stac_request_count);
            active_model.other_request_count =
                Set(active_model.other_request_count.unwrap() + stats.other_request_count);
            active_model.last_accessed_at = Set(chrono::Utc::now());

            stats_entity::Entity::update(active_model).exec(db).await?;
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
                other_request_count: Set(stats.other_request_count),
            };

            stats_entity::Entity::insert(new_record).exec(db).await?;
        }

        synced_count += 1;
    }

    Ok(synced_count)
}

#[derive(Debug)]
struct StatsCounter {
    _layer_id: String,
    _date: String,
    xyz_tile_count: i32,
    cog_download_count: i32,
    pixel_query_count: i32,
    stac_request_count: i32,
    other_request_count: i32,
}

impl StatsCounter {
    fn new(layer_id: String, date: String) -> Self {
        Self {
            _layer_id: layer_id,
            _date: date,
            xyz_tile_count: 0,
            cog_download_count: 0,
            pixel_query_count: 0,
            stac_request_count: 0,
            other_request_count: 0,
        }
    }
}
