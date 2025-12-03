use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
    routing::{delete, get, post},
};
use crate::common::state::AppState;
use crate::common::auth::Role;
use crate::routes::admin::db::layer_statistics;
use axum_keycloak_auth::{layer::KeycloakAuthLayer, PassthroughMode};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use utoipa_axum::router::OpenApiRouter;
use tracing::{info, debug, warn, error};

/// Builds the statistics router with protected endpoints.
pub fn stats_router(state: &AppState) -> OpenApiRouter {
    let mut router = OpenApiRouter::new()
        .route("/summary", get(get_stats_summary))
        .route("/", get(get_layer_stats))  // List all statistics (for React Admin with Content-Range headers)
        .route("/{id}", get(get_layer_stat_detail))  // Get individual statistic
        .route("/{id}/timeline", get(get_layer_timeline))
        .route("/live", get(get_live_stats))
        .with_state(state.db.clone());

    // Protect stats routes with Keycloak authentication
    if let Some(instance) = state.keycloak_auth_instance.clone() {
        router = router.layer(
            KeycloakAuthLayer::<Role>::builder()
                .instance(instance)
                .passthrough_mode(PassthroughMode::Block)
                .persist_raw_claims(false)
                .expected_audiences(vec![String::from("account")])
                .required_roles(vec![Role::Administrator])
                .build(),
        );
    } else if !state.config.tests_running {
        warn!("Statistics routes are not protected - Keycloak is disabled");
    }

    router
}

/// Builds the cache management router with protected endpoints.
pub fn cache_router(state: &AppState) -> OpenApiRouter {
    let mut router = OpenApiRouter::new()
        .route("/info", get(get_cache_info))
        .route("/keys", get(get_cache_keys))
        .route("/clear", post(clear_all_cache))
        .route("/layers/{layer_name}", delete(clear_layer_cache))
        .route("/layers/{layer_name}/warm", post(warm_layer_cache))
        .route("/layers/{layer_name}/persist", post(persist_layer_cache))
        .route("/layers/{layer_name}/persist", delete(unpersist_layer_cache))
        .route("/ttl", get(get_cache_ttl))
        .with_state(state.db.clone());

    // Protect cache routes with Keycloak authentication
    if let Some(instance) = state.keycloak_auth_instance.clone() {
        router = router.layer(
            KeycloakAuthLayer::<Role>::builder()
                .instance(instance)
                .passthrough_mode(PassthroughMode::Block)
                .persist_raw_claims(false)
                .expected_audiences(vec![String::from("account")])
                .required_roles(vec![Role::Administrator])
                .build(),
        );
    } else if !state.config.tests_running {
        warn!("Cache management routes are not protected - Keycloak is disabled");
    }

    router
}

#[derive(Deserialize)]
struct StatsQuery {
    filter: Option<String>,  // React-Admin sends filters as JSON string
    range: Option<String>,   // React-Admin sends range as JSON string
    sort: Option<String>,    // React-Admin sends sort as JSON string
}

#[derive(Deserialize)]
struct StatsFilter {
    layer_name: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
}

#[derive(Serialize)]
struct DailyRequests {
    date: String,
    requests: i64,
}

#[derive(Serialize)]
struct StatsSummary {
    total_requests_all_time: i64,
    total_requests_today: i64,
    total_requests_week: i64,
    most_accessed_layer: Option<LayerAccessInfo>,
    active_layers_24h: i64,
    total_layers: i64,
    // Breakdown by request type for today
    xyz_tile_count_today: i64,
    cog_download_count_today: i64,
    pixel_query_count_today: i64,
    stac_request_count_today: i64,
    other_request_count_today: i64,
    // Daily breakdown for last 7 days (for charts)
    daily_requests: Vec<DailyRequests>,
}

#[derive(Serialize)]
struct LayerAccessInfo {
    layer_name: String,
    total_requests: i64,
}

#[derive(Serialize)]
struct LayerStatDetail {
    id: String,  // Required by React-Admin
    layer_id: String,
    layer_name: String,
    stat_date: String,
    last_accessed_at: String,
    xyz_tile_count: i32,
    cog_download_count: i32,
    pixel_query_count: i32,
    stac_request_count: i32,
    other_request_count: i32,
    total_requests: i32,
}

#[derive(Serialize)]
struct CacheInfo {
    redis_connected: bool,
    cache_size_mb: f64,
    max_memory_mb: Option<f64>,
    cached_layers_count: usize,
    current_ttl_seconds: u64,
    last_sync_time: Option<String>,
}

#[derive(Serialize)]
struct CachedLayer {
    layer_name: String,
    layer_id: Option<uuid::Uuid>,
    cache_key: String,
    size_bytes: Option<usize>,
    size_mb: Option<f64>,
    ttl_seconds: Option<i64>,
    ttl_hours: Option<f64>,
    cached_since: Option<String>,
}

/// GET /api/admin/stats/summary - Dashboard overview
async fn get_stats_summary(
    State(db): State<DatabaseConnection>,
) -> Result<Json<StatsSummary>, StatusCode> {
    use super::db::layer_statistics;
    use crate::routes::layers::db as layer;

    let today = chrono::Utc::now().naive_utc().date();
    let week_ago = today - chrono::Duration::days(7);
    let day_ago = chrono::Utc::now() - chrono::Duration::hours(24);

    // Total requests all time
    let all_stats = layer_statistics::Entity::find().all(&db).await.map_err(|e| {
        error!(error = %e, "Database error fetching stats");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let total_requests_all_time: i64 = all_stats
        .iter()
        .map(|s| {
            s.xyz_tile_count as i64
                + s.cog_download_count as i64
                + s.pixel_query_count as i64
                + s.stac_request_count as i64
                + s.other_request_count as i64
        })
        .sum();

    // Total requests today
    let today_stats = layer_statistics::Entity::find()
        .filter(layer_statistics::Column::StatDate.eq(today))
        .all(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let total_requests_today: i64 = today_stats
        .iter()
        .map(|s| {
            s.xyz_tile_count as i64
                + s.cog_download_count as i64
                + s.pixel_query_count as i64
                + s.stac_request_count as i64
                + s.other_request_count as i64
        })
        .sum();

    // Breakdown by request type for today
    let xyz_tile_count_today: i64 = today_stats.iter().map(|s| s.xyz_tile_count as i64).sum();
    let cog_download_count_today: i64 = today_stats.iter().map(|s| s.cog_download_count as i64).sum();
    let pixel_query_count_today: i64 = today_stats.iter().map(|s| s.pixel_query_count as i64).sum();
    let stac_request_count_today: i64 = today_stats.iter().map(|s| s.stac_request_count as i64).sum();
    let other_request_count_today: i64 = today_stats.iter().map(|s| s.other_request_count as i64).sum();

    // Total requests this week
    let week_stats = layer_statistics::Entity::find()
        .filter(layer_statistics::Column::StatDate.gte(week_ago))
        .all(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let total_requests_week: i64 = week_stats
        .iter()
        .map(|s| {
            s.xyz_tile_count as i64
                + s.cog_download_count as i64
                + s.pixel_query_count as i64
                + s.stac_request_count as i64
                + s.other_request_count as i64
        })
        .sum();

    // Most accessed layer (past 7 days)
    let mut layer_totals: HashMap<uuid::Uuid, i64> = HashMap::new();
    for stat in &week_stats {
        let total = stat.xyz_tile_count as i64
            + stat.cog_download_count as i64
            + stat.pixel_query_count as i64
            + stat.stac_request_count as i64
            + stat.other_request_count as i64;
        *layer_totals.entry(stat.layer_id).or_insert(0) += total;
    }

    let most_accessed_layer = if let Some((layer_id, total)) = layer_totals.iter().max_by_key(|&(_, v)| v) {
        let layer_record = layer::Entity::find_by_id(*layer_id)
            .one(&db)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        layer_record.map(|l| LayerAccessInfo {
            layer_name: l.layer_name.unwrap_or_else(|| layer_id.to_string()),
            total_requests: *total,
        })
    } else {
        None
    };

    // Active layers in past 24 hours
    let active_layers_24h = layer_statistics::Entity::find()
        .filter(layer_statistics::Column::LastAccessedAt.gte(day_ago.naive_utc()))
        .all(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .iter()
        .map(|s| s.layer_id)
        .collect::<std::collections::HashSet<_>>()
        .len() as i64;

    // Total layers
    let total_layers = layer::Entity::find()
        .count(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? as i64;

    // Daily breakdown for last 7 days (for charts)
    let mut daily_requests = Vec::new();
    for i in (0..7).rev() {
        let date = today - chrono::Duration::days(i);
        let date_str = date.format("%Y-%m-%d").to_string();

        // Sum all requests for this date across all layers
        let day_total: i64 = week_stats
            .iter()
            .filter(|s| s.stat_date == date)
            .map(|s| {
                s.xyz_tile_count as i64
                    + s.cog_download_count as i64
                    + s.pixel_query_count as i64
                    + s.stac_request_count as i64
                    + s.other_request_count as i64
            })
            .sum();

        daily_requests.push(DailyRequests {
            date: date_str,
            requests: day_total,
        });
    }

    Ok(Json(StatsSummary {
        total_requests_all_time,
        total_requests_today,
        total_requests_week,
        most_accessed_layer,
        active_layers_24h,
        total_layers,
        xyz_tile_count_today,
        cog_download_count_today,
        pixel_query_count_today,
        stac_request_count_today,
        other_request_count_today,
        daily_requests,
    }))
}

/// GET /api/admin/stats/layers - All layer statistics
async fn get_layer_stats(
    State(db): State<DatabaseConnection>,
    Query(params): Query<StatsQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    use super::db::layer_statistics;
    use crate::routes::layers::db as layer;
    use axum::http::HeaderMap;

    // Parse filter JSON if provided
    let filter: Option<StatsFilter> = params.filter.as_ref().and_then(|f| {
        serde_json::from_str(f).ok()
    });

    // Parse range JSON if provided [start, end]
    let (limit, offset) = if let Some(range_str) = &params.range {
        if let Ok(range) = serde_json::from_str::<Vec<u64>>(range_str) {
            if range.len() == 2 {
                let start = range[0];
                let end = range[1];
                let limit = end - start + 1;
                (limit, start)
            } else {
                (100, 0)
            }
        } else {
            (100, 0)
        }
    } else {
        (100, 0)
    };

    let mut query = layer_statistics::Entity::find();

    // Apply layer_name filter
    if let Some(ref f) = filter {
        if let Some(ref layer_name) = f.layer_name {
            debug!(layer_name, "Filtering statistics by layer_name");
            // Find the layer by name first
            let layer_record = layer::Entity::find()
                .filter(layer::Column::LayerName.eq(layer_name))
                .one(&db)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

            if let Some(layer) = layer_record {
                debug!(layer_id = %layer.id, "Found layer, filtering statistics");
                // Filter statistics by layer_id
                query = query.filter(layer_statistics::Column::LayerId.eq(layer.id));
            } else {
                debug!(layer_name, "Layer not found, returning empty results");
                // If layer not found, return empty results
                let mut headers = HeaderMap::new();
                headers.insert("Content-Range", "statistics 0-0/0".parse().unwrap());
                headers.insert("Access-Control-Expose-Headers", "Content-Range".parse().unwrap());
                return Ok((headers, Json(vec![])));
            }
        }

        // Apply date filters
        if let Some(ref start) = f.start_date
            && let Ok(date) = chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d") {
                query = query.filter(layer_statistics::Column::StatDate.gte(date));
            }

        if let Some(ref end) = f.end_date
            && let Ok(date) = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d") {
                query = query.filter(layer_statistics::Column::StatDate.lte(date));
            }
    } else {
        debug!("No filter provided");
    }

    // Get total count for Content-Range header
    let total_count = query
        .clone()
        .count(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? as usize;

    let stats = query
        .order_by_desc(layer_statistics::Column::LastAccessedAt)
        .limit(limit)
        .offset(offset)
        .all(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Fetch all layer names in a single query to avoid N+1 problem
    let layer_ids: Vec<uuid::Uuid> = stats.iter().map(|s| s.layer_id).collect();
    let layers = layer::Entity::find()
        .filter(layer::Column::Id.is_in(layer_ids))
        .all(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Create a map for quick lookup
    let layer_map: std::collections::HashMap<uuid::Uuid, String> = layers
        .into_iter()
        .map(|l| (l.id, l.layer_name.unwrap_or_else(|| l.id.to_string())))
        .collect();

    // Build results with layer names
    let results: Vec<LayerStatDetail> = stats
        .into_iter()
        .filter_map(|stat| {
            layer_map.get(&stat.layer_id).map(|layer_name| LayerStatDetail {
                id: stat.id.to_string(),  // React-Admin requires id field
                layer_id: stat.layer_id.to_string(),
                layer_name: layer_name.clone(),
                stat_date: stat.stat_date.to_string(),
                last_accessed_at: stat.last_accessed_at.to_string(),
                xyz_tile_count: stat.xyz_tile_count,
                cog_download_count: stat.cog_download_count,
                pixel_query_count: stat.pixel_query_count,
                stac_request_count: stat.stac_request_count,
                other_request_count: stat.other_request_count,
                total_requests: stat.xyz_tile_count
                    + stat.cog_download_count
                    + stat.pixel_query_count
                    + stat.stac_request_count
                    + stat.other_request_count,
            })
        })
        .collect();

    // Build Content-Range header
    let end_index = if results.is_empty() {
        offset.saturating_sub(1)
    } else {
        offset + results.len() as u64 - 1
    };
    let content_range = format!("statistics {}-{}/{}", offset, end_index, total_count);

    let mut headers = HeaderMap::new();
    headers.insert("Content-Range", content_range.parse().unwrap());
    headers.insert("Access-Control-Expose-Headers", "Content-Range".parse().unwrap());

    Ok((headers, Json(results)))
}

/// GET /api/statistics/:id - Get single statistic by ID (for React Admin)
async fn get_layer_stat_detail(
    State(db): State<DatabaseConnection>,
    Path(stat_id): Path<String>,
) -> Result<Json<LayerStatDetail>, StatusCode> {
    use super::db::layer_statistics;
    use crate::routes::layers::db as layer;

    let stat_uuid = uuid::Uuid::parse_str(&stat_id).map_err(|_| StatusCode::BAD_REQUEST)?;

    let stat = layer_statistics::Entity::find_by_id(stat_uuid)
        .one(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Fetch layer name
    let layer_record = layer::Entity::find_by_id(stat.layer_id)
        .one(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let layer_name = if let Some(layer) = layer_record {
        layer.layer_name.unwrap_or_else(|| stat.layer_id.to_string())
    } else {
        stat.layer_id.to_string()
    };

    let result = LayerStatDetail {
        id: stat.id.to_string(),
        layer_id: stat.layer_id.to_string(),
        layer_name,
        stat_date: stat.stat_date.to_string(),
        last_accessed_at: stat.last_accessed_at.to_string(),
        xyz_tile_count: stat.xyz_tile_count,
        cog_download_count: stat.cog_download_count,
        pixel_query_count: stat.pixel_query_count,
        stac_request_count: stat.stac_request_count,
        other_request_count: stat.other_request_count,
        total_requests: stat.xyz_tile_count
            + stat.cog_download_count
            + stat.pixel_query_count
            + stat.stac_request_count
            + stat.other_request_count,
    };

    Ok(Json(result))
}

/// GET /api/admin/statistics/:stat_id/timeline - Time-series data for charts
/// This gets the timeline for the layer associated with the given statistic record
async fn get_layer_timeline(
    State(db): State<DatabaseConnection>,
    Path(stat_id): Path<String>,
) -> Result<Json<Vec<LayerStatDetail>>, StatusCode> {
    // First get the statistic record to find the layer_id
    let stat_uuid = uuid::Uuid::parse_str(&stat_id)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let stat = layer_statistics::Entity::find_by_id(stat_uuid)
        .one(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Now get all statistics for this layer, ordered by date
    let stats = layer_statistics::Entity::find()
        .filter(layer_statistics::Column::LayerId.eq(stat.layer_id))
        .order_by_asc(layer_statistics::Column::StatDate)
        .all(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Get the layer name
    let layer = crate::routes::layers::db::Entity::find_by_id(stat.layer_id)
        .one(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let layer_name = layer.layer_name.unwrap_or_default();

    // Convert to LayerStatDetail format
    let results: Vec<LayerStatDetail> = stats.into_iter().map(|s| {
        LayerStatDetail {
            id: s.id.to_string(),
            layer_id: s.layer_id.to_string(),
            layer_name: layer_name.clone(),
            stat_date: s.stat_date.to_string(),
            last_accessed_at: s.last_accessed_at.to_rfc3339(),
            xyz_tile_count: s.xyz_tile_count,
            cog_download_count: s.cog_download_count,
            pixel_query_count: s.pixel_query_count,
            stac_request_count: s.stac_request_count,
            other_request_count: s.other_request_count,
            total_requests: s.xyz_tile_count + s.cog_download_count + s.pixel_query_count + s.stac_request_count + s.other_request_count,
        }
    }).collect();

    Ok(Json(results))
}

/// GET /api/admin/cache/info - Cache statistics
async fn get_cache_info() -> Result<Json<CacheInfo>, StatusCode> {
    let config = crate::config::Config::from_env();
    let redis_client = crate::routes::tiles::cache::get_redis_client(&config);

    match redis_client.get_multiplexed_async_connection().await {
        Ok(mut con) => {
            use redis::AsyncCommands;

            // Get Redis INFO
            let info: String = redis::cmd("INFO")
                .arg("memory")
                .query_async(&mut con)
                .await
                .unwrap_or_default();

            // Parse memory usage (rough estimation)
            let cache_size_mb = info
                .lines()
                .find(|line| line.starts_with("used_memory:"))
                .and_then(|line| line.split(':').nth(1))
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(0.0)
                / 1024.0
                / 1024.0;

            // Parse maxmemory (0 means unlimited)
            let max_memory_bytes = info
                .lines()
                .find(|line| line.starts_with("maxmemory:"))
                .and_then(|line| line.split(':').nth(1))
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(0.0);

            let max_memory_mb = if max_memory_bytes > 0.0 {
                Some(max_memory_bytes / 1024.0 / 1024.0)
            } else {
                None
            };

            // Count cached layers (exclude stats and internal keys)
            let cache_pattern = format!("{}-{}/*", config.app_name, config.deployment);
            let all_keys: Vec<String> = scan_keys(&mut con, &cache_pattern).await.unwrap_or_default();
            let cached_layers_count = all_keys.iter()
                .filter(|k| !k.contains("/stats:") && !k.ends_with(":downloading"))
                .count();

            // Get last sync time
            let last_sync_key = format!("{}-{}/stats:last_sync_time", config.app_name, config.deployment);
            let last_sync_time: Option<String> = con.get(&last_sync_key).await.ok();

            Ok(Json(CacheInfo {
                redis_connected: true,
                cache_size_mb,
                max_memory_mb,
                cached_layers_count,
                current_ttl_seconds: config.tile_cache_ttl,
                last_sync_time,
            }))
        }
        Err(_) => Ok(Json(CacheInfo {
            redis_connected: false,
            cache_size_mb: 0.0,
            max_memory_mb: None,
            cached_layers_count: 0,
            current_ttl_seconds: config.tile_cache_ttl,
            last_sync_time: None,
        })),
    }
}

/// GET /api/admin/cache/keys - List all cached layers
async fn get_cache_keys(
    State(db): State<DatabaseConnection>,
) -> Result<Json<Vec<CachedLayer>>, StatusCode> {
    let config = crate::config::Config::from_env();
    let redis_client = crate::routes::tiles::cache::get_redis_client(&config);

    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Match actual cache key pattern: {app}-{deployment}/{filename}
    // Exclude stats and lock keys
    let cache_pattern = format!("{}-{}/*", config.app_name, config.deployment);
    let all_keys = scan_keys(&mut con, &cache_pattern)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Filter out stats and internal keys
    let prefix = format!("{}-{}/", config.app_name, config.deployment);
    let keys: Vec<String> = all_keys.into_iter()
        .filter(|k| !k.contains("/stats:") && !k.ends_with(":downloading"))
        .collect();

    let mut cached_layers = Vec::new();
    for key in keys {
        let layer_name = key
            .strip_prefix(&prefix)
            .unwrap_or(&key)
            .to_string();

        // Get TTL for this key (in seconds, -1 if no expiry, -2 if doesn't exist)
        let ttl_seconds: i64 = redis::cmd("TTL")
            .arg(&key)
            .query_async(&mut con)
            .await
            .unwrap_or(-2);

        let ttl_hours = if ttl_seconds > 0 {
            Some(ttl_seconds as f64 / 3600.0)
        } else {
            None
        };

        // Get size in bytes using STRLEN (works for string keys)
        let size_bytes: Option<usize> = redis::cmd("STRLEN")
            .arg(&key)
            .query_async(&mut con)
            .await
            .ok();

        let size_mb = size_bytes.map(|bytes| bytes as f64 / (1024.0 * 1024.0));

        // Look up layer_id from database by layer_name
        use crate::routes::layers::db as layer;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

        let layer_id = layer::Entity::find()
            .filter(layer::Column::LayerName.eq(&layer_name))
            .one(&db)
            .await
            .ok()
            .flatten()
            .map(|l| l.id);

        // If not found, try with .tif extension
        let layer_id = if layer_id.is_none() && !layer_name.ends_with(".tif") {
            layer::Entity::find()
                .filter(layer::Column::LayerName.eq(format!("{}.tif", layer_name)))
                .one(&db)
                .await
                .ok()
                .flatten()
                .map(|l| l.id)
        } else {
            layer_id
        };

        // If still not found, try without .tif extension
        let layer_id = if layer_id.is_none() && layer_name.ends_with(".tif") {
            layer::Entity::find()
                .filter(layer::Column::LayerName.eq(layer_name.replace(".tif", "")))
                .one(&db)
                .await
                .ok()
                .flatten()
                .map(|l| l.id)
        } else {
            layer_id
        };

        cached_layers.push(CachedLayer {
            layer_name,
            layer_id,
            cache_key: key,
            size_bytes,
            size_mb,
            ttl_seconds: if ttl_seconds >= 0 { Some(ttl_seconds) } else { None },
            ttl_hours,
            cached_since: None,
        });
    }

    Ok(Json(cached_layers))
}

/// POST /api/admin/cache/clear - Clear all cache
async fn clear_all_cache() -> Result<impl IntoResponse, StatusCode> {
    let config = crate::config::Config::from_env();
    let redis_client = crate::routes::tiles::cache::get_redis_client(&config);

    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Match actual cache key pattern and filter out stats/lock keys
    let cache_pattern = format!("{}-{}/*", config.app_name, config.deployment);
    let all_keys = scan_keys(&mut con, &cache_pattern)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Filter out stats and internal keys
    let keys: Vec<String> = all_keys.into_iter()
        .filter(|k| !k.contains("/stats:") && !k.ends_with(":downloading"))
        .collect();

    if !keys.is_empty() {
        let _: () = redis::cmd("DEL")
            .arg(&keys)
            .query_async(&mut con)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    info!(count = keys.len(), "Cleared cache keys");

    Ok(Json(json!({
        "message": format!("Cleared {} cached layers", keys.len()),
        "keys_cleared": keys.len()
    })))
}

/// DELETE /api/admin/cache/layers/:layer_name - Clear specific layer cache
async fn clear_layer_cache(Path(layer_name): Path<String>) -> Result<impl IntoResponse, StatusCode> {
    let config = crate::config::Config::from_env();
    // Add .tif extension if not present (cache keys use filename format)
    let filename = if layer_name.ends_with(".tif") {
        layer_name.clone()
    } else {
        format!("{}.tif", layer_name)
    };
    let cache_key = crate::routes::tiles::cache::build_cache_key(&config, &filename);
    let redis_client = crate::routes::tiles::cache::get_redis_client(&config);

    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let deleted: u32 = redis::cmd("DEL")
        .arg(&cache_key)
        .query_async(&mut con)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if deleted > 0 {
        info!(layer_name, "Cleared cache for layer");
        Ok(Json(json!({
            "message": format!("Cleared cache for layer: {}", layer_name)
        })))
    } else {
        debug!(layer_name, "No cache found for layer");
        Ok(Json(json!({
            "message": format!("No cache found for layer: {}", layer_name)
        })))
    }
}

/// GET /api/admin/cache/ttl - Get current TTL
async fn get_cache_ttl() -> Result<Json<serde_json::Value>, StatusCode> {
    let config = crate::config::Config::from_env();
    Ok(Json(json!({
        "ttl_seconds": config.tile_cache_ttl,
        "ttl_hours": config.tile_cache_ttl / 3600
    })))
}

/// POST /api/admin/cache/layers/:layer_name/warm - Pre-warm cache for a layer
async fn warm_layer_cache(Path(layer_name): Path<String>) -> Result<impl IntoResponse, StatusCode> {
    let config = crate::config::Config::from_env();

    // Add .tif extension if not present
    let filename = if layer_name.ends_with(".tif") {
        layer_name.clone()
    } else {
        format!("{}.tif", layer_name)
    };

    // Use the storage module to fetch and cache the layer
    match crate::routes::tiles::storage::get_object(&config, &filename).await {
        Ok(data) => {
            info!(layer_name, size = data.len(), "Warmed cache for layer");
            Ok(Json(json!({
                "message": format!("Cache warmed for layer: {}", layer_name),
                "size_bytes": data.len(),
                "size_mb": data.len() as f64 / (1024.0 * 1024.0)
            })))
        }
        Err(e) => {
            error!(layer_name, error = %e, "Failed to warm cache for layer");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// POST /api/admin/cache/layers/:layer_name/persist - Remove TTL from cache (make permanent)
async fn persist_layer_cache(Path(layer_name): Path<String>) -> Result<impl IntoResponse, StatusCode> {
    let config = crate::config::Config::from_env();
    let redis_client = crate::routes::tiles::cache::get_redis_client(&config);

    // Add .tif extension if not present
    let filename = if layer_name.ends_with(".tif") {
        layer_name.clone()
    } else {
        format!("{}.tif", layer_name)
    };
    let cache_key = crate::routes::tiles::cache::build_cache_key(&config, &filename);

    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Check if the key exists
    let exists: bool = redis::cmd("EXISTS")
        .arg(&cache_key)
        .query_async(&mut con)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !exists {
        return Ok(Json(json!({
            "message": format!("Layer not in cache: {}. Use /warm first.", layer_name),
            "persisted": false
        })));
    }

    // Remove TTL using PERSIST command
    let result: i32 = redis::cmd("PERSIST")
        .arg(&cache_key)
        .query_async(&mut con)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result == 1 {
        info!(layer_name, "Persisted cache for layer (removed TTL)");
        Ok(Json(json!({
            "message": format!("Cache persisted for layer: {} (TTL removed)", layer_name),
            "persisted": true
        })))
    } else {
        // Key exists but had no TTL (already persistent)
        Ok(Json(json!({
            "message": format!("Layer already persistent: {}", layer_name),
            "persisted": true
        })))
    }
}

/// DELETE /api/admin/cache/layers/:layer_name/persist - Restore TTL to cache
async fn unpersist_layer_cache(Path(layer_name): Path<String>) -> Result<impl IntoResponse, StatusCode> {
    let config = crate::config::Config::from_env();
    let redis_client = crate::routes::tiles::cache::get_redis_client(&config);

    // Add .tif extension if not present
    let filename = if layer_name.ends_with(".tif") {
        layer_name.clone()
    } else {
        format!("{}.tif", layer_name)
    };
    let cache_key = crate::routes::tiles::cache::build_cache_key(&config, &filename);

    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Check if the key exists
    let exists: bool = redis::cmd("EXISTS")
        .arg(&cache_key)
        .query_async(&mut con)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !exists {
        return Ok(Json(json!({
            "message": format!("Layer not in cache: {}", layer_name),
            "unpersisted": false
        })));
    }

    // Restore TTL using EXPIRE command
    let _: bool = redis::cmd("EXPIRE")
        .arg(&cache_key)
        .arg(config.tile_cache_ttl)
        .query_async(&mut con)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    info!(layer_name, ttl = config.tile_cache_ttl, "Restored TTL for layer cache");
    Ok(Json(json!({
        "message": format!("TTL restored for layer: {} ({} seconds)", layer_name, config.tile_cache_ttl),
        "unpersisted": true,
        "ttl_seconds": config.tile_cache_ttl
    })))
}

// TTL updates removed - TTL is a deployment parameter set via TILE_CACHE_TTL environment variable

#[derive(Serialize)]
struct LiveLayerStats {
    layer_id: Option<String>,  // Added for navigation
    layer_name: String,
    date: String,
    xyz_tile_count: i64,
    cog_download_count: i64,
    pixel_query_count: i64,
    stac_request_count: i64,
    other_request_count: i64,
    total_requests: i64,
}

/// GET /api/admin/stats/live - Get real-time statistics from Redis (today's data)
async fn get_live_stats(State(db): State<DatabaseConnection>) -> Result<Json<Vec<LiveLayerStats>>, StatusCode> {
    use crate::routes::layers::db as layer;

    let config = crate::config::Config::from_env();
    let redis_client = crate::routes::tiles::cache::get_redis_client(&config);

    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let stats_pattern = format!("{}-{}/stats:{}:*", config.app_name, config.deployment, today);

    let keys = scan_keys(&mut con, &stats_pattern)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Group by layer
    let mut layer_stats_map: HashMap<String, LiveLayerStats> = HashMap::new();

    for key in keys {
        if let Some((date, layer_name, stat_type)) = parse_live_stats_key(&key, &config) {
            use redis::AsyncCommands;
            let count: i64 = con.get(&key).await.unwrap_or(0);

            let entry = layer_stats_map
                .entry(layer_name.clone())
                .or_insert_with(|| LiveLayerStats {
                    layer_id: None,  // Will be filled in later
                    layer_name: layer_name.clone(),
                    date: date.clone(),
                    xyz_tile_count: 0,
                    cog_download_count: 0,
                    pixel_query_count: 0,
                    stac_request_count: 0,
                    other_request_count: 0,
                    total_requests: 0,
                });

            match stat_type.as_str() {
                "xyz" => entry.xyz_tile_count += count,
                "cog" => entry.cog_download_count += count,
                "pixel" => entry.pixel_query_count += count,
                "stac" => entry.stac_request_count += count,
                "other" => entry.other_request_count += count,
                _ => {}
            }

            entry.total_requests += count;
        }
    }

    // Fetch layer IDs from database
    let mut results: Vec<LiveLayerStats> = layer_stats_map.into_values().collect();
    for stat in &mut results {
        let layer_record = layer::Entity::find()
            .filter(layer::Column::LayerName.eq(&stat.layer_name))
            .one(&db)
            .await
            .ok()
            .flatten();

        if let Some(layer) = layer_record {
            stat.layer_id = Some(layer.id.to_string());
        }
    }

    results.sort_by(|a, b| b.total_requests.cmp(&a.total_requests));

    Ok(Json(results))
}

/// Parses a live stats key from Redis.
fn parse_live_stats_key(key: &str, config: &crate::config::Config) -> Option<(String, String, String)> {
    let prefix = format!("{}-{}/stats:", config.app_name, config.deployment);
    let rest = key.strip_prefix(&prefix)?;
    let parts: Vec<&str> = rest.splitn(3, ':').collect();

    if parts.len() == 3 {
        Some((parts[0].to_string(), parts[1].to_string(), parts[2].to_string()))
    } else {
        None
    }
}

/// Helper function to scan Redis keys
async fn scan_keys(
    con: &mut redis::aio::MultiplexedConnection,
    pattern: &str,
) -> anyhow::Result<Vec<String>> {
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
