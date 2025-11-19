use anyhow::Result;
use redis;

/// Builds the cache key based on the app configuration and object ID.
pub fn build_cache_key(object_id: &str) -> String {
    let config = crate::config::Config::from_env();
    let path = format!("{}-{}", config.app_name, config.deployment);
    format!("{}/{}", path, object_id)
}

/// Builds the key used to indicate that a download is in progress.
pub fn build_downloading_key(object_id: &str) -> String {
    let cache_key = build_cache_key(object_id);
    format!("{}:downloading", cache_key)
}

/// Returns a Redis client using the cache DB.
pub fn get_redis_client() -> redis::Client {
    let config = crate::config::Config::from_env();
    redis::Client::open(config.tile_cache_uri).unwrap()
}

/// Pushes the data to Redis using the provided key with TTL from config.
pub async fn push_cache_raw(key: &str, data: &[u8]) -> Result<()> {
    let client = get_redis_client();
    let mut con = client.get_multiplexed_async_connection().await.unwrap();
    let config = crate::config::Config::from_env();

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
pub async fn remove_downloading_state_raw(key: &str) -> Result<()> {
    let client = get_redis_client();
    let mut con = client.get_multiplexed_async_connection().await.unwrap();
    let _: () = redis::cmd("DEL").arg(key).query_async(&mut con).await?;
    Ok(())
}

/// Helper function to get a value from Redis by key.
pub async fn redis_get(
    con: &mut redis::aio::MultiplexedConnection,
    key: &str,
) -> Result<Option<Vec<u8>>> {
    let result: Option<Vec<u8>> = redis::cmd("GET").arg(key).query_async(con).await?;
    Ok(result)
}

/// Gets a value from Redis and resets its TTL atomically using GETEX.
/// This ensures frequently accessed layers stay cached longer.
pub async fn redis_get_and_refresh_ttl(
    con: &mut redis::aio::MultiplexedConnection,
    key: &str,
    ttl_seconds: u64,
) -> Result<Option<Vec<u8>>> {
    let result: Option<Vec<u8>> = redis::cmd("GETEX")
        .arg(key)
        .arg("EX")
        .arg(ttl_seconds)
        .query_async(con)
        .await?;
    Ok(result)
}

/// Builds a statistics key for tracking layer access by type.
/// Format: {app}-{deploy}/stats:{YYYY-MM-DD}:{layer_id}:{type}
pub fn build_stats_key(layer_id: &str, stat_type: &str) -> String {
    let config = crate::config::Config::from_env();
    let prefix = format!("{}-{}", config.app_name, config.deployment);
    let today = chrono::Utc::now().format("%Y-%m-%d");
    format!("{}/stats:{}:{}:{}", prefix, today, layer_id, stat_type)
}

/// Increments a statistics counter in Redis asynchronously.
/// This is a fire-and-forget operation to avoid blocking the request.
pub async fn increment_stats(layer_id: &str, stat_type: &str) {
    let key = build_stats_key(layer_id, stat_type);

    // Spawn a task to avoid blocking the request
    tokio::spawn(async move {
        match async {
            let client = get_redis_client();
            let mut con = client.get_multiplexed_async_connection().await?;
            let _: i64 = redis::cmd("INCR")
                .arg(&key)
                .query_async(&mut con)
                .await?;
            Ok::<(), anyhow::Error>(())
        }.await {
            Ok(_) => {},
            Err(e) => {
                eprintln!("[Stats] Failed to increment stats for {}: {}", key, e);
            }
        }
    });
}
