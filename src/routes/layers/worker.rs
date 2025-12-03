//! Background worker that processes layer recalculation jobs.
//!
//! Each API replica runs this worker, which polls Redis for work items
//! and processes them. Multiple workers can run concurrently across replicas.

use sea_orm::DatabaseConnection;
use sea_orm::entity::*;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use super::jobs::{self, WORKER_POLL_INTERVAL_SECS};
use super::utils::{get_min_max_of_raster, get_global_average_of_raster};
use crate::config::Config;
use crate::routes::tiles::storage;

/// Generate a unique worker ID for this instance
pub fn generate_worker_id() -> String {
    let pid = std::process::id();
    let uuid_short = Uuid::new_v4().to_string();
    let uuid_prefix = uuid_short.split('-').next().unwrap_or("x");
    format!("worker-{}-{}", pid, uuid_prefix)
}

/// Start the background worker loop
/// This should be spawned as a tokio task during application startup
pub async fn start_worker(config: Config, db: DatabaseConnection) {
    let worker_id = generate_worker_id();
    info!(worker_id, "Starting background recalculation worker");

    loop {
        // Sleep first to avoid hammering Redis on startup
        tokio::time::sleep(tokio::time::Duration::from_secs(WORKER_POLL_INTERVAL_SECS)).await;

        // Check if there's an active job
        if !jobs::is_job_active(&config).await {
            debug!("No active job, sleeping...");
            continue;
        }

        // Check for cancellation
        if jobs::is_cancellation_requested(&config).await {
            debug!("Job cancelled, sleeping...");
            continue;
        }

        // Recover any stale items first (any worker can do this)
        if let Err(e) = jobs::recover_stale_items(&config).await {
            warn!(error = %e, "Failed to recover stale items");
        }

        // Try to claim work
        match jobs::claim_work(&config, &worker_id).await {
            Ok(Some(layer_id)) => {
                // Process this layer
                process_layer(&config, &db, &worker_id, layer_id).await;

                // Check if job is now complete
                match jobs::is_job_complete(&config).await {
                    Ok(true) => {
                        info!(worker_id, "Job complete, marking as finished");
                        if let Err(e) = jobs::mark_job_completed(&config).await {
                            error!(error = %e, "Failed to mark job as completed");
                        }
                    }
                    Ok(false) => {}
                    Err(e) => warn!(error = %e, "Failed to check job completion"),
                }
            }
            Ok(None) => {
                // No work available, check if job should be marked complete
                match jobs::is_job_complete(&config).await {
                    Ok(true) => {
                        if jobs::is_job_active(&config).await {
                            info!(worker_id, "Job complete, marking as finished");
                            if let Err(e) = jobs::mark_job_completed(&config).await {
                                error!(error = %e, "Failed to mark job as completed");
                            }
                        }
                    }
                    Ok(false) => {
                        debug!("No work available but job not complete, other workers processing");
                    }
                    Err(e) => warn!(error = %e, "Failed to check job completion"),
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to claim work");
            }
        }
    }
}

/// Process a single layer - calculate its statistics
async fn process_layer(config: &Config, db: &DatabaseConnection, worker_id: &str, layer_id: Uuid) {
    info!(layer_id = %layer_id, worker_id, "Processing layer");

    // Fetch the layer from database
    let layer = match super::db::Entity::find_by_id(layer_id).one(db).await {
        Ok(Some(l)) => l,
        Ok(None) => {
            let error_msg = format!("Layer not found: {}", layer_id);
            error!(layer_id = %layer_id, "Layer not found in database");
            let _ = jobs::mark_layer_failed(config, layer_id, &error_msg).await;
            return;
        }
        Err(e) => {
            let error_msg = format!("Database error: {}", e);
            error!(layer_id = %layer_id, error = %e, "Failed to fetch layer");
            let _ = jobs::mark_layer_failed(config, layer_id, &error_msg).await;
            return;
        }
    };

    let layer_name = layer.layer_name.clone().unwrap_or_default();

    // Get filename
    let filename = match &layer.filename {
        Some(f) => f.clone(),
        None => {
            let error_msg = "Layer has no filename";
            error!(layer_id = %layer_id, layer_name, "Layer has no filename");
            update_layer_error_status(db, layer.clone(), error_msg).await;
            let _ = jobs::mark_layer_failed(config, layer_id, error_msg).await;
            return;
        }
    };

    // Fetch from S3
    let object = match storage::get_object_direct(config, &filename).await {
        Ok(o) => o,
        Err(e) => {
            let error_msg = format!("S3 fetch failed: {}", e);
            error!(layer_id = %layer_id, layer_name, error = %e, "Failed to fetch from S3");
            update_layer_error_status(db, layer.clone(), &error_msg).await;
            let _ = jobs::mark_layer_failed(config, layer_id, &error_msg).await;
            return;
        }
    };

    let file_size = object.len() as i64;

    // Validate file size
    if file_size < 1024 {
        let error_msg = format!("File too small: {} bytes", file_size);
        error!(layer_id = %layer_id, layer_name, file_size, "File too small");
        update_layer_error_status(db, layer.clone(), &error_msg).await;
        let _ = jobs::mark_layer_failed(config, layer_id, &error_msg).await;
        return;
    }

    // Calculate min/max
    let (min_val, max_val) = match get_min_max_of_raster(&object) {
        Ok(v) => v,
        Err(e) => {
            let error_msg = format!("Min/max calculation failed: {}", e);
            error!(layer_id = %layer_id, layer_name, error = %e, "Failed to calculate min/max");
            update_layer_error_status(db, layer.clone(), &error_msg).await;
            let _ = jobs::mark_layer_failed(config, layer_id, &error_msg).await;
            return;
        }
    };

    // Calculate global average
    let global_avg = match get_global_average_of_raster(&object) {
        Ok(v) => v,
        Err(e) => {
            let error_msg = format!("Average calculation failed: {}", e);
            error!(layer_id = %layer_id, layer_name, error = %e, "Failed to calculate average");
            update_layer_error_status(db, layer.clone(), &error_msg).await;
            let _ = jobs::mark_layer_failed(config, layer_id, &error_msg).await;
            return;
        }
    };

    // Validate values
    if !min_val.is_finite() || !max_val.is_finite() || !global_avg.is_finite() {
        let error_msg = "Calculated statistics contain invalid values (NaN/Inf)";
        error!(layer_id = %layer_id, layer_name, min_val, max_val, global_avg, "Invalid statistics values");
        update_layer_error_status(db, layer.clone(), error_msg).await;
        let _ = jobs::mark_layer_failed(config, layer_id, error_msg).await;
        return;
    }

    // Update layer with success
    use super::db::ActiveModel as LayerActiveModel;
    let mut active_layer: LayerActiveModel = layer.into();
    active_layer.min_value = Set(Some(min_val));
    active_layer.max_value = Set(Some(max_val));
    active_layer.global_average = Set(Some(global_avg));
    active_layer.file_size = Set(Some(file_size));
    active_layer.stats_status = Set(Some(serde_json::json!({
        "status": "success",
        "last_run": chrono::Utc::now(),
        "error": null,
        "details": format!("min: {}, max: {}, avg: {}, file_size: {} bytes", min_val, max_val, global_avg, file_size)
    })));

    if let Err(e) = active_layer.update(db).await {
        let error_msg = format!("Database update failed: {}", e);
        error!(layer_id = %layer_id, error = %e, "Failed to update layer");
        let _ = jobs::mark_layer_failed(config, layer_id, &error_msg).await;
        return;
    }

    info!(
        layer_id = %layer_id,
        layer_name,
        min_val, max_val, global_avg,
        worker_id,
        "Successfully processed layer"
    );

    // Mark as completed in the job queue
    if let Err(e) = jobs::mark_layer_completed(config, layer_id).await {
        error!(error = %e, layer_id = %layer_id, "Failed to mark layer as completed in job queue");
    }
}

/// Update layer's stats_status field with error
async fn update_layer_error_status(db: &DatabaseConnection, layer: super::db::Model, error_msg: &str) {
    use super::db::ActiveModel as LayerActiveModel;
    let mut active_layer: LayerActiveModel = layer.into();
    active_layer.stats_status = Set(Some(serde_json::json!({
        "status": "error",
        "last_run": chrono::Utc::now(),
        "error": error_msg
    })));
    let _ = active_layer.update(db).await;
}
