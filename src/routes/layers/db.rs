use chrono::{DateTime, Utc};
use crudcrate::{CRUDResource, EntityToModels, ApiError};
use sea_orm::EntityTrait;
use sea_orm::entity::prelude::*;
use tracing::debug;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct CacheStatus {
    pub cached: bool,
    pub cache_key: Option<String>,
    pub size_mb: Option<f64>,
    pub ttl_hours: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct LayerStats {
    pub total_requests: i32,
    pub xyz_tile_count: i32,
    pub cog_download_count: i32,
    pub pixel_query_count: i32,
    pub stac_request_count: i32,
    pub other_request_count: i32,
    pub last_accessed_at: Option<DateTime<Utc>>,
}

/// Status of the last statistics recalculation
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct StatsStatus {
    /// Status of the last recalculation: "success", "error", or "pending"
    pub status: String,
    /// Timestamp of when the stats were last calculated
    pub last_run: Option<DateTime<Utc>>,
    /// Error message if the last recalculation failed
    pub error: Option<String>,
    /// Additional details (e.g., file size at time of calculation)
    pub details: Option<String>,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, EntityToModels, serde::Serialize, serde::Deserialize)]
#[sea_orm(table_name = "layer")]
#[crudcrate(
    api_struct = "Layer",
    name_singular = "layer",
    name_plural = "layers",
    delete::many::body = delete_many,
    generate_router,
    operations = LayerOperations,
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    #[crudcrate(primary_key, exclude(update, create), on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[sea_orm(unique)]
    #[crudcrate(filterable, fulltext)]
    pub layer_name: Option<String>,
    #[crudcrate(filterable, fulltext)]
    pub crop: Option<String>,
    #[crudcrate(filterable, fulltext)]
    pub water_model: Option<String>,
    #[crudcrate(filterable, fulltext)]
    pub climate_model: Option<String>,
    #[crudcrate(filterable, fulltext)]
    pub scenario: Option<String>,
    #[crudcrate(filterable, fulltext)]
    pub variable: Option<String>,
    #[crudcrate(filterable, fulltext)]
    pub year: Option<i32>,
    #[crudcrate(filterable, sortable)]
    pub last_updated: DateTime<Utc>,
    #[crudcrate(filterable)]
    pub enabled: bool,
    pub uploaded_at: DateTime<Utc>,
    #[sea_orm(column_type = "Double", nullable)]
    #[crudcrate(sortable)]
    pub global_average: Option<f64>,
    pub filename: Option<String>,
    #[sea_orm(column_type = "Double", nullable)]
    pub min_value: Option<f64>,
    #[sea_orm(column_type = "Double", nullable)]
    pub max_value: Option<f64>,
    #[crudcrate(filterable)]
    pub style_id: Option<Uuid>,
    #[crudcrate(filterable)]
    pub is_crop_specific: bool,
    /// Total view count across all statistics (updated automatically by database trigger)
    #[crudcrate(sortable, filterable, exclude(create, update))]
    pub total_views: i64,
    /// Status of the last statistics recalculation (JSON with status, timestamp, error message)
    #[crudcrate(exclude(create, update))]
    pub stats_status: Option<serde_json::Value>,
    /// File size in bytes (from S3)
    #[crudcrate(sortable, exclude(create, update))]
    pub file_size: Option<i64>,
    // Metadata fields (populated by after_get_one hook, not stored in DB)
    #[sea_orm(ignore)]
    #[crudcrate(non_db_attr = true, exclude(create, update))]
    pub cache_status: Option<CacheStatus>,
    #[sea_orm(ignore)]
    #[crudcrate(non_db_attr = true, exclude(create, update))]
    pub stats: Option<LayerStats>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::countries::db::Entity")]
    Layercountrylink,
    #[sea_orm(
        belongs_to = "crate::routes::styles::db::Entity",
        from = "Column::StyleId",
        to = "crate::routes::styles::db::Column::Id",
        on_update = "NoAction",
        on_delete = "NoAction"
    )]
    Style,
}

impl Related<super::countries::db::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Layercountrylink.def()
    }
}

impl Related<crate::routes::styles::db::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Style.def()
    }
}

impl Related<crate::routes::countries::db::Entity> for Entity {
    fn to() -> RelationDef {
        super::countries::db::Relation::Country.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::countries::db::Relation::Layer.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}

// Operations struct for implementing hooks
pub struct LayerOperations;

// Implement CRUDOperations to add metadata enrichment hook
#[async_trait::async_trait]
impl crudcrate::CRUDOperations for LayerOperations {
    type Resource = Layer;

    /// Enrich layer data with cache status and stats after fetching
    async fn after_get_one(
        &self,
        db: &sea_orm::DatabaseConnection,
        entity: &mut Self::Resource,
    ) -> Result<(), ApiError> {
        // Fetch cache status from Redis (gracefully handle errors)
        if let Some(ref layer_name) = entity.layer_name {
            // Only try to fetch cache status if config is available
            if let Ok(config) = std::panic::catch_unwind(crate::config::Config::from_env) {
                entity.cache_status = fetch_cache_status_with_config(&config, layer_name).await.ok();
            }
        }

        // Fetch stats from database (this should always work if database is available)
        entity.stats = fetch_layer_stats(db, entity.id).await.ok().flatten();

        Ok(())
    }
}

/// Helper function to fetch cache status with provided config
async fn fetch_cache_status_with_config(
    config: &crate::config::Config,
    layer_name: &str,
) -> anyhow::Result<CacheStatus> {
    use crate::routes::tiles::cache;

    let redis_client = cache::get_redis_client(config);
    let mut con = redis_client.get_multiplexed_async_connection().await?;

    // Try to find the cache key - check with and without .tif extension
    let cache_key = cache::build_cache_key(config, layer_name);
    let cache_key_tif = cache::build_cache_key(config, &format!("{}.tif", layer_name));

    // Try to get TTL for the cache key
    let mut ttl_seconds: i64 = redis::cmd("TTL")
        .arg(&cache_key)
        .query_async(&mut con)
        .await?;

    let mut actual_key = cache_key.clone();

    // If not found, try with .tif extension
    if ttl_seconds == -2 {
        ttl_seconds = redis::cmd("TTL")
            .arg(&cache_key_tif)
            .query_async(&mut con)
            .await?;
        actual_key = cache_key_tif;
    }

    // TTL returns: -2 if key doesn't exist, -1 if key has no expiry (persistent), >= 0 if key has TTL
    if ttl_seconds != -2 {
        // Cache exists, get size
        let size_bytes: Option<usize> = redis::cmd("STRLEN")
            .arg(&actual_key)
            .query_async(&mut con)
            .await
            .ok();

        Ok(CacheStatus {
            cached: true,
            cache_key: Some(actual_key),
            size_mb: size_bytes.map(|bytes| bytes as f64 / (1024.0 * 1024.0)),
            // -1 means no expiry (persistent), show as None to indicate "permanent"
            ttl_hours: if ttl_seconds >= 0 { Some(ttl_seconds as f64 / 3600.0) } else { None },
        })
    } else {
        // Cache doesn't exist (TTL = -2)
        Ok(CacheStatus {
            cached: false,
            cache_key: None,
            size_mb: None,
            ttl_hours: None,
        })
    }
}

pub async fn delete_many(
    db: &sea_orm::DatabaseConnection,
    ids: Vec<Uuid>,
) -> Result<Vec<Uuid>, crudcrate::ApiError> {
    debug!(ids = ?ids, "Called delete_many");
    let config = crate::config::Config::from_env();
    let mut deleted_ids = Vec::new();

    for id in &ids {
        let _ = crate::routes::tiles::storage::delete_s3_object_by_db_id(&config, db, id).await;

        if Entity::delete_by_id(*id).exec(db).await.is_ok() {
            deleted_ids.push(*id);
        }
    }

    Ok(deleted_ids)
}

/// Helper function to fetch stats from database
async fn fetch_layer_stats(
    db: &sea_orm::DatabaseConnection,
    layer_id: Uuid,
) -> anyhow::Result<Option<LayerStats>> {
    use crate::routes::admin::db::layer_statistics;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    // Get all stats for this layer and aggregate
    let stats = layer_statistics::Entity::find()
        .filter(layer_statistics::Column::LayerId.eq(layer_id))
        .all(db)
        .await?;

    if stats.is_empty() {
        return Ok(None);
    }

    // Aggregate all stats
    let mut total_xyz = 0;
    let mut total_cog = 0;
    let mut total_pixel = 0;
    let mut total_stac = 0;
    let mut total_other = 0;
    let mut last_accessed: Option<DateTime<Utc>> = None;

    for stat in stats {
        total_xyz += stat.xyz_tile_count;
        total_cog += stat.cog_download_count;
        total_pixel += stat.pixel_query_count;
        total_stac += stat.stac_request_count;
        total_other += stat.other_request_count;

        // Track most recent access
        if last_accessed.is_none() || stat.last_accessed_at > last_accessed.unwrap() {
            last_accessed = Some(stat.last_accessed_at);
        }
    }

    Ok(Some(LayerStats {
        total_requests: total_xyz + total_cog + total_pixel + total_stac + total_other,
        xyz_tile_count: total_xyz,
        cog_download_count: total_cog,
        pixel_query_count: total_pixel,
        stac_request_count: total_stac,
        other_request_count: total_other,
        last_accessed_at: last_accessed,
    }))
}
