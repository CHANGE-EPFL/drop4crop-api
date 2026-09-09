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
use crate::routes::admin::cache_detail::{cache_entry_for_layer, LayerCacheEntryKind};
use axum_keycloak_auth::{layer::KeycloakAuthLayer, PassthroughMode};
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use utoipa_axum::router::OpenApiRouter;
use tracing::{info, debug, warn, error};

/// Builds the statistics router with protected endpoints.
pub fn stats_router(state: &AppState) -> OpenApiRouter {
    let mut router = OpenApiRouter::new()
        .route("/summary", get(get_stats_summary))
        .route("/daily", get(get_daily_stats))
        .route("/activity", get(super::activity::get_activity))
        .route("/", get(get_layer_stats))  // List all statistics (for React Admin with Content-Range headers)
        .route("/{id}", get(get_layer_stat_detail))  // Get individual statistic
        .route("/{id}/timeline", get(get_layer_timeline))
        .route("/live", get(get_live_stats))
        .with_state(state.clone());

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
        .route("/aggregated", get(get_cache_aggregated))
        .route("/layers/{layer_name}/detail", get(get_layer_cache_detail))
        .route("/clear", post(clear_all_cache))
        .route("/layers/{layer_name}", delete(clear_layer_cache))
        .route("/layers/{layer_name}/warm", post(warm_layer_cache))
        .route("/layers/{layer_name}/persist", post(persist_layer_cache))
        .route("/layers/{layer_name}/persist", delete(unpersist_layer_cache))
        .route("/ttl", get(get_cache_ttl))
        .with_state(state.clone());

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
    layer_id: Option<String>,
    layer_name: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
}

/// The statistics filter after parsing, as the query needs it.
#[derive(Debug)]
struct ParsedStatsFilter {
    layer_id: Option<uuid::Uuid>,
    layer_name: Option<String>,
    start_date: Option<chrono::NaiveDate>,
    end_date: Option<chrono::NaiveDate>,
}

/// Reads the react-admin `filter` parameter. A filter that cannot be read is a
/// bad request: answering with an unfiltered page shows the reader the whole
/// table under the filter they just entered.
fn parse_stats_filter(raw: Option<&str>) -> Result<Option<ParsedStatsFilter>, StatusCode> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let filter: StatsFilter = serde_json::from_str(raw).map_err(|e| {
        debug!(error = %e, filter = raw, "Rejecting unreadable statistics filter");
        StatusCode::BAD_REQUEST
    })?;
    let parse_date = |d: &Option<String>| match d {
        Some(s) => chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map(Some)
            .map_err(|e| {
                debug!(error = %e, date = s, "Rejecting unreadable statistics date");
                StatusCode::BAD_REQUEST
            }),
        None => Ok(None),
    };
    let layer_id = filter
        .layer_id
        .as_deref()
        .map(uuid::Uuid::parse_str)
        .transpose()
        .map_err(|e| {
            debug!(error = %e, "Rejecting unreadable layer ID");
            StatusCode::BAD_REQUEST
        })?;
    Ok(Some(ParsedStatsFilter {
        layer_id,
        start_date: parse_date(&filter.start_date)?,
        end_date: parse_date(&filter.end_date)?,
        layer_name: filter.layer_name,
    }))
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
    cache_hit_count: i32,
    cache_miss_count: i32,
    total_requests: i32,
}

#[derive(FromQueryResult)]
struct AggregatedLayerStat {
    layer_id: uuid::Uuid,
    layer_name: Option<String>,
    last_accessed_at: chrono::DateTime<chrono::Utc>,
    xyz_tile_count: i64,
    cog_download_count: i64,
    pixel_query_count: i64,
    stac_request_count: i64,
}

#[derive(Serialize)]
struct LayerStatSummary {
    id: String,
    layer_id: String,
    layer_name: String,
    last_accessed_at: String,
    xyz_tile_count: i64,
    cog_download_count: i64,
    pixel_query_count: i64,
    stac_request_count: i64,
    total_requests: i64,
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

#[derive(Serialize)]
struct AggregatedCacheEntry {
    layer_name: String,
    layer_id: Option<uuid::Uuid>,
    total_size_bytes: usize,
    total_size_mb: f64,
    cog_cached: bool,
    cog_size_mb: f64,
    cog_ttl_hours: Option<f64>,
    png_tile_count: usize,
    png_tile_size_mb: f64,
}

#[derive(Serialize)]
struct CacheObjectDetail {
    cache_key: String,
    size_bytes: usize,
    size_mb: f64,
    ttl_seconds: Option<i64>,
    ttl_hours: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tile_coords: Option<String>,
}

#[derive(Serialize)]
struct LayerCacheDetail {
    total_items: usize,
    total_size_bytes: usize,
    total_size_mb: f64,
    cog_file: Option<CacheObjectDetail>,
    png_tiles: Vec<CacheObjectDetail>,
}

/// Days covered by the summary's weekly total and its daily breakdown.
const STATS_WINDOW_DAYS: i64 = 7;

/// Inclusive date range covering the `days` most recent days, ending on `today`.
fn recent_days(today: chrono::NaiveDate, days: i64) -> (chrono::NaiveDate, chrono::NaiveDate) {
    (today - chrono::Duration::days(days - 1), today)
}

/// The days of an inclusive range, oldest first.
fn days_in_range(from: chrono::NaiveDate, to: chrono::NaiveDate) -> Vec<chrono::NaiveDate> {
    from.iter_days().take_while(|d| *d <= to).collect()
}

/// GET /api/admin/stats/summary - Dashboard overview
async fn get_stats_summary(
    State(app_state): State<AppState>,
) -> Result<Json<StatsSummary>, StatusCode> {
    use super::db::layer_statistics;
    use crate::routes::layers::db as layer;

    let db = &app_state.db;
    let today = chrono::Utc::now().naive_utc().date();
    let (week_start, _) = recent_days(today, STATS_WINDOW_DAYS);
    let day_ago = chrono::Utc::now() - chrono::Duration::hours(24);

    // Total requests all time
    let all_stats = layer_statistics::Entity::find().all(db).await.map_err(|e| {
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
        })
        .sum();

    // Total requests today
    let today_stats = layer_statistics::Entity::find()
        .filter(layer_statistics::Column::StatDate.eq(today))
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let total_requests_today: i64 = today_stats
        .iter()
        .map(|s| {
            s.xyz_tile_count as i64
                + s.cog_download_count as i64
                + s.pixel_query_count as i64
                + s.stac_request_count as i64
        })
        .sum();

    // Breakdown by request type for today
    let xyz_tile_count_today: i64 = today_stats.iter().map(|s| s.xyz_tile_count as i64).sum();
    let cog_download_count_today: i64 = today_stats.iter().map(|s| s.cog_download_count as i64).sum();
    let pixel_query_count_today: i64 = today_stats.iter().map(|s| s.pixel_query_count as i64).sum();
    let stac_request_count_today: i64 = today_stats.iter().map(|s| s.stac_request_count as i64).sum();

    // Total requests this week
    let week_stats = layer_statistics::Entity::find()
        .filter(layer_statistics::Column::StatDate.gte(week_start))
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let total_requests_week: i64 = week_stats
        .iter()
        .map(|s| {
            s.xyz_tile_count as i64
                + s.cog_download_count as i64
                + s.pixel_query_count as i64
                + s.stac_request_count as i64
        })
        .sum();

    // Most accessed layer, over the same window as the totals
    let mut layer_totals: HashMap<uuid::Uuid, i64> = HashMap::new();
    for stat in &week_stats {
        let total = stat.xyz_tile_count as i64
            + stat.cog_download_count as i64
            + stat.pixel_query_count as i64
            + stat.stac_request_count as i64;
        *layer_totals.entry(stat.layer_id).or_insert(0) += total;
    }

    let most_accessed_layer = if let Some((layer_id, total)) = layer_totals.iter().max_by_key(|&(_, v)| v) {
        let layer_record = layer::Entity::find_by_id(*layer_id)
            .one(db)
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
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .iter()
        .map(|s| s.layer_id)
        .collect::<std::collections::HashSet<_>>()
        .len() as i64;

    // Total layers
    let total_layers = layer::Entity::find()
        .count(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? as i64;

    // Daily breakdown over the same window, for charts
    let mut daily_requests = Vec::new();
    for date in days_in_range(week_start, today) {
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
        daily_requests,
    }))
}

/// The column a statistics list sort orders on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatsSort {
    LastAccessedAt,
    XyzTileCount,
    CogDownloadCount,
    PixelQueryCount,
    StacRequestCount,
    LayerName,
    TotalRequests,
}

fn aggregated_total_requests_expr() -> sea_orm::sea_query::SimpleExpr {
    use sea_orm::sea_query::Expr;
    Expr::col(layer_statistics::Column::XyzTileCount)
        .sum()
        .add(Expr::col(layer_statistics::Column::CogDownloadCount).sum())
        .add(Expr::col(layer_statistics::Column::PixelQueryCount).sum())
        .add(Expr::col(layer_statistics::Column::StacRequestCount).sum())
}

fn stats_sort_target(field: Option<&str>) -> Option<StatsSort> {
    match field {
        Some("last_accessed_at") => Some(StatsSort::LastAccessedAt),
        Some("xyz_tile_count") => Some(StatsSort::XyzTileCount),
        Some("cog_download_count") => Some(StatsSort::CogDownloadCount),
        Some("pixel_query_count") => Some(StatsSort::PixelQueryCount),
        Some("stac_request_count") => Some(StatsSort::StacRequestCount),
        Some("layer_name") => Some(StatsSort::LayerName),
        Some("total_requests") => Some(StatsSort::TotalRequests),
        None => Some(StatsSort::LastAccessedAt),
        Some(_) => None,
    }
}

/// GET /api/admin/stats/layers - All layer statistics
async fn get_layer_stats(
    State(app_state): State<AppState>,
    Query(params): Query<StatsQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    use super::db::layer_statistics;
    use crate::routes::layers::db as layer;
    use sea_orm::Order;

    let db = &app_state.db;

    let filter = parse_stats_filter(params.filter.as_deref())?;

    // Parse range using crudcrate utility (returns [start, end])
    let (start, end) = crudcrate::parse_range(params.range.clone());
    let limit = end - start + 1;
    let offset = start;

    // Parse sort JSON if provided ["field", "ASC"|"DESC"]
    let (sort_field, sort_order) = if let Some(sort_str) = &params.sort {
        if let Ok(sort) = serde_json::from_str::<Vec<String>>(sort_str) {
            if sort.len() == 2 {
                let order = if sort[1].to_uppercase() == "ASC" { Order::Asc } else { Order::Desc };
                (Some(sort[0].clone()), order)
            } else {
                (None, Order::Desc)
            }
        } else {
            (None, Order::Desc)
        }
    } else {
        (None, Order::Desc)
    };

    let mut query = layer_statistics::Entity::find().join(
        sea_orm::JoinType::InnerJoin,
        layer_statistics::Relation::Layer.def(),
    );

    // Apply layer filters
    if let Some(ref f) = filter {
        if let Some(layer_id) = f.layer_id {
            query = query.filter(layer_statistics::Column::LayerId.eq(layer_id));
        }
        if let Some(ref layer_name) = f.layer_name {
            debug!(layer_name, "Filtering statistics by layer_name");
            query = query.filter(layer::Column::LayerName.eq(layer_name));
        }

        // Apply date filters
        if let Some(start) = f.start_date {
            query = query.filter(layer_statistics::Column::StatDate.gte(start));
        }

        if let Some(end) = f.end_date {
            query = query.filter(layer_statistics::Column::StatDate.lte(end));
        }
    } else {
        debug!("No filter provided");
    }

    // Count layers after filtering, before cutting the page.
    let total_count = query
        .clone()
        .select_only()
        .column(layer_statistics::Column::LayerId)
        .distinct()
        .count(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? as usize;

    let Some(sort_target) = stats_sort_target(sort_field.as_deref()) else {
        warn!(field = ?sort_field, "Unknown statistics sort field");
        return Err(StatusCode::BAD_REQUEST);
    };
    let query = match sort_target {
        StatsSort::LastAccessedAt => query.order_by(
            sea_orm::sea_query::Expr::col(layer_statistics::Column::LastAccessedAt).max(),
            sort_order,
        ),
        StatsSort::XyzTileCount => query.order_by(
            sea_orm::sea_query::Expr::col(layer_statistics::Column::XyzTileCount).sum(),
            sort_order,
        ),
        StatsSort::CogDownloadCount => query.order_by(
            sea_orm::sea_query::Expr::col(layer_statistics::Column::CogDownloadCount).sum(),
            sort_order,
        ),
        StatsSort::PixelQueryCount => query.order_by(
            sea_orm::sea_query::Expr::col(layer_statistics::Column::PixelQueryCount).sum(),
            sort_order,
        ),
        StatsSort::StacRequestCount => query.order_by(
            sea_orm::sea_query::Expr::col(layer_statistics::Column::StacRequestCount).sum(),
            sort_order,
        ),
        StatsSort::LayerName => query.order_by(layer::Column::LayerName, sort_order),
        StatsSort::TotalRequests => query.order_by(aggregated_total_requests_expr(), sort_order),
    };

    let stats = query
        .select_only()
        .column(layer_statistics::Column::LayerId)
        .column(layer::Column::LayerName)
        .column_as(
            sea_orm::sea_query::Expr::col(layer_statistics::Column::LastAccessedAt).max(),
            "last_accessed_at",
        )
        .column_as(
            sea_orm::sea_query::Expr::col(layer_statistics::Column::XyzTileCount).sum(),
            "xyz_tile_count",
        )
        .column_as(
            sea_orm::sea_query::Expr::col(layer_statistics::Column::CogDownloadCount).sum(),
            "cog_download_count",
        )
        .column_as(
            sea_orm::sea_query::Expr::col(layer_statistics::Column::PixelQueryCount).sum(),
            "pixel_query_count",
        )
        .column_as(
            sea_orm::sea_query::Expr::col(layer_statistics::Column::StacRequestCount).sum(),
            "stac_request_count",
        )
        .group_by(layer_statistics::Column::LayerId)
        .group_by(layer::Column::LayerName)
        .limit(limit)
        .offset(offset)
        .into_model::<AggregatedLayerStat>()
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let results: Vec<LayerStatSummary> = stats
        .into_iter()
        .map(|stat| {
            let id = stat.layer_id.to_string();
            LayerStatSummary {
                id: id.clone(),
                layer_id: id,
                layer_name: stat.layer_name.unwrap_or_else(|| stat.layer_id.to_string()),
                last_accessed_at: stat.last_accessed_at.to_rfc3339(),
                xyz_tile_count: stat.xyz_tile_count,
                cog_download_count: stat.cog_download_count,
                pixel_query_count: stat.pixel_query_count,
                stac_request_count: stat.stac_request_count,
                total_requests: stat.xyz_tile_count
                    + stat.cog_download_count
                    + stat.pixel_query_count
                    + stat.stac_request_count,
            }
        })
        .collect();

    // Build Content-Range header using crudcrate utility
    let mut headers = crudcrate::calculate_content_range(offset, limit, total_count as u64, "statistics");
    headers.insert("Access-Control-Expose-Headers", "Content-Range".parse().unwrap());

    Ok((headers, Json(results)))
}

async fn get_daily_stats(
    State(app_state): State<AppState>,
    Query(params): Query<StatsQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    use crate::routes::layers::db as layer;
    use sea_orm::Order;

    let db = &app_state.db;
    let filter = parse_stats_filter(params.filter.as_deref())?;
    let (start, end) = crudcrate::parse_range(params.range.clone());
    let limit = end - start + 1;
    let offset = start;
    let sort_order = params
        .sort
        .as_ref()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .and_then(|sort| sort.get(1).cloned())
        .map_or(Order::Desc, |order| {
            if order.eq_ignore_ascii_case("ASC") { Order::Asc } else { Order::Desc }
        });

    let mut query = layer_statistics::Entity::find();
    if let Some(filter) = filter {
        if let Some(layer_id) = filter.layer_id {
            query = query.filter(layer_statistics::Column::LayerId.eq(layer_id));
        }
        if let Some(layer_name) = filter.layer_name {
            query = query
                .join(
                    sea_orm::JoinType::InnerJoin,
                    layer_statistics::Relation::Layer.def(),
                )
                .filter(layer::Column::LayerName.eq(layer_name));
        }
        if let Some(start) = filter.start_date {
            query = query.filter(layer_statistics::Column::StatDate.gte(start));
        }
        if let Some(end) = filter.end_date {
            query = query.filter(layer_statistics::Column::StatDate.lte(end));
        }
    }

    let total_count = query
        .clone()
        .count(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? as usize;
    let stats = query
        .order_by(layer_statistics::Column::StatDate, sort_order)
        .limit(limit)
        .offset(offset)
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let layer_ids: Vec<uuid::Uuid> = stats.iter().map(|stat| stat.layer_id).collect();
    let layers = layer::Entity::find()
        .filter(layer::Column::Id.is_in(layer_ids))
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let layer_names: HashMap<uuid::Uuid, String> = layers
        .into_iter()
        .map(|layer| (layer.id, layer.layer_name.unwrap_or_else(|| layer.id.to_string())))
        .collect();
    let results: Vec<LayerStatDetail> = stats
        .into_iter()
        .filter_map(|stat| {
            layer_names.get(&stat.layer_id).map(|layer_name| LayerStatDetail {
                id: stat.id.to_string(),
                layer_id: stat.layer_id.to_string(),
                layer_name: layer_name.clone(),
                stat_date: stat.stat_date.to_string(),
                last_accessed_at: stat.last_accessed_at.to_rfc3339(),
                xyz_tile_count: stat.xyz_tile_count,
                cog_download_count: stat.cog_download_count,
                pixel_query_count: stat.pixel_query_count,
                stac_request_count: stat.stac_request_count,
                cache_hit_count: stat.cache_hit_count,
                cache_miss_count: stat.cache_miss_count,
                total_requests: stat.xyz_tile_count
                    + stat.cog_download_count
                    + stat.pixel_query_count
                    + stat.stac_request_count,
            })
        })
        .collect();
    let mut headers = crudcrate::calculate_content_range(offset, limit, total_count as u64, "statistics");
    headers.insert("Access-Control-Expose-Headers", "Content-Range".parse().unwrap());

    Ok((headers, Json(results)))
}

/// GET /api/statistics/:id - Get single statistic by ID (for React Admin)
async fn get_layer_stat_detail(
    State(app_state): State<AppState>,
    Path(stat_id): Path<String>,
) -> Result<Json<LayerStatDetail>, StatusCode> {
    use super::db::layer_statistics;
    use crate::routes::layers::db as layer;

    let db = &app_state.db;
    let stat_uuid = uuid::Uuid::parse_str(&stat_id).map_err(|_| StatusCode::BAD_REQUEST)?;

    let stat = layer_statistics::Entity::find_by_id(stat_uuid)
        .one(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Fetch layer name
    let layer_record = layer::Entity::find_by_id(stat.layer_id)
        .one(db)
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
        cache_hit_count: stat.cache_hit_count,
        cache_miss_count: stat.cache_miss_count,
        total_requests: stat.xyz_tile_count
            + stat.cog_download_count
            + stat.pixel_query_count
            + stat.stac_request_count,
    };

    Ok(Json(result))
}

/// GET /api/admin/statistics/:stat_id/timeline - Time-series data for charts
/// This gets the timeline for the layer associated with the given statistic record.
/// Returns a continuous date range from the first recorded date to today,
/// filling in days with no traffic with zero values.
async fn get_layer_timeline(
    State(app_state): State<AppState>,
    Path(stat_id): Path<String>,
) -> Result<Json<Vec<LayerStatDetail>>, StatusCode> {
    let db = &app_state.db;
    // First get the statistic record to find the layer_id
    let stat_uuid = uuid::Uuid::parse_str(&stat_id)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let stat = layer_statistics::Entity::find_by_id(stat_uuid)
        .one(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Now get all statistics for this layer, ordered by date
    let stats = layer_statistics::Entity::find()
        .filter(layer_statistics::Column::LayerId.eq(stat.layer_id))
        .order_by_asc(layer_statistics::Column::StatDate)
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Get the layer name
    let layer = crate::routes::layers::db::Entity::find_by_id(stat.layer_id)
        .one(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let layer_name = layer.layer_name.unwrap_or_default();
    let layer_id_str = stat.layer_id.to_string();

    // Build a map of date -> stats for quick lookup
    let stats_map: std::collections::HashMap<chrono::NaiveDate, &layer_statistics::Model> = stats
        .iter()
        .map(|s| (s.stat_date, s))
        .collect();

    // Determine date range: from earliest record to today
    let today = chrono::Utc::now().naive_utc().date();
    let start_date = stats.first().map(|s| s.stat_date).unwrap_or(today);

    // Generate continuous date range with all days filled in
    let mut results: Vec<LayerStatDetail> = Vec::new();
    let mut current_date = start_date;

    while current_date <= today {
        let date_str = current_date.to_string();

        if let Some(s) = stats_map.get(&current_date) {
            // We have data for this day
            results.push(LayerStatDetail {
                id: s.id.to_string(),
                layer_id: layer_id_str.clone(),
                layer_name: layer_name.clone(),
                stat_date: date_str,
                last_accessed_at: s.last_accessed_at.to_rfc3339(),
                xyz_tile_count: s.xyz_tile_count,
                cog_download_count: s.cog_download_count,
                pixel_query_count: s.pixel_query_count,
                stac_request_count: s.stac_request_count,
                cache_hit_count: s.cache_hit_count,
                cache_miss_count: s.cache_miss_count,
                total_requests: s.xyz_tile_count + s.cog_download_count + s.pixel_query_count + s.stac_request_count,
            });
        } else {
            // No data for this day - fill with zeros
            results.push(LayerStatDetail {
                id: format!("synthetic-{}-{}", layer_id_str, date_str),
                layer_id: layer_id_str.clone(),
                layer_name: layer_name.clone(),
                stat_date: date_str,
                last_accessed_at: String::new(),
                xyz_tile_count: 0,
                cog_download_count: 0,
                pixel_query_count: 0,
                stac_request_count: 0,
                cache_hit_count: 0,
                cache_miss_count: 0,
                total_requests: 0,
            });
        }

        current_date += chrono::Duration::days(1);
    }

    Ok(Json(results))
}

/// GET /api/admin/cache/info - Cache statistics
async fn get_cache_info(
    State(app_state): State<AppState>,
) -> Result<Json<CacheInfo>, StatusCode> {
    let config = &app_state.config;
    let redis_client = crate::routes::tiles::cache::get_redis_client(config);

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
            let all_keys: Vec<String> = crate::routes::tiles::cache::scan_keys(&mut con, &cache_pattern).await.unwrap_or_default();
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
    State(app_state): State<AppState>,
) -> Result<Json<Vec<CachedLayer>>, StatusCode> {
    let db = &app_state.db;
    let config = &app_state.config;
    let redis_client = crate::routes::tiles::cache::get_redis_client(config);

    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Match actual cache key pattern: {app}-{deployment}/{filename}
    // Exclude stats and lock keys
    let cache_pattern = format!("{}-{}/*", config.app_name, config.deployment);
    let all_keys = crate::routes::tiles::cache::scan_keys(&mut con, &cache_pattern)
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
            .one(db)
            .await
            .ok()
            .flatten()
            .map(|l| l.id);

        // If not found, try with .tif extension
        let layer_id = if layer_id.is_none() && !layer_name.ends_with(".tif") {
            layer::Entity::find()
                .filter(layer::Column::LayerName.eq(format!("{}.tif", layer_name)))
                .one(db)
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
                .one(db)
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

/// GET /api/admin/cache/aggregated - Aggregated cache data grouped by layer
///
/// Parses Redis cache keys into four categories and groups by layer name:
///   COG file:   `{prefix}/{layer}.tif`  or  `{prefix}/{project_uuid}/{layer}.tif`
///   XYZ tile:   `{prefix}/png/{layer}/{style_id}/{z}/{x}/{y}`
///   Globe tile: `{prefix}/png-globe/{layer}/{z}/{x}/{y}`
///   Card tile:  `{prefix}/png-card/{project_slug}/{layer}/{z}/{x}/{y}`
async fn get_cache_aggregated(
    State(app_state): State<AppState>,
) -> Result<Json<Vec<AggregatedCacheEntry>>, StatusCode> {
    let db = &app_state.db;
    let config = &app_state.config;
    let redis_client = crate::routes::tiles::cache::get_redis_client(config);

    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let cache_pattern = format!("{}-{}/*", config.app_name, config.deployment);
    let all_keys = crate::routes::tiles::cache::scan_keys(&mut con, &cache_pattern)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let prefix = format!("{}-{}/", config.app_name, config.deployment);
    let keys: Vec<String> = all_keys
        .into_iter()
        .filter(|k| !k.contains("/stats:") && !k.ends_with(":downloading"))
        .collect();

    // Accumulator: layer_name → (cog_size, cog_ttl, png_count, png_size)
    let mut map: HashMap<String, (usize, Option<f64>, usize, usize)> = HashMap::new();

    for key in &keys {
        let stem = key.strip_prefix(&prefix).unwrap_or(key);

        let size_bytes: usize = redis::cmd("STRLEN")
            .arg(key)
            .query_async(&mut con)
            .await
            .unwrap_or(0);

        let ttl_seconds: i64 = redis::cmd("TTL")
            .arg(key)
            .query_async(&mut con)
            .await
            .unwrap_or(-2);

        let ttl_hours = if ttl_seconds > 0 {
            Some(ttl_seconds as f64 / 3600.0)
        } else {
            None
        };

        // Parse key to determine type and extract layer name
        if let Some(rest) = stem.strip_prefix("png/") {
            // XYZ tile: png/{layer}/{style_id}/{z}/{x}/{y}
            if let Some(layer_name) = rest.split('/').next() {
                let entry = map.entry(layer_name.to_string()).or_insert((0, None, 0, 0));
                entry.2 += 1;
                entry.3 += size_bytes;
            }
        } else if let Some(rest) = stem.strip_prefix("png-globe/") {
            // Globe tile: png-globe/{layer}/{z}/{x}/{y}
            if let Some(layer_name) = rest.split('/').next() {
                let entry = map.entry(layer_name.to_string()).or_insert((0, None, 0, 0));
                entry.2 += 1;
                entry.3 += size_bytes;
            }
        } else if let Some(rest) = stem.strip_prefix("png-card/") {
            // Card tile: png-card/{project_slug}/{layer}/{z}/{x}/{y}
            let parts: Vec<&str> = rest.splitn(3, '/').collect();
            if parts.len() >= 2 {
                let layer_name = parts[1];
                let entry = map.entry(layer_name.to_string()).or_insert((0, None, 0, 0));
                entry.2 += 1;
                entry.3 += size_bytes;
            }
        } else {
            // COG file: {layer}.tif  or  {project_uuid}/{layer}.tif
            let filename_part = if let Some(slash_pos) = stem.find('/') {
                let maybe_uuid = &stem[..slash_pos];
                if uuid::Uuid::parse_str(maybe_uuid).is_ok() {
                    &stem[slash_pos + 1..]
                } else {
                    stem
                }
            } else {
                stem
            };
            let layer_name = filename_part.trim_end_matches(".tif").to_string();
            let entry = map.entry(layer_name).or_insert((0, None, 0, 0));
            entry.0 = size_bytes;
            entry.1 = ttl_hours;
        }
    }

    let mut entries: Vec<AggregatedCacheEntry> = map
        .into_iter()
        .map(|(layer_name, (cog_size, cog_ttl, png_count, png_size))| {
            let total = cog_size + png_size;
            AggregatedCacheEntry {
                layer_name,
                layer_id: None,
                total_size_bytes: total,
                total_size_mb: total as f64 / (1024.0 * 1024.0),
                cog_cached: cog_size > 0,
                cog_size_mb: cog_size as f64 / (1024.0 * 1024.0),
                cog_ttl_hours: cog_ttl,
                png_tile_count: png_count,
                png_tile_size_mb: png_size as f64 / (1024.0 * 1024.0),
            }
        })
        .collect();

    // Batch-resolve layer_ids from database
    use crate::routes::layers::db as layer;
    let layer_names: Vec<String> = entries.iter().map(|e| e.layer_name.clone()).collect();
    let layers = layer::Entity::find()
        .filter(layer::Column::LayerName.is_in(layer_names))
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let layer_map: HashMap<String, uuid::Uuid> = layers
        .into_iter()
        .filter_map(|l| l.layer_name.map(|name| (name, l.id)))
        .collect();

    for entry in &mut entries {
        entry.layer_id = layer_map.get(&entry.layer_name).copied();
    }

    entries.sort_by(|a, b| b.total_size_bytes.cmp(&a.total_size_bytes));

    Ok(Json(entries))
}

/// GET /api/admin/cache/layers/:layer_name/detail - All cached objects for one layer.
async fn get_layer_cache_detail(
    State(app_state): State<AppState>,
    Path(layer_name): Path<String>,
) -> Result<Json<LayerCacheDetail>, StatusCode> {
    let config = &app_state.config;
    let redis_client = crate::routes::tiles::cache::get_redis_client(config);
    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let prefix = format!("{}-{}/", config.app_name, config.deployment);
    let pattern = format!("{}*", prefix);
    let mut matching_keys: Vec<(String, LayerCacheEntryKind)> =
        crate::routes::tiles::cache::scan_keys(&mut con, &pattern)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .into_iter()
            .filter_map(|key| {
                cache_entry_for_layer(&key, &prefix, &layer_name).map(|kind| (key, kind))
            })
            .collect();
    matching_keys.sort_by(|left, right| left.0.cmp(&right.0));

    let mut cog_file = None;
    let mut png_tiles = Vec::new();
    let mut total_size_bytes = 0usize;

    for (cache_key, kind) in matching_keys {
        if matches!(kind, LayerCacheEntryKind::Cog) && cog_file.is_some() {
            continue;
        }
        let size_bytes: usize = redis::cmd("STRLEN")
            .arg(&cache_key)
            .query_async(&mut con)
            .await
            .unwrap_or(0);
        let redis_ttl: i64 = redis::cmd("TTL")
            .arg(&cache_key)
            .query_async(&mut con)
            .await
            .unwrap_or(-2);
        let ttl_seconds = (redis_ttl >= 0).then_some(redis_ttl);
        let ttl_hours = (redis_ttl > 0).then_some(redis_ttl as f64 / 3600.0);
        total_size_bytes += size_bytes;

        let tile_coords = match &kind {
            LayerCacheEntryKind::Cog => None,
            LayerCacheEntryKind::Png { tile_coords } => Some(tile_coords.clone()),
        };
        let detail = CacheObjectDetail {
            cache_key,
            size_bytes,
            size_mb: size_bytes as f64 / (1024.0 * 1024.0),
            ttl_seconds,
            ttl_hours,
            tile_coords,
        };

        match kind {
            LayerCacheEntryKind::Cog => {
                if cog_file.is_none() {
                    cog_file = Some(detail);
                }
            }
            LayerCacheEntryKind::Png { .. } => png_tiles.push(detail),
        }
    }

    let total_items = png_tiles.len() + usize::from(cog_file.is_some());
    Ok(Json(LayerCacheDetail {
        total_items,
        total_size_bytes,
        total_size_mb: total_size_bytes as f64 / (1024.0 * 1024.0),
        cog_file,
        png_tiles,
    }))
}

/// POST /api/admin/cache/clear - Clear all cache
async fn clear_all_cache(
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    let config = &app_state.config;
    let redis_client = crate::routes::tiles::cache::get_redis_client(config);

    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Match actual cache key pattern and filter out stats/lock keys
    let cache_pattern = format!("{}-{}/*", config.app_name, config.deployment);
    let all_keys = crate::routes::tiles::cache::scan_keys(&mut con, &cache_pattern)
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

    // Re-warm important tiles after clearing
    let warm_config = app_state.config.clone();
    let warm_db = app_state.db.clone();
    tokio::spawn(async move {
        crate::routes::tiles::warming::warm_all_important_tiles(&warm_config, &warm_db).await;
    });

    Ok(Json(json!({
        "message": format!("Cleared {} cached layers, re-warming important tiles", keys.len()),
        "keys_cleared": keys.len()
    })))
}

/// DELETE /api/admin/cache/layers/:layer_name - Clear specific layer cache
async fn clear_layer_cache(
    State(app_state): State<AppState>,
    Path(layer_name): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let config = &app_state.config;
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
async fn get_cache_ttl(
    State(app_state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let config = &app_state.config;
    Ok(Json(json!({
        "ttl_seconds": config.tile_cache_ttl,
        "ttl_hours": config.tile_cache_ttl / 3600
    })))
}

/// POST /api/admin/cache/layers/:layer_name/warm - Pre-warm cache for a layer
async fn warm_layer_cache(
    State(app_state): State<AppState>,
    Path(layer_name): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let config = &app_state.config;
    let db = &app_state.db;

    // Add .tif extension if not present
    let filename = if layer_name.ends_with(".tif") {
        layer_name.clone()
    } else {
        format!("{}.tif", layer_name)
    };

    // Resolve the layer's project_id so the cache warm targets the correct
    // project-scoped S3 object. A stem without an extension may have been
    // passed; strip it for the lookup.
    let layer_name_stem = layer_name.trim_end_matches(".tif");
    let project_id = crate::routes::layers::db::Entity::find()
        .filter(crate::routes::layers::db::Column::LayerName.eq(layer_name_stem))
        .one(db)
        .await
        .map_err(|e| {
            error!(layer_name, error = %e, "DB lookup failed for warm cache");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .and_then(|l| l.project_id);

    // Use the storage module to fetch and cache the layer
    match crate::routes::tiles::storage::get_object(&config, project_id, &filename).await {
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
async fn persist_layer_cache(
    State(app_state): State<AppState>,
    Path(layer_name): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let config = &app_state.config;
    let redis_client = crate::routes::tiles::cache::get_redis_client(config);

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
async fn unpersist_layer_cache(
    State(app_state): State<AppState>,
    Path(layer_name): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let config = &app_state.config;
    let redis_client = crate::routes::tiles::cache::get_redis_client(config);

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
    cache_hit_count: i64,
    cache_miss_count: i64,
    total_requests: i64,
}

impl LiveLayerStats {
    fn new(layer_name: String, date: String) -> Self {
        Self {
            layer_id: None,
            layer_name,
            date,
            xyz_tile_count: 0,
            cog_download_count: 0,
            pixel_query_count: 0,
            stac_request_count: 0,
            cache_hit_count: 0,
            cache_miss_count: 0,
            total_requests: 0,
        }
    }
}

/// GET /api/admin/stats/live - Today's statistics: the rows already synced to
/// Postgres plus the counts still sitting in Redis.
async fn get_live_stats(State(app_state): State<AppState>) -> Result<Json<Vec<LiveLayerStats>>, StatusCode> {
    use super::db::layer_statistics;
    use crate::routes::stats_sync::{find_layer, parse_layer_identifier};

    let db = &app_state.db;
    let config = &app_state.config;
    let redis_client = crate::routes::tiles::cache::get_redis_client(config);

    let mut con = redis_client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let stats_pattern = format!("{}-{}/stats:{}:*", config.app_name, config.deployment, today);

    let keys = crate::routes::tiles::cache::scan_keys(&mut con, &stats_pattern)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // What Redis still holds, keyed by whatever the increment named the layer.
    let mut pending_by_identifier: HashMap<String, LiveLayerStats> = HashMap::new();

    for key in keys {
        if let Some((date, identifier, stat_type)) = parse_live_stats_key(&key, &config) {
            use redis::AsyncCommands;
            let count: i64 = con.get(&key).await.unwrap_or(0);

            let entry = pending_by_identifier
                .entry(identifier.clone())
                .or_insert_with(|| LiveLayerStats::new(identifier.clone(), date.clone()));

            match stat_type.as_str() {
                "xyz" => entry.xyz_tile_count += count,
                "cog" => entry.cog_download_count += count,
                "pixel" => entry.pixel_query_count += count,
                "stac" => entry.stac_request_count += count,
                // A cache outcome is a property of a tile request already counted,
                // so it stays out of the total.
                "hit" => {
                    entry.cache_hit_count += count;
                    continue;
                }
                "miss" => {
                    entry.cache_miss_count += count;
                    continue;
                }
                _ => {}
            }

            entry.total_requests += count;
        }
    }

    // Resolve each identifier, so a key written under a layer UUID lands on the
    // same layer as one written under its name.
    let mut pending: Vec<LiveLayerStats> = Vec::with_capacity(pending_by_identifier.len());
    for (identifier, mut stats) in pending_by_identifier {
        if let Ok(Some(layer)) = find_layer(db, &parse_layer_identifier(&identifier)).await {
            stats.layer_id = Some(layer.id.to_string());
            if let Some(name) = layer.layer_name {
                stats.layer_name = name;
            }
        }
        pending.push(stats);
    }

    let today_date = chrono::Utc::now().date_naive();
    let rows = layer_statistics::Entity::find()
        .filter(layer_statistics::Column::StatDate.eq(today_date))
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut stored: Vec<LiveLayerStats> = Vec::with_capacity(rows.len());
    for row in rows {
        let layer = crate::routes::layers::db::Entity::find_by_id(row.layer_id)
            .one(db)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let Some(layer) = layer else { continue };
        let Some(layer_name) = layer.layer_name else { continue };

        stored.push(LiveLayerStats {
            layer_id: Some(layer.id.to_string()),
            layer_name,
            date: today.clone(),
            xyz_tile_count: i64::from(row.xyz_tile_count),
            cog_download_count: i64::from(row.cog_download_count),
            pixel_query_count: i64::from(row.pixel_query_count),
            stac_request_count: i64::from(row.stac_request_count),
            cache_hit_count: i64::from(row.cache_hit_count),
            cache_miss_count: i64::from(row.cache_miss_count),
            total_requests: i64::from(row.xyz_tile_count)
                + i64::from(row.cog_download_count)
                + i64::from(row.pixel_query_count)
                + i64::from(row.stac_request_count),
        });
    }

    Ok(Json(merge_live_stats(stored, pending)))
}

/// Adds the counts still in Redis onto the day's stored counts, layer by layer,
/// busiest first.
fn merge_live_stats(stored: Vec<LiveLayerStats>, pending: Vec<LiveLayerStats>) -> Vec<LiveLayerStats> {
    let mut merged: HashMap<String, LiveLayerStats> = HashMap::new();

    for stats in stored.into_iter().chain(pending) {
        match merged.get_mut(&stats.layer_name) {
            Some(entry) => {
                entry.xyz_tile_count += stats.xyz_tile_count;
                entry.cog_download_count += stats.cog_download_count;
                entry.pixel_query_count += stats.pixel_query_count;
                entry.stac_request_count += stats.stac_request_count;
                entry.cache_hit_count += stats.cache_hit_count;
                entry.cache_miss_count += stats.cache_miss_count;
                entry.total_requests += stats.total_requests;
                if entry.layer_id.is_none() {
                    entry.layer_id = stats.layer_id.clone();
                }
            }
            None => {
                merged.insert(stats.layer_name.clone(), stats);
            }
        }
    }

    let mut results: Vec<LiveLayerStats> = merged.into_values().collect();
    results.sort_by(|a, b| b.total_requests.cmp(&a.total_requests));
    results
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn test_recent_days_seven_days_ends_today_and_spans_seven() {
        let (from, to) = recent_days(date(2026, 9, 9), 7);
        assert_eq!(to, date(2026, 9, 9));
        assert_eq!(from, date(2026, 9, 3));
        // Inclusive on both ends, so the span is exactly seven days
        assert_eq!((to - from).num_days() + 1, 7);
    }

    #[test]
    fn test_recent_days_boundary_excludes_the_day_before_from() {
        let (from, _to) = recent_days(date(2026, 9, 9), 7);
        assert_eq!(from, date(2026, 9, 3));
        assert!(date(2026, 9, 3) >= from);
        assert!(date(2026, 9, 2) < from);
    }

    #[test]
    fn test_recent_days_one_day_is_today_alone() {
        let (from, to) = recent_days(date(2026, 9, 9), 1);
        assert_eq!(from, to);
        assert_eq!(from, date(2026, 9, 9));
    }

    #[test]
    fn test_recent_days_crosses_a_month_boundary() {
        let (from, to) = recent_days(date(2026, 3, 2), 7);
        assert_eq!(to, date(2026, 3, 2));
        assert_eq!(from, date(2026, 2, 24));
    }

    #[test]
    fn test_days_in_range_covers_the_window_oldest_first() {
        let (from, to) = recent_days(date(2026, 9, 9), 7);
        let bars = days_in_range(from, to);
        assert_eq!(bars.len(), 7);
        assert_eq!(*bars.first().unwrap(), from);
        assert_eq!(*bars.last().unwrap(), to);
        assert_eq!(
            bars,
            vec![
                date(2026, 9, 3),
                date(2026, 9, 4),
                date(2026, 9, 5),
                date(2026, 9, 6),
                date(2026, 9, 7),
                date(2026, 9, 8),
                date(2026, 9, 9),
            ]
        );
    }

    #[test]
    fn test_days_in_range_single_day() {
        let day = date(2026, 9, 9);
        assert_eq!(days_in_range(day, day), vec![day]);
    }

    #[test]
    fn test_days_in_range_inverted_is_empty() {
        assert!(days_in_range(date(2026, 9, 9), date(2026, 9, 8)).is_empty());
    }


    fn date_str(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn test_parse_stats_filter_absent() {
        assert!(parse_stats_filter(None).unwrap().is_none());
    }

    #[test]
    fn test_parse_stats_filter_empty_object() {
        let filter = parse_stats_filter(Some("{}")).unwrap().unwrap();
        assert!(filter.layer_id.is_none());
        assert!(filter.layer_name.is_none());
        assert!(filter.start_date.is_none());
        assert!(filter.end_date.is_none());
    }

    #[test]
    fn test_parse_stats_filter_valid() {
        let filter = parse_stats_filter(Some(
            r#"{"layer_id":"650e8400-e29b-41d4-a716-446655440001","layer_name":"wheat","start_date":"2026-09-01","end_date":"2026-09-04"}"#,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(
            filter.layer_id,
            Some(
                uuid::Uuid::parse_str("650e8400-e29b-41d4-a716-446655440001").unwrap()
            )
        );
        assert_eq!(filter.layer_name.as_deref(), Some("wheat"));
        assert_eq!(filter.start_date, Some(date_str("2026-09-01")));
        assert_eq!(filter.end_date, Some(date_str("2026-09-04")));
    }

    #[test]
    fn test_parse_stats_filter_unparseable_date_is_rejected() {
        assert_eq!(
            parse_stats_filter(Some(r#"{"start_date":"not-a-date"}"#)).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            parse_stats_filter(Some(r#"{"end_date":"2026-13-01"}"#)).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn test_parse_stats_filter_unparseable_layer_id_is_rejected() {
        assert_eq!(
            parse_stats_filter(Some(r#"{"layer_id":"not-a-uuid"}"#)).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn test_parse_stats_filter_wrong_type_is_rejected() {
        // A number where a date string belongs used to discard the whole filter,
        // layer_name included.
        assert_eq!(
            parse_stats_filter(Some(r#"{"start_date":20260901}"#)).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            parse_stats_filter(Some(r#"{"layer_name":"wheat","start_date":20260901}"#)).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn test_parse_stats_filter_malformed_json_is_rejected() {
        assert_eq!(
            parse_stats_filter(Some("{not json")).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn test_parse_stats_filter_one_bound_only() {
        let filter = parse_stats_filter(Some(r#"{"start_date":"2026-09-01"}"#))
            .unwrap()
            .unwrap();
        assert_eq!(filter.start_date, Some(date_str("2026-09-01")));
        assert!(filter.end_date.is_none());
    }


    /// The sources the statistics datagrid renders, each of which react-admin
    /// offers a sort on.
    const DATAGRID_SOURCES: [&str; 3] = [
        "layer_name",
        "total_requests",
        "last_accessed_at",
    ];

    #[test]
    fn test_stats_sort_target_covers_every_datagrid_column() {
        assert_eq!(
            stats_sort_target(Some("layer_name")),
            Some(StatsSort::LayerName)
        );
        assert_eq!(
            stats_sort_target(Some("total_requests")),
            Some(StatsSort::TotalRequests)
        );
        assert_eq!(
            stats_sort_target(Some("last_accessed_at")),
            Some(StatsSort::LastAccessedAt)
        );
        let mut targets: Vec<Option<StatsSort>> =
            DATAGRID_SOURCES.iter().map(|s| stats_sort_target(Some(s))).collect();
        targets.dedup();
        assert_eq!(targets.len(), DATAGRID_SOURCES.len(), "two columns share a sort target");
    }

    #[test]
    fn test_stats_sort_target_counters() {
        assert_eq!(
            stats_sort_target(Some("xyz_tile_count")),
            Some(StatsSort::XyzTileCount)
        );
    }

    #[test]
    fn test_stats_sort_target_defaults_without_a_field() {
        assert_eq!(stats_sort_target(None), Some(StatsSort::LastAccessedAt));
    }

    #[test]
    fn test_stats_sort_target_rejects_an_unknown_field() {
        assert_eq!(stats_sort_target(Some("id")), None);
        assert_eq!(stats_sort_target(Some("stat_date")), None);
        assert_eq!(stats_sort_target(Some("")), None);
    }

    fn stats(name: &str, xyz: i64) -> LiveLayerStats {
        LiveLayerStats {
            layer_id: Some(uuid::Uuid::nil().to_string()),
            layer_name: name.to_string(),
            date: "2026-09-09".to_string(),
            xyz_tile_count: xyz,
            cog_download_count: 0,
            pixel_query_count: 0,
            stac_request_count: 0,
            cache_hit_count: 0,
            cache_miss_count: 0,
            total_requests: xyz,
        }
    }

    // Scenario: the sync moves today's counts to Postgres every 30 seconds, so a
    // layer's day is split between the stored row and whatever Redis has taken
    // since. The card shows the day, which is both.
    #[test]
    fn test_merge_live_stats_adds_pending_to_stored() {
        let merged = merge_live_stats(vec![stats("barley", 6)], vec![stats("barley", 2)]);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].xyz_tile_count, 8);
        assert_eq!(merged[0].total_requests, 8);
    }

    #[test]
    fn test_merge_live_stats_keeps_layers_that_are_only_stored() {
        let merged = merge_live_stats(vec![stats("barley", 6)], vec![]);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].total_requests, 6);
    }

    #[test]
    fn test_merge_live_stats_keeps_layers_that_are_only_pending() {
        let merged = merge_live_stats(vec![], vec![stats("barley", 2)]);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].total_requests, 2);
    }

    #[test]
    fn test_merge_live_stats_orders_by_total_requests() {
        let merged = merge_live_stats(
            vec![stats("quiet", 1), stats("busy", 40)],
            vec![stats("quiet", 1)],
        );

        let names: Vec<&str> = merged.iter().map(|s| s.layer_name.as_str()).collect();
        assert_eq!(names, vec!["busy", "quiet"]);
    }

    #[test]
    fn test_merge_live_stats_takes_the_layer_id_from_whichever_side_has_it() {
        let mut stored = stats("barley", 6);
        stored.layer_id = None;

        let merged = merge_live_stats(vec![stored], vec![stats("barley", 2)]);

        assert_eq!(merged[0].layer_id, Some(uuid::Uuid::nil().to_string()));
    }

    #[test]
    fn test_merge_live_stats_empty() {
        assert!(merge_live_stats(vec![], vec![]).is_empty());
    }
}
