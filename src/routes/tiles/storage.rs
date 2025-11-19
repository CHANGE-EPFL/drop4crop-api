use anyhow::Result;
use aws_config::BehaviorVersion;
use aws_sdk_s3::{Client, config::Region, config::Credentials};
use aws_sdk_s3::primitives::ByteStream;
use crudcrate::CRUDResource;
use redis;
use tokio::{
    task,
    time::{Duration, sleep},
};
use uuid::Uuid;
use tracing::{debug, info, error};

/// Returns an S3 client configured using the provided config.
async fn get_s3_client(config: &crate::config::Config) -> Result<Client> {
    // Configure for S3 endpoint
    let credentials = Credentials::new(
        &config.s3_access_key,
        &config.s3_secret_key,
        None,
        None,
        "static",
    );

    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(config.s3_region.clone()))
        .endpoint_url(config.s3_endpoint.clone())
        .credentials_provider(credentials)
        .load()
        .await;

    let client_config = aws_sdk_s3::config::Builder::from(&sdk_config)
        .force_path_style(true) // Required for S3-compatible services
        .build();

    Ok(Client::from_conf(client_config))
}

/// Asynchronously fetches an object by first checking the Redis cache. If the file is not cached,
/// it attempts to set a downloading flag (with a TTL) and spawns a background task to fetch it from S3.
/// Meanwhile, callers loop waiting for the cache to be filled.
pub async fn get_object(config: &crate::config::Config, object_id: &str) -> Result<Vec<u8>> {
    // Create the keys for the cache and downloading state.
    let cache_key = super::cache::build_cache_key(config, object_id);
    // Create a key to indicate that a download is in progress.
    let downloading_key = super::cache::build_downloading_key(config, object_id);

    let client = super::cache::get_redis_client(config);
    let mut con = client.get_multiplexed_async_connection().await.unwrap();

    // Check if the object is already in the cache and reset its TTL on access.
    // This ensures frequently accessed layers stay cached longer.
    if let Some(data) = super::cache::redis_get_and_refresh_ttl(&mut con, &cache_key, config.tile_cache_ttl).await? {
        // println!("Cache hit for {} (TTL reset to {} seconds)", cache_key, config.tile_cache_ttl);
        return Ok(data);
    }

    // Try to set the downloading flag atomically (NX) with a 60-second TTL.
    let set_result: Option<String> = redis::cmd("SET")
        .arg(&[&downloading_key, "true", "NX", "EX", "60"])
        .query_async(&mut con)
        .await?;
    if set_result.is_some() {
        debug!(cache_key, "Downloading not in progress, setting downloading state");
        // We are the downloader. Spawn a background task.
        let cache_key_clone = cache_key.clone();
        let downloading_key_clone = downloading_key.clone();
        let config_clone = config.clone();
        task::spawn(async move {
            if let Err(e) = download_and_cache(&config_clone, &cache_key_clone, &downloading_key_clone).await {
                error!(cache_key = %cache_key_clone, error = %e, "Error downloading");
            }
        });
    } else {
        debug!(cache_key, "Download already in progress");
    }

    // Wait for the file to appear in the cache with a timeout (max 60 seconds)
    let start_time = std::time::Instant::now();
    let timeout_duration = std::time::Duration::from_secs(60);

    loop {
        // Check for timeout
        if start_time.elapsed() > timeout_duration {
            error!(cache_key, "Timeout waiting for download to complete");
            return Err(anyhow::anyhow!("Timeout waiting for tile download"));
        }

        // Wait briefly before checking again (exponential backoff up to 1 second)
        let wait_time = std::cmp::min(
            100 * (1 << (start_time.elapsed().as_secs() / 5)), // Double every 5 seconds
            1000 // Max 1 second
        );
        sleep(Duration::from_millis(wait_time)).await;

        if let Some(data) = super::cache::redis_get_and_refresh_ttl(&mut con, &cache_key, config.tile_cache_ttl).await? {
            debug!(cache_key, ttl = config.tile_cache_ttl, elapsed_ms = start_time.elapsed().as_millis(), "Cache filled");
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
                debug!(cache_key, "Re-setting downloading state after flag expiration");
                let cache_key_clone = cache_key.clone();
                let downloading_key_clone = downloading_key.clone();
                let config_clone = config.clone();
                task::spawn(async move {
                    if let Err(e) =
                        download_and_cache(&config_clone, &cache_key_clone, &downloading_key_clone).await
                    {
                        error!(cache_key = %cache_key_clone, error = %e, "Error re-downloading");
                    }
                });
            }
        }
    }
}

/// Fetches a specific byte range of an object from S3 (for HTTP Range requests / COG streaming)
/// Does NOT use caching since range requests are typically for different byte ranges each time
pub async fn get_object_range(config: &crate::config::Config, object_id: &str, range_header: &str) -> Result<Vec<u8>> {
    let client = get_s3_client(config).await?;
    let s3_key = get_s3_key(config, object_id);

    // S3 GetObject supports the Range header directly
    let response = client
        .get_object()
        .bucket(&config.s3_bucket_id)
        .key(&s3_key)
        .range(range_header)
        .send()
        .await?;

    let data = response.body.collect().await?.into_bytes().to_vec();
    Ok(data)
}

/// Downloads the object from S3 and pushes it to the cache. On completion (or error), it removes
/// the downloading flag so that waiting threads can act accordingly.
async fn download_and_cache(config: &crate::config::Config, cache_key: &str, downloading_key: &str) -> Result<()> {
    debug!(cache_key, "Downloading object from S3");
    let client = get_s3_client(config).await?;

    // Extract the filename from cache_key (remove app-deployment prefix)
    let filename = cache_key.split('/').next_back().unwrap_or(cache_key);

    // Use the same S3 key format as uploads/deletes for consistency
    let s3_key = get_s3_key(config, filename);
    debug!(s3_key, cache_key, "Using S3 key");

    let response = client
        .get_object()
        .bucket(&config.s3_bucket_id)
        .key(&s3_key)
        .send()
        .await?;

    let data = response.body.collect().await?.into_bytes().to_vec();
    debug!(cache_key, size = data.len(), "Downloaded object from S3, pushing to cache");
    super::cache::push_cache_raw(config, cache_key, &data).await?;
    debug!(cache_key, "Removing downloading state");
    super::cache::remove_downloading_state_raw(config, downloading_key).await?;
    Ok(())
}

/// Uploads an object to S3 using AWS SDK
pub async fn upload_object(config: &crate::config::Config, key: &str, data: &[u8]) -> Result<()> {
    debug!(key, size = data.len(), "Uploading object to S3 using AWS SDK");

    let client = get_s3_client(config).await?;

    let upload_start = std::time::Instant::now();

    let body = ByteStream::from(data.to_vec());
    let response = client
        .put_object()
        .bucket(&config.s3_bucket_id)
        .key(key)
        .body(body)
        .send()
        .await;

    let upload_duration = upload_start.elapsed();
    debug!(duration = ?upload_duration, "AWS SDK upload completed");

    match response {
        Ok(_) => {
            info!(key, duration = ?upload_duration, "Successfully uploaded to S3 via AWS SDK");
            Ok(())
        }
        Err(e) => {
            error!(key, error = %e, "AWS SDK upload error");
            Err(anyhow::anyhow!("AWS SDK upload error: {}", e))
        }
    }
}
/// Deletes an object from S3 using AWS SDK
pub async fn delete_object(config: &crate::config::Config, key: &str) -> Result<()> {
    debug!(key, "Deleting object from S3");

    let client = get_s3_client(config).await?;

    let delete_start = std::time::Instant::now();

    let response = client
        .delete_object()
        .bucket(&config.s3_bucket_id)
        .key(key)
        .send()
        .await;

    let delete_duration = delete_start.elapsed();
    debug!(duration = ?delete_duration, "AWS SDK delete completed");

    match response {
        Ok(_) => {
            info!(key, duration = ?delete_duration, "Successfully deleted from S3 via AWS SDK");
            Ok(())
        }
        Err(e) => {
            error!(key, error = %e, "AWS SDK delete error");
            Err(anyhow::anyhow!("AWS SDK delete error: {}", e))
        }
    }
}

/// Gets the S3 key for a given filename based on configuration.
pub fn get_s3_key(config: &crate::config::Config, filename: &str) -> String {
    format!("{}/{}", config.s3_prefix, filename)
}

pub async fn delete_s3_object_by_db_id(config: &crate::config::Config, db: &sea_orm::DatabaseConnection, id: &Uuid) -> Result<()> {
    use crate::routes::layers::db::Layer;

    // Query the layer to get the filename
    let layer: Layer = Layer::get_one(db, *id).await?;

    match layer.filename {
        None => {
            error!(layer_id = %id, "Layer not found in DB");
            Err(anyhow::anyhow!("Layer not found"))
        }
        Some(filename) => {
            let s3_key = get_s3_key(config, &filename);
            debug!(layer_id = %id, s3_key, "Deleting S3 object for layer");
            delete_object(config, &s3_key).await?;
            info!(layer_id = %id, s3_key, "Deleted S3 object for layer");
            Ok(())
        }
    }
}
