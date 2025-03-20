use anyhow::Result;
use redis;
use s3::creds::Credentials;
use s3::region::Region;
use s3::Bucket;
use tokio::task;
use tokio::time::{sleep, Duration};

/// Returns an S3 bucket configured using environment values.
fn get_bucket() -> Box<Bucket> {
    let config = crate::config::Config::from_env();
    let credentials = Credentials::new(
        Some(&config.s3_access_key),
        Some(&config.s3_secret_key),
        None,
        None,
        None,
    )
    .unwrap();

    let region = Region::Custom {
        region: config.s3_region,
        endpoint: config.s3_endpoint,
    };

    Bucket::new(&config.s3_bucket_id, region, credentials).unwrap()
}

/// Builds the cache key based on the app configuration and object ID.
fn build_cache_key(object_id: &str) -> String {
    let config = crate::config::Config::from_env();
    let path = format!("{}-{}", config.app_name, config.deployment);
    format!("{}/{}", path, object_id)
}

/// Builds the key used to indicate that a download is in progress.
fn build_downloading_key(object_id: &str) -> String {
    let cache_key = build_cache_key(object_id);
    format!("{}:downloading", cache_key)
}

/// Returns a Redis connection URL. When `with_db` is true, it includes the database number.
fn get_redis_connection_url(with_db: bool) -> String {
    let config = crate::config::Config::from_env();
    if with_db {
        format!(
            "redis://{}:{}/{}",
            config.redis_url, config.redis_port, config.redis_db
        )
    } else {
        format!("redis://{}:{}/", config.redis_url, config.redis_port)
    }
}

/// Returns a Redis client using the cache DB.
fn get_redis_client() -> redis::Client {
    let url = get_redis_connection_url(true);
    redis::Client::open(url).unwrap()
}

/// Asynchronously fetches an object by first checking the Redis cache. If the file is not cached,
/// it attempts to set a downloading flag (with a TTL) and spawns a background task to fetch it from S3.
/// Meanwhile, callers loop waiting for the cache to be filled.
pub async fn get_object(object_id: &str) -> Result<Vec<u8>> {
    // Create the keys for the cache and downloading state.
    let cache_key = build_cache_key(object_id);
    // Create a key to indicate that a download is in progress.
    let downloading_key = build_downloading_key(object_id);

    let client = get_redis_client();
    let mut con = client.get_multiplexed_async_connection().await.unwrap();

    // Check if the object is already in the cache.
    if let Some(data) = redis_get(&mut con, &cache_key).await? {
        // println!("Cache hit for {}", cache_key);
        return Ok(data);
    }

    // Try to set the downloading flag atomically (NX) with a 60-second TTL.
    let set_result: Option<String> = redis::cmd("SET")
        .arg(&[&downloading_key, "true", "NX", "EX", "60"])
        .query_async(&mut con)
        .await?;
    if set_result.is_some() {
        println!(
            "Downloading not in progress. Setting downloading state for {}",
            cache_key
        );
        // We are the downloader. Spawn a background task.
        let cache_key_clone = cache_key.clone();
        let downloading_key_clone = downloading_key.clone();
        task::spawn(async move {
            if let Err(e) = download_and_cache(&cache_key_clone, &downloading_key_clone).await {
                eprintln!("Error downloading {}: {:?}", cache_key_clone, e);
            }
        });
    } else {
        println!("Download already in progress for {}", cache_key);
    }

    // Loop until the file appears in the cache.
    loop {
        sleep(Duration::from_secs(1)).await;
        if let Some(data) = redis_get(&mut con, &cache_key).await? {
            println!("Cache filled for {}", cache_key);
            return Ok(data);
        }

        // In case the downloading flag has expired (e.g. due to an error),
        // try to re-establish it and spawn the background download.
        let existing: Option<String> = redis::cmd("GET")
            .arg(&[&downloading_key])
            .query_async(&mut con)
            .await?;
        if existing.is_none() {
            let set_result: Option<String> = redis::cmd("SET")
                .arg(&[&downloading_key, "true", "NX", "EX", "60"])
                .query_async(&mut con)
                .await?;
            if set_result.is_some() {
                println!("Re-setting downloading state for {}", cache_key);
                let cache_key_clone = cache_key.clone();
                let downloading_key_clone = downloading_key.clone();
                task::spawn(async move {
                    if let Err(e) =
                        download_and_cache(&cache_key_clone, &downloading_key_clone).await
                    {
                        eprintln!("Error re-downloading {}: {:?}", cache_key_clone, e);
                    }
                });
            }
        }
    }
}

/// Helper function to get a value from Redis by key.
async fn redis_get(
    con: &mut redis::aio::MultiplexedConnection,
    key: &str,
) -> Result<Option<Vec<u8>>> {
    let result: Option<Vec<u8>> = redis::cmd("GET").arg(key).query_async(con).await?;
    Ok(result)
}

/// Downloads the object from S3 and pushes it to the cache. On completion (or error), it removes
/// the downloading flag so that waiting threads can act accordingly.
async fn download_and_cache(cache_key: &str, downloading_key: &str) -> Result<()> {
    println!("Downloading object {} from S3", cache_key);
    let bucket = get_bucket();
    // Here the S3 object key is the same as the cache_key (which includes the app/deployment prefix).
    let data = bucket.get_object(cache_key).await?.bytes().to_vec();
    println!("Downloaded object {} from S3, pushing to cache", cache_key);
    push_cache_raw(cache_key, &data).await?;
    println!("Removing downloading state for {}", cache_key);
    remove_downloading_state_raw(downloading_key).await?;
    Ok(())
}

/// Pushes the data to Redis using the provided key.
async fn push_cache_raw(key: &str, data: &[u8]) -> Result<()> {
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
async fn remove_downloading_state_raw(key: &str) -> Result<()> {
    let client = get_redis_client();
    let mut con = client.get_multiplexed_async_connection().await.unwrap();
    let _: () = redis::cmd("DEL").arg(key).query_async(&mut con).await?;
    Ok(())
}
