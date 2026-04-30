use crate::config::Config;
use anyhow::Result;
use redis;
use tracing::{error, info};

/// Builds the cache key based on the app configuration and object ID.
pub fn build_cache_key(config: &Config, object_id: &str) -> String {
    let path = format!("{}-{}", config.app_name, config.deployment);
    format!("{}/{}", path, object_id)
}

/// Builds the key used to indicate that a download is in progress.
pub fn build_downloading_key(config: &Config, object_id: &str) -> String {
    let cache_key = build_cache_key(config, object_id);
    format!("{}:downloading", cache_key)
}

/// Builds the cache key for a fully-rendered PNG tile. The effective style
/// id (override or layer default) is part of the key so different style
/// pickings don't collide.
pub fn build_rendered_tile_key(
    config: &Config,
    layer_name: &str,
    style_id: Option<uuid::Uuid>,
    z: u32,
    x: u32,
    y: u32,
) -> String {
    let style_part = match style_id {
        Some(s) => s.to_string(),
        None => "default".to_string(),
    };
    build_cache_key(
        config,
        &format!("png/{}/{}/{}/{}/{}", layer_name, style_part, z, x, y),
    )
}

/// Returns a Redis client using the cache DB.
pub fn get_redis_client(config: &Config) -> redis::Client {
    redis::Client::open(config.tile_cache_uri.clone()).unwrap()
}

/// Pushes the data to Redis using the provided key with TTL from config.
pub async fn push_cache_raw(config: &Config, key: &str, data: &[u8]) -> Result<()> {
    let client = get_redis_client(config);
    let mut con = client.get_multiplexed_async_connection().await?;

    let _: () = redis::cmd("SET")
        .arg(key)
        .arg(data)
        .arg("EX")
        .arg(config.tile_cache_ttl) // Apply TTL from config (default: 24 hours)
        .query_async(&mut con)
        .await?;
    Ok(())
}

/// Removes the downloading flag from Redis.
pub async fn remove_downloading_state_raw(config: &Config, key: &str) -> Result<()> {
    let client = get_redis_client(config);
    let mut con = client.get_multiplexed_async_connection().await?;
    let _: () = redis::cmd("DEL").arg(key).query_async(&mut con).await?;
    Ok(())
}

/// Gets a value from Redis and resets its TTL atomically using GETEX.
/// This ensures frequently accessed layers stay cached longer.
/// IMPORTANT: If the key has no TTL (persistent/pinned), we use GET instead
/// of GETEX to preserve the permanent status.
pub async fn redis_get(
    con: &mut redis::aio::MultiplexedConnection,
    key: &str,
    ttl_seconds: u64,
) -> Result<Option<Vec<u8>>> {
    // First check if the key is persistent (TTL = -1)
    let current_ttl: i64 = redis::cmd("TTL")
        .arg(key)
        .query_async(con)
        .await
        .unwrap_or(-2);

    if current_ttl == -1 {
        // Key exists with no expiry (persistent) - use GET to preserve it
        let result: Option<Vec<u8>> = redis::cmd("GET")
            .arg(key)
            .query_async(con)
            .await?;
        Ok(result)
    } else {
        // Key has TTL or doesn't exist - use GETEX to reset TTL on access
        let result: Option<Vec<u8>> = redis::cmd("GETEX")
            .arg(key)
            .arg("EX")
            .arg(ttl_seconds)
            .query_async(con)
            .await?;
        Ok(result)
    }
}

/// Builds a statistics key for tracking layer access by type.
/// Format: {app}-{deploy}/stats:{YYYY-MM-DD}:{layer_id}:{type}
pub fn build_stats_key(config: &Config, layer_id: &str, stat_type: &str) -> String {
    let prefix = format!("{}-{}", config.app_name, config.deployment);
    let today = chrono::Utc::now().format("%Y-%m-%d");
    format!("{}/stats:{}:{}:{}", prefix, today, layer_id, stat_type)
}

/// Scans Redis for all keys matching the given glob pattern.
pub async fn scan_keys(
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

/// Deletes all cache keys matching the given glob pattern.
/// Returns the number of keys deleted. Skips stats and downloading keys.
pub async fn delete_keys_by_pattern(config: &Config, pattern: &str) -> Result<usize> {
    let client = get_redis_client(config);
    let mut con = client.get_multiplexed_async_connection().await?;

    let keys = scan_keys(&mut con, pattern).await?;
    let keys: Vec<String> = keys
        .into_iter()
        .filter(|k| !k.contains("/stats:") && !k.ends_with(":downloading"))
        .collect();

    let count = keys.len();
    if !keys.is_empty() {
        let _: () = redis::cmd("DEL")
            .arg(&keys)
            .query_async(&mut con)
            .await?;
    }

    if count > 0 {
        info!(count, pattern, "Invalidated cache keys");
    }

    Ok(count)
}

/// Invalidates all rendered tiles that used a particular style.
pub async fn invalidate_style_tiles(config: &Config, style_id: uuid::Uuid) -> Result<usize> {
    let pattern = format!("{}-{}/*/{}/{}/*", config.app_name, config.deployment, "png*", style_id);
    delete_keys_by_pattern(config, &pattern).await
}

/// Invalidates all globe tiles.
pub async fn invalidate_globe_tiles(config: &Config) -> Result<usize> {
    let pattern = format!("{}-{}/png-globe/*", config.app_name, config.deployment);
    delete_keys_by_pattern(config, &pattern).await
}

/// Invalidates all card tiles for a specific project slug.
pub async fn invalidate_card_tiles(config: &Config, project_slug: &str) -> Result<usize> {
    let pattern = format!(
        "{}-{}/png-card/{}/*",
        config.app_name, config.deployment, project_slug
    );
    delete_keys_by_pattern(config, &pattern).await
}

/// Invalidates all rendered tiles for a specific layer.
pub async fn invalidate_layer_tiles(config: &Config, layer_name: &str) -> Result<usize> {
    let pattern = format!(
        "{}-{}/png/{}/*",
        config.app_name, config.deployment, layer_name
    );
    delete_keys_by_pattern(config, &pattern).await
}

/// Removes the TTL on a cache key, making it persistent (never expires).
pub async fn persist_key(config: &Config, key: &str) -> Result<()> {
    let client = get_redis_client(config);
    let mut con = client.get_multiplexed_async_connection().await?;
    let _: () = redis::cmd("PERSIST")
        .arg(key)
        .query_async(&mut con)
        .await?;
    Ok(())
}

/// Increments a statistics counter in Redis asynchronously.
/// This is a fire-and-forget operation to avoid blocking the request.
pub async fn increment_stats(config: Config, layer_id: String, stat_type: String) {
    let key = build_stats_key(&config, &layer_id, &stat_type);

    // Spawn a task to avoid blocking the request
    tokio::spawn(async move {
        match async {
            let client = get_redis_client(&config);
            let mut con = client.get_multiplexed_async_connection().await?;
            let _: i64 = redis::cmd("INCR").arg(&key).query_async(&mut con).await?;
            Ok::<(), anyhow::Error>(())
        }
        .await
        {
            Ok(_) => {}
            Err(e) => {
                error!(key, error = %e, "Failed to increment stats");
            }
        }
    });
}
