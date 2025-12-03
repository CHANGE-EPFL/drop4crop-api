//! Distributed background job management for layer operations.
//!
//! Uses Redis as a distributed work queue that multiple API replicas can process.
//! Each replica runs a background worker that polls for work and processes layers.
//!
//! ## Redis Keys Structure:
//! - `jobs:recalc:status` - HASH with job metadata (is_running, started_at, total_layers)
//! - `jobs:recalc:todo` - SET of layer IDs waiting to be processed
//! - `jobs:recalc:processing` - HASH of {layer_id: "worker_id:timestamp"}
//! - `jobs:recalc:completed` - SET of successfully processed layer IDs
//! - `jobs:recalc:errors` - HASH of {layer_id: error_message}
//! - `jobs:recalc:cancel` - Flag key (exists = cancel requested)

use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Timeout in seconds after which a processing item is considered stale
const STALE_TIMEOUT_SECS: i64 = 60;

/// How often workers poll for work when idle (in seconds)
pub const WORKER_IDLE_POLL_INTERVAL_SECS: u64 = 30;

/// Maximum number of retries before marking an item as failed
const MAX_RETRIES: u64 = 3;

// ============================================================================
// Redis Key Functions
// ============================================================================

fn key_prefix(config: &crate::config::Config) -> String {
    format!("{}-{}/jobs:recalc", config.app_name, config.deployment)
}

fn status_key(config: &crate::config::Config) -> String {
    format!("{}:status", key_prefix(config))
}

fn todo_key(config: &crate::config::Config) -> String {
    format!("{}:todo", key_prefix(config))
}

fn processing_key(config: &crate::config::Config) -> String {
    format!("{}:processing", key_prefix(config))
}

fn completed_key(config: &crate::config::Config) -> String {
    format!("{}:completed", key_prefix(config))
}

fn errors_key(config: &crate::config::Config) -> String {
    format!("{}:errors", key_prefix(config))
}

fn cancel_key(config: &crate::config::Config) -> String {
    format!("{}:cancel", key_prefix(config))
}

fn retries_key(config: &crate::config::Config) -> String {
    format!("{}:retries", key_prefix(config))
}

// ============================================================================
// Data Structures
// ============================================================================

/// Job metadata stored in Redis HASH
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JobMetadata {
    pub is_running: bool,
    pub started_at: Option<DateTime<Utc>>,
    pub total_layers: u64,
    pub started_by: Option<String>,
}

/// Aggregated job status (computed from all Redis keys)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecalculateJobStatus {
    /// Whether the job is currently running
    pub is_running: bool,
    /// When the job started
    pub started_at: Option<DateTime<Utc>>,
    /// Total number of layers to process
    pub total_layers: u64,
    /// Number of layers still in todo queue
    pub todo_count: u64,
    /// Number of layers currently being processed
    pub processing_count: u64,
    /// Number of layers processed so far (completed + errors)
    pub processed_count: u64,
    /// Number of successful recalculations
    pub success_count: u64,
    /// Number of failed recalculations
    pub error_count: u64,
    /// Recent errors (last 10)
    pub recent_errors: Vec<String>,
    /// When the job completed (if finished)
    pub completed_at: Option<DateTime<Utc>>,
    /// Who started the job
    pub started_by: Option<String>,
    /// Number of active workers (items in processing)
    pub active_workers: u64,
    /// Whether any items appear stale
    pub has_stale_items: bool,
    /// Count of stale items
    pub stale_count: u64,
}

impl Default for RecalculateJobStatus {
    fn default() -> Self {
        Self {
            is_running: false,
            started_at: None,
            total_layers: 0,
            todo_count: 0,
            processing_count: 0,
            processed_count: 0,
            success_count: 0,
            error_count: 0,
            recent_errors: Vec::new(),
            completed_at: None,
            started_by: None,
            active_workers: 0,
            has_stale_items: false,
            stale_count: 0,
        }
    }
}

impl RecalculateJobStatus {
    /// Calculate elapsed time in seconds
    pub fn elapsed_seconds(&self) -> Option<i64> {
        self.started_at.map(|start| {
            let end = self.completed_at.unwrap_or_else(Utc::now);
            (end - start).num_seconds()
        })
    }

    /// Calculate progress percentage
    pub fn progress_percent(&self) -> f64 {
        if self.total_layers == 0 {
            0.0
        } else {
            (self.processed_count as f64 / self.total_layers as f64) * 100.0
        }
    }
}

/// Information about a processing item
#[derive(Debug, Clone)]
pub struct ProcessingItem {
    pub layer_id: Uuid,
    pub worker_id: String,
    pub started_at: DateTime<Utc>,
}

impl ProcessingItem {
    pub fn is_stale(&self) -> bool {
        (Utc::now() - self.started_at).num_seconds() > STALE_TIMEOUT_SECS
    }
}

// ============================================================================
// Redis Connection Helper
// ============================================================================

async fn get_connection(config: &crate::config::Config) -> Result<redis::aio::MultiplexedConnection, String> {
    let redis_client = crate::routes::tiles::cache::get_redis_client(config);
    redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| format!("Redis connection error: {}", e))
}

// ============================================================================
// Job Control Functions
// ============================================================================

/// Start a new distributed job by populating the todo queue
pub async fn start_job(
    config: &crate::config::Config,
    layer_ids: Vec<Uuid>,
    worker_id: &str,
) -> Result<u64, String> {
    let mut con = get_connection(config).await?;
    let total = layer_ids.len() as u64;

    if total == 0 {
        return Ok(0);
    }

    // Clear any existing job data
    clear_job_data(config).await?;

    // Set job metadata
    let metadata = JobMetadata {
        is_running: true,
        started_at: Some(Utc::now()),
        total_layers: total,
        started_by: Some(worker_id.to_string()),
    };
    let metadata_json = serde_json::to_string(&metadata)
        .map_err(|e| format!("JSON error: {}", e))?;

    let _: () = con.set_ex(status_key(config), metadata_json, 86400)
        .await
        .map_err(|e| format!("Redis error: {}", e))?;

    // Add all layer IDs to the todo set
    let todo_key = todo_key(config);
    let layer_id_strings: Vec<String> = layer_ids.iter().map(|id| id.to_string()).collect();

    // Add in batches to avoid huge commands
    for chunk in layer_id_strings.chunks(1000) {
        let _: () = con.sadd(&todo_key, chunk)
            .await
            .map_err(|e| format!("Redis SADD error: {}", e))?;
    }

    // Set TTL on todo key (24 hours)
    let _: () = con.expire(&todo_key, 86400)
        .await
        .map_err(|e| format!("Redis EXPIRE error: {}", e))?;

    info!(total_layers = total, worker_id, "Started distributed recalculation job");
    Ok(total)
}

/// Add additional layers to an existing job's todo queue
/// Used for recovering pending layers from the database
pub async fn add_layers_to_queue(config: &crate::config::Config, layer_ids: Vec<Uuid>) -> Result<u64, String> {
    if layer_ids.is_empty() {
        return Ok(0);
    }

    let mut con = get_connection(config).await?;
    let todo_key = todo_key(config);
    let total = layer_ids.len() as u64;

    let layer_id_strings: Vec<String> = layer_ids.iter().map(|id| id.to_string()).collect();

    for chunk in layer_id_strings.chunks(1000) {
        let _: () = con.sadd(&todo_key, chunk)
            .await
            .map_err(|e| format!("Redis SADD error: {}", e))?;
    }

    debug!(count = total, "Added layers to todo queue");
    Ok(total)
}

/// Clear all job data from Redis
pub async fn clear_job_data(config: &crate::config::Config) -> Result<(), String> {
    let mut con = get_connection(config).await?;

    let keys = vec![
        status_key(config),
        todo_key(config),
        processing_key(config),
        completed_key(config),
        errors_key(config),
        cancel_key(config),
        retries_key(config),
    ];

    for key in keys {
        let _: Result<(), _> = con.del(&key).await;
    }

    Ok(())
}

/// Request cancellation of the job
pub async fn request_cancellation(config: &crate::config::Config) -> Result<(), String> {
    let mut con = get_connection(config).await?;
    let _: () = con.set_ex(cancel_key(config), "1", 3600)
        .await
        .map_err(|e| format!("Redis error: {}", e))?;
    info!("Job cancellation requested");
    Ok(())
}

/// Check if cancellation was requested
pub async fn is_cancellation_requested(config: &crate::config::Config) -> bool {
    let mut con = match get_connection(config).await {
        Ok(c) => c,
        Err(_) => return false,
    };
    let exists: bool = con.exists(cancel_key(config)).await.unwrap_or(false);
    exists
}

/// Mark job as completed
pub async fn mark_job_completed(config: &crate::config::Config) -> Result<(), String> {
    let mut con = get_connection(config).await?;

    // Get current metadata and update it
    let status_key = status_key(config);
    let metadata_json: Option<String> = con.get(&status_key).await.unwrap_or(None);

    let mut metadata: JobMetadata = metadata_json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();

    metadata.is_running = false;

    let updated_json = serde_json::to_string(&metadata)
        .map_err(|e| format!("JSON error: {}", e))?;

    let _: () = con.set_ex(&status_key, updated_json, 86400)
        .await
        .map_err(|e| format!("Redis error: {}", e))?;

    // Clear cancel flag if any
    let _: Result<(), _> = con.del(cancel_key(config)).await;

    info!("Job marked as completed");
    Ok(())
}

// ============================================================================
// Worker Functions (called by background task)
// ============================================================================

/// Atomically claim a work item from the todo queue
/// Returns None if no work available
pub async fn claim_work(config: &crate::config::Config, worker_id: &str) -> Result<Option<Uuid>, String> {
    let mut con = get_connection(config).await?;

    // SPOP atomically removes and returns a random element
    let todo_key = todo_key(config);
    let layer_id_str: Option<String> = con.spop(&todo_key)
        .await
        .map_err(|e| format!("Redis SPOP error: {}", e))?;

    match layer_id_str {
        Some(id_str) => {
            let layer_id = Uuid::parse_str(&id_str)
                .map_err(|e| format!("Invalid UUID: {}", e))?;

            // Record that we're processing this item
            let processing_key = processing_key(config);
            let value = format!("{}:{}", worker_id, Utc::now().to_rfc3339());
            let _: () = con.hset(&processing_key, &id_str, &value)
                .await
                .map_err(|e| format!("Redis HSET error: {}", e))?;

            // Set TTL on processing key
            let _: () = con.expire(&processing_key, 86400)
                .await
                .map_err(|e| format!("Redis EXPIRE error: {}", e))?;

            debug!(layer_id = %layer_id, worker_id, "Claimed work item");
            Ok(Some(layer_id))
        }
        None => Ok(None),
    }
}

/// Mark a layer as successfully completed
pub async fn mark_layer_completed(config: &crate::config::Config, layer_id: Uuid) -> Result<(), String> {
    let mut con = get_connection(config).await?;
    let id_str = layer_id.to_string();

    // Remove from processing
    let _: () = con.hdel(processing_key(config), &id_str)
        .await
        .map_err(|e| format!("Redis HDEL error: {}", e))?;

    // Add to completed
    let _: () = con.sadd(completed_key(config), &id_str)
        .await
        .map_err(|e| format!("Redis SADD error: {}", e))?;

    // Set TTL
    let _: () = con.expire(completed_key(config), 86400).await.unwrap_or(());

    debug!(layer_id = %layer_id, "Marked layer as completed");
    Ok(())
}

/// Mark a layer as failed with an error message
pub async fn mark_layer_failed(config: &crate::config::Config, layer_id: Uuid, error: &str) -> Result<(), String> {
    let mut con = get_connection(config).await?;
    let id_str = layer_id.to_string();

    // Remove from processing
    let _: () = con.hdel(processing_key(config), &id_str)
        .await
        .map_err(|e| format!("Redis HDEL error: {}", e))?;

    // Add to errors hash
    let _: () = con.hset(errors_key(config), &id_str, error)
        .await
        .map_err(|e| format!("Redis HSET error: {}", e))?;

    // Set TTL
    let _: () = con.expire(errors_key(config), 86400).await.unwrap_or(());

    debug!(layer_id = %layer_id, error, "Marked layer as failed");
    Ok(())
}

/// Recover stale items from processing back to todo
/// Returns the number of items recovered
pub async fn recover_stale_items(config: &crate::config::Config) -> Result<u64, String> {
    let mut con = get_connection(config).await?;
    let processing_key = processing_key(config);
    let todo_key = todo_key(config);
    let retries_key = retries_key(config);
    let errors_key = errors_key(config);

    // Get all processing items
    let items: std::collections::HashMap<String, String> = con.hgetall(&processing_key)
        .await
        .map_err(|e| format!("Redis HGETALL error: {}", e))?;

    let mut recovered = 0u64;
    let mut failed = 0u64;
    let now = Utc::now();

    for (layer_id, value) in items {
        // Parse the value: "worker_id:timestamp"
        let parts: Vec<&str> = value.splitn(2, ':').collect();
        if parts.len() == 2 {
            if let Ok(started_at) = DateTime::parse_from_rfc3339(parts[1]) {
                let elapsed = (now - started_at.with_timezone(&Utc)).num_seconds();
                if elapsed > STALE_TIMEOUT_SECS {
                    // Remove from processing
                    let _: () = con.hdel(&processing_key, &layer_id)
                        .await
                        .map_err(|e| format!("Redis HDEL error: {}", e))?;

                    // Increment retry count
                    let retry_count: u64 = con.hincr(&retries_key, &layer_id, 1i64)
                        .await
                        .map_err(|e| format!("Redis HINCR error: {}", e))?;

                    if retry_count >= MAX_RETRIES {
                        // Too many retries - mark as failed
                        let error_msg = format!("Timed out {} times (worker crashed or layer processing too slow)", retry_count);
                        let _: () = con.hset(&errors_key, &layer_id, &error_msg)
                            .await
                            .map_err(|e| format!("Redis HSET error: {}", e))?;
                        failed += 1;
                        warn!(layer_id, retry_count, "Layer failed after max retries");
                    } else {
                        // Put back in todo queue for retry
                        let _: () = con.sadd(&todo_key, &layer_id)
                            .await
                            .map_err(|e| format!("Redis SADD error: {}", e))?;
                        recovered += 1;
                        info!(layer_id, retry_count, elapsed_secs = elapsed, "Recovered stale item for retry");
                    }
                }
            }
        }
    }

    if recovered > 0 || failed > 0 {
        info!(recovered, failed, "Processed stale items");
    }

    Ok(recovered)
}

/// Check if the job is complete (no todo, no processing)
pub async fn is_job_complete(config: &crate::config::Config) -> Result<bool, String> {
    let mut con = get_connection(config).await?;

    let todo_count: u64 = con.scard(todo_key(config)).await.unwrap_or(0);
    let processing_count: u64 = con.hlen(processing_key(config)).await.unwrap_or(0);

    Ok(todo_count == 0 && processing_count == 0)
}

/// Check if a job is currently active (has work to do or items being processed)
pub async fn is_job_active(config: &crate::config::Config) -> bool {
    let mut con = match get_connection(config).await {
        Ok(c) => c,
        Err(_) => return false,
    };

    // Check if job metadata says it's running
    let metadata_json: Option<String> = con.get(status_key(config)).await.unwrap_or(None);
    let metadata_running = metadata_json
        .and_then(|j| serde_json::from_str::<JobMetadata>(&j).ok())
        .map(|m| m.is_running)
        .unwrap_or(false);

    if !metadata_running {
        return false;
    }

    // Job is active if there's work in todo OR items in processing (might be stale)
    let todo_count: u64 = con.scard(todo_key(config)).await.unwrap_or(0);
    let processing_count: u64 = con.hlen(processing_key(config)).await.unwrap_or(0);

    todo_count > 0 || processing_count > 0
}

// ============================================================================
// Status Functions
// ============================================================================

/// Get the full aggregated job status
pub async fn get_job_status(config: &crate::config::Config) -> RecalculateJobStatus {
    let mut con = match get_connection(config).await {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "Failed to get Redis connection for status");
            return RecalculateJobStatus::default();
        }
    };

    // Get metadata
    let metadata_json: Option<String> = con.get(status_key(config)).await.unwrap_or(None);
    let metadata: JobMetadata = metadata_json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();

    // Get counts
    let todo_count: u64 = con.scard(todo_key(config)).await.unwrap_or(0);
    let processing_count: u64 = con.hlen(processing_key(config)).await.unwrap_or(0);
    let success_count: u64 = con.scard(completed_key(config)).await.unwrap_or(0);
    let error_count: u64 = con.hlen(errors_key(config)).await.unwrap_or(0);

    // Get recent errors (last 10)
    let all_errors: std::collections::HashMap<String, String> =
        con.hgetall(errors_key(config)).await.unwrap_or_default();
    let recent_errors: Vec<String> = all_errors
        .into_iter()
        .take(10)
        .map(|(id, err)| format!("{}: {}", &id[..8.min(id.len())], err))
        .collect();

    // Check for stale items
    let processing_items: std::collections::HashMap<String, String> =
        con.hgetall(processing_key(config)).await.unwrap_or_default();

    let now = Utc::now();
    let mut stale_count = 0u64;
    let mut active_workers = std::collections::HashSet::new();

    for (_layer_id, value) in &processing_items {
        let parts: Vec<&str> = value.splitn(2, ':').collect();
        if parts.len() == 2 {
            active_workers.insert(parts[0].to_string());
            if let Ok(started_at) = DateTime::parse_from_rfc3339(parts[1]) {
                if (now - started_at.with_timezone(&Utc)).num_seconds() > STALE_TIMEOUT_SECS {
                    stale_count += 1;
                }
            }
        }
    }

    let processed_count = success_count + error_count;

    // Determine if job is complete
    let completed_at = if metadata.is_running && todo_count == 0 && processing_count == 0 && processed_count > 0 {
        Some(Utc::now())
    } else {
        None
    };

    RecalculateJobStatus {
        is_running: metadata.is_running && (todo_count > 0 || processing_count > 0),
        started_at: metadata.started_at,
        total_layers: metadata.total_layers,
        todo_count,
        processing_count,
        processed_count,
        success_count,
        error_count,
        recent_errors,
        completed_at,
        started_by: metadata.started_by,
        active_workers: active_workers.len() as u64,
        has_stale_items: stale_count > 0,
        stale_count,
    }
}

/// Get list of items currently being processed
pub async fn get_processing_items(config: &crate::config::Config) -> Vec<ProcessingItem> {
    let mut con = match get_connection(config).await {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let items: std::collections::HashMap<String, String> =
        con.hgetall(processing_key(config)).await.unwrap_or_default();

    items
        .into_iter()
        .filter_map(|(id_str, value)| {
            let layer_id = Uuid::parse_str(&id_str).ok()?;
            let parts: Vec<&str> = value.splitn(2, ':').collect();
            if parts.len() == 2 {
                let worker_id = parts[0].to_string();
                let started_at = DateTime::parse_from_rfc3339(parts[1])
                    .ok()?
                    .with_timezone(&Utc);
                Some(ProcessingItem {
                    layer_id,
                    worker_id,
                    started_at,
                })
            } else {
                None
            }
        })
        .collect()
}
