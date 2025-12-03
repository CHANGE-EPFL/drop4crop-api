use crate::config::Config;
use anyhow::Result;
use redis;
use tracing::error;

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

/// Returns a Redis client using the cache DB.
pub fn get_redis_client(config: &Config) -> redis::Client {
    redis::Client::open(config.tile_cache_uri.clone()).unwrap()
}

/// Pushes the data to Redis using the provided key with TTL from config.
pub async fn push_cache_raw(config: &Config, key: &str, data: &[u8]) -> Result<()> {
    let client = get_redis_client(config);
    let mut con = client.get_multiplexed_async_connection().await.unwrap();

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
    let mut con = client.get_multiplexed_async_connection().await.unwrap();
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
