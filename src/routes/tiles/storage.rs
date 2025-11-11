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

/// Returns an S3 client configured using environment values.
async fn get_s3_client() -> Result<Client> {
    let config = crate::config::Config::from_env();

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
pub async fn get_object(object_id: &str) -> Result<Vec<u8>> {
    // Create the keys for the cache and downloading state.
    let cache_key = super::cache::build_cache_key(object_id);
    // Create a key to indicate that a download is in progress.
    let downloading_key = super::cache::build_downloading_key(object_id);

    let client = super::cache::get_redis_client();
    let mut con = client.get_multiplexed_async_connection().await.unwrap();

    // Check if the object is already in the cache.
    if let Some(data) = super::cache::redis_get(&mut con, &cache_key).await? {
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
        if let Some(data) = super::cache::redis_get(&mut con, &cache_key).await? {
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

/// Fetches a specific byte range of an object from S3 (for HTTP Range requests / COG streaming)
/// Does NOT use caching since range requests are typically for different byte ranges each time
pub async fn get_object_range(object_id: &str, range_header: &str) -> Result<Vec<u8>> {
    let client = get_s3_client().await?;
    let config = crate::config::Config::from_env();
    let s3_key = get_s3_key(object_id);

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
async fn download_and_cache(cache_key: &str, downloading_key: &str) -> Result<()> {
    println!("Downloading object {} from S3", cache_key);
    let client = get_s3_client().await?;
    let config = crate::config::Config::from_env();

    // Extract the filename from cache_key (remove app-deployment prefix)
    let filename = cache_key.split('/').next_back().unwrap_or(cache_key);

    // Use the same S3 key format as uploads/deletes for consistency
    let s3_key = get_s3_key(filename);
    println!("Using S3 key: {} for cache key: {}", s3_key, cache_key);

    let response = client
        .get_object()
        .bucket(&config.s3_bucket_id)
        .key(&s3_key)
        .send()
        .await?;

    let data = response.body.collect().await?.into_bytes().to_vec();
    println!("Downloaded object {} from S3, pushing to cache", cache_key);
    super::cache::push_cache_raw(cache_key, &data).await?;
    println!("Removing downloading state for {}", cache_key);
    super::cache::remove_downloading_state_raw(downloading_key).await?;
    Ok(())
}

/// Uploads an object to S3 using AWS SDK
pub async fn upload_object(key: &str, data: &[u8]) -> Result<()> {
    println!(
        "Uploading object {} to S3 using AWS SDK (size: {} bytes)",
        key,
        data.len()
    );

    let client = get_s3_client().await?;
    let config = crate::config::Config::from_env();

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
    println!("AWS SDK upload completed after {:?}", upload_duration);

    match response {
        Ok(_) => {
            println!(
                "SUCCESS: {} uploaded to S3 via AWS SDK in {:?}",
                key, upload_duration
            );
            Ok(())
        }
        Err(e) => {
            println!("FAILED: AWS SDK upload error for {}: {:?}", key, e);
            Err(anyhow::anyhow!("AWS SDK upload error: {}", e))
        }
    }
}
/// Deletes an object from S3 using AWS SDK
pub async fn delete_object(key: &str) -> Result<()> {
    println!("Deleting object {} from S3", key);

    let client = get_s3_client().await?;
    let config = crate::config::Config::from_env();

    let delete_start = std::time::Instant::now();

    let response = client
        .delete_object()
        .bucket(&config.s3_bucket_id)
        .key(key)
        .send()
        .await;

    let delete_duration = delete_start.elapsed();
    println!("AWS SDK delete completed after {:?}", delete_duration);

    match response {
        Ok(_) => {
            println!(
                "SUCCESS: {} deleted from S3 via AWS SDK in {:?}",
                key, delete_duration
            );
            Ok(())
        }
        Err(e) => {
            println!("FAILED: AWS SDK delete error for {}: {:?}", key, e);
            Err(anyhow::anyhow!("AWS SDK delete error: {}", e))
        }
    }
}

/// Gets the S3 key for a given filename based on configuration.
pub fn get_s3_key(filename: &str) -> String {
    let config = crate::config::Config::from_env();
    format!("{}/{}", config.s3_prefix, filename)
}

pub async fn delete_s3_object_by_db_id(db: &sea_orm::DatabaseConnection, id: &Uuid) -> Result<()> {
    use crate::routes::layers::db::Layer;

    // Query the layer to get the filename
    let layer: Layer = Layer::get_one(db, *id).await?;

    match layer.filename {
        None => {
            println!("Layer with ID {} not found in DB", id);
            Err(anyhow::anyhow!("Layer not found"))
        }
        Some(filename) => {
            let s3_key = get_s3_key(&filename);
            println!("Deleting S3 object for layer ID {}: {}", id, s3_key);
            delete_object(&s3_key).await?;
            println!("Deleted S3 object for layer ID {}: {}", id, s3_key);
            Ok(())
        }
    }
}
