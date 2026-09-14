use anyhow::Result;
use aws_config::BehaviorVersion;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::{Client, config::Credentials, config::Region};
use crudcrate::CRUDResource;
use redis;
use std::error::Error;
use tokio::{
    task,
    time::{Duration, sleep},
};
use tracing::{debug, error, info};
use uuid::Uuid;

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

/// Builds the project-scoped S3 key stem for a layer.
///
/// Layers with a `project_id` land under `{project_id}/{filename}` in the bucket
/// so two projects can legitimately share a filename without clobbering each
/// other. S3 has no concept of "folders": a PUT to `{prefix}/{pid}/{file}`
/// just creates that object; the prefix is implicit, so we never need to
/// separately `mkdir` the project subpath.
///
/// Legacy rows without a `project_id` keep the flat `{filename}` layout.
pub fn s3_key_stem(project_id: Option<Uuid>, filename: &str) -> String {
    match project_id {
        Some(pid) => format!("{}/{}", pid, filename),
        None => filename.to_string(),
    }
}

/// How long a failed download is remembered, so a missing or unreachable object is
/// not re-fetched on every request.
const DOWNLOAD_FAILURE_TTL: u64 = 30;

/// What a request waiting on a download should do at a poll.
#[derive(Debug, PartialEq, Eq)]
enum PollDecision {
    Serve(Vec<u8>),
    Fail(String),
    Claim,
    Wait,
}

/// Decides a waiter's next move from the state of the three keys. Cached bytes win over a
/// failure marker left by an earlier attempt; a recorded failure ends the wait rather than
/// running it out to the deadline; an unclaimed download is claimed by the waiter.
fn poll_decision(
    cached: Option<Vec<u8>>,
    failure: Option<String>,
    downloading: bool,
    timed_out: bool,
) -> PollDecision {
    if let Some(data) = cached {
        return PollDecision::Serve(data);
    }
    if let Some(reason) = failure {
        return PollDecision::Fail(reason);
    }
    if timed_out {
        return PollDecision::Fail("Timeout waiting for tile download".to_string());
    }
    if downloading {
        PollDecision::Wait
    } else {
        PollDecision::Claim
    }
}

/// Asynchronously fetches an object by first checking the Redis cache. If the file is not cached,
/// it attempts to set a downloading flag (with a TTL) and spawns a background task to fetch it from S3.
/// Meanwhile, callers loop waiting for the cache to be filled.
///
/// The `(project_id, filename)` pair is combined via [`s3_key_stem`] so the same
/// filename in two different projects maps to two distinct cache keys and S3 objects.
pub async fn get_object(
    config: &crate::config::Config,
    project_id: Option<Uuid>,
    filename: &str,
) -> Result<Vec<u8>> {
    let stem = s3_key_stem(project_id, filename);

    // Create the keys for the cache and downloading state.
    let cache_key = super::cache::build_cache_key(config, &stem);
    // Create a key to indicate that a download is in progress.
    let downloading_key = super::cache::build_downloading_key(config, &stem);
    let failed_key = super::cache::build_failed_key(config, &stem);

    let client = super::cache::get_redis_client(config);
    let mut con = match client.get_multiplexed_async_connection().await {
        Ok(con) => con,
        Err(e) => {
            tracing::warn!(error = %e, cache_key, "Redis unavailable, falling back to S3");
            return get_object_direct(config, project_id, filename).await;
        }
    };

    let start_time = std::time::Instant::now();
    let timeout_duration = std::time::Duration::from_secs(60);

    loop {
        // Check the object, then the record of a failed attempt, then who owns the download.
        let cached = super::cache::redis_get(&mut con, &cache_key, config.tile_cache_ttl).await?;
        let failure: Option<String> = redis::cmd("GET")
            .arg(&[&failed_key])
            .query_async(&mut con)
            .await?;
        let downloading: Option<String> = redis::cmd("GET")
            .arg(&[&downloading_key])
            .query_async(&mut con)
            .await?;

        match poll_decision(
            cached,
            failure,
            downloading.is_some(),
            start_time.elapsed() > timeout_duration,
        ) {
            PollDecision::Serve(data) => {
                debug!(
                    cache_key,
                    elapsed_ms = start_time.elapsed().as_millis(),
                    "Serving object from cache"
                );
                return Ok(data);
            }
            PollDecision::Fail(reason) => {
                error!(cache_key, reason, "Giving up on object");
                return Err(anyhow::anyhow!(reason));
            }
            PollDecision::Claim => {
                // Claim the download atomically; a racing waiter that wins goes back to waiting.
                let set_result: Option<String> = redis::cmd("SET")
                    .arg(&[&downloading_key, "true", "NX", "EX", "60"])
                    .query_async(&mut con)
                    .await?;
                if set_result.is_some() {
                    debug!(cache_key, "Claimed download");
                    let cache_key_clone = cache_key.clone();
                    let downloading_key_clone = downloading_key.clone();
                    let failed_key_clone = failed_key.clone();
                    let stem_clone = stem.clone();
                    let config_clone = config.clone();
                    task::spawn(async move {
                        download_and_cache(
                            &config_clone,
                            &cache_key_clone,
                            &downloading_key_clone,
                            &failed_key_clone,
                            &stem_clone,
                        )
                        .await;
                    });
                }
            }
            PollDecision::Wait => {}
        }

        // Back off up to a second between polls.
        let wait_time = std::cmp::min(100 * (1 << (start_time.elapsed().as_secs() / 5)), 1000);
        sleep(Duration::from_millis(wait_time)).await;
    }
}

/// Fetches an object directly from S3, bypassing the Redis cache.
/// Use this for operations like statistics recalculation where we don't want to pollute the cache.
pub async fn get_object_direct(
    config: &crate::config::Config,
    project_id: Option<Uuid>,
    filename: &str,
) -> Result<Vec<u8>> {
    let client = get_s3_client(config).await?;
    let s3_key = get_s3_key(config, project_id, filename);

    debug!(s3_key, "Fetching object directly from S3 (bypassing cache)");

    let response = client
        .get_object()
        .bucket(&config.s3_bucket_id)
        .key(&s3_key)
        .send()
        .await?;

    let data = response.body.collect().await?.into_bytes().to_vec();
    debug!(s3_key, size = data.len(), "Fetched object directly from S3");
    Ok(data)
}

/// Fetches a specific byte range of an object from S3 (for HTTP Range requests / COG streaming)
/// Does NOT use caching since range requests are typically for different byte ranges each time
pub async fn get_object_range(
    config: &crate::config::Config,
    project_id: Option<Uuid>,
    filename: &str,
    range_header: &str,
) -> Result<Vec<u8>> {
    let client = get_s3_client(config).await?;
    let s3_key = get_s3_key(config, project_id, filename);

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

/// Downloads the object from S3 and pushes it to the cache. The downloading flag is cleared on
/// every exit, and a failure is recorded under `failed_key` so waiters stop waiting and the next
/// request does not immediately retry.
///
/// `stem` is the already-computed `{project_id}/{filename}` (or bare `{filename}`)
/// stem, passed explicitly so we don't try to recover it by splitting the
/// cache key (which contains the app-name/deployment prefix too).
async fn download_and_cache(
    config: &crate::config::Config,
    cache_key: &str,
    downloading_key: &str,
    failed_key: &str,
    stem: &str,
) {
    if let Err(e) = fetch_and_push(config, cache_key, stem).await {
        error!(cache_key, error = %e, "Error downloading");
        if let Err(marker_error) =
            super::cache::push_failure_raw(config, failed_key, &e.to_string(), DOWNLOAD_FAILURE_TTL)
                .await
        {
            error!(cache_key, error = %marker_error, "Could not record download failure");
        }
    }
    if let Err(e) = super::cache::remove_downloading_state_raw(config, downloading_key).await {
        error!(cache_key, error = %e, "Could not clear downloading state");
    }
}

/// Fetches the object from S3 and writes it to the cache.
async fn fetch_and_push(config: &crate::config::Config, cache_key: &str, stem: &str) -> Result<()> {
    debug!(cache_key, "Downloading object from S3");
    let client = get_s3_client(config).await?;

    // Reuse the same prefix-joined key format that uploads/deletes use.
    let s3_key = format!("{}/{}", config.s3_prefix, stem);
    debug!(s3_key, cache_key, "Using S3 key");

    let response = client
        .get_object()
        .bucket(&config.s3_bucket_id)
        .key(&s3_key)
        .send()
        .await?;

    let data = response.body.collect().await?.into_bytes().to_vec();
    debug!(
        cache_key,
        size = data.len(),
        "Downloaded object from S3, pushing to cache"
    );
    super::cache::push_cache_raw(config, cache_key, &data).await?;
    Ok(())
}

/// Uploads an object to S3 using AWS SDK
pub async fn upload_object(config: &crate::config::Config, key: &str, data: &[u8]) -> Result<()> {
    debug!(
        key,
        size = data.len(),
        "Uploading object to S3 using AWS SDK"
    );

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
            error!(
                key,
                error = %e,
                debug_error = ?e,
                "AWS SDK upload error - full details"
            );
            Err(anyhow::anyhow!(
                "AWS SDK upload error: {} | Debug: {:?} | Source: {:?}",
                e,
                e,
                e.source()
            ))
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

/// Gets the full S3 key (prefix + stem) for a layer identified by
/// `(project_id, filename)`. This is the single function every read/write
/// site should go through: one place that knows how the bucket is laid out.
pub fn get_s3_key(
    config: &crate::config::Config,
    project_id: Option<Uuid>,
    filename: &str,
) -> String {
    format!(
        "{}/{}",
        config.s3_prefix,
        s3_key_stem(project_id, filename)
    )
}

pub async fn delete_s3_object_by_db_id(
    config: &crate::config::Config,
    db: &sea_orm::DatabaseConnection,
    id: &Uuid,
) -> Result<()> {
    use crate::routes::layers::db::Layer;

    // Query the layer to get the filename and its owning project (if any).
    let layer: Layer = Layer::get_one(db, *id).await?;

    match layer.filename {
        None => {
            error!(layer_id = %id, "Layer not found in DB");
            Err(anyhow::anyhow!("Layer not found"))
        }
        Some(filename) => {
            let s3_key = get_s3_key(config, layer.project_id, &filename);
            debug!(layer_id = %id, s3_key, "Deleting S3 object for layer");
            delete_object(config, &s3_key).await?;
            info!(layer_id = %id, s3_key, "Deleted S3 object for layer");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_poll_decision_serves_cached_bytes() {
        let decision = poll_decision(Some(vec![1, 2, 3]), None, true, false);
        assert_eq!(decision, PollDecision::Serve(vec![1, 2, 3]));
    }

    #[test]
    fn test_poll_decision_serves_cached_bytes_over_stale_failure() {
        let decision = poll_decision(
            Some(vec![1, 2, 3]),
            Some("NoSuchKey".to_string()),
            false,
            false,
        );
        assert_eq!(decision, PollDecision::Serve(vec![1, 2, 3]));
    }

    #[test]
    fn test_poll_decision_fails_immediately_on_recorded_failure() {
        let decision = poll_decision(None, Some("NoSuchKey".to_string()), true, false);
        assert_eq!(decision, PollDecision::Fail("NoSuchKey".to_string()));
    }

    #[test]
    fn test_poll_decision_waits_while_download_is_claimed() {
        let decision = poll_decision(None, None, true, false);
        assert_eq!(decision, PollDecision::Wait);
    }

    #[test]
    fn test_poll_decision_claims_when_flag_expired() {
        let decision = poll_decision(None, None, false, false);
        assert_eq!(decision, PollDecision::Claim);
    }

    #[test]
    fn test_poll_decision_fails_on_timeout() {
        let decision = poll_decision(None, None, true, true);
        assert_eq!(
            decision,
            PollDecision::Fail("Timeout waiting for tile download".to_string())
        );
    }
}
