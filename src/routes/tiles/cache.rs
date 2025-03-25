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

/// Pushes the data to Redis using the provided key.
pub async fn push_cache_raw(key: &str, data: &[u8]) -> Result<()> {
    let client = get_redis_client();
    let mut con = client.get_multiplexed_async_connection().await.unwrap();
    let _: () = redis::cmd("SET")
        .arg(key)
        .arg(data)
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
