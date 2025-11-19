use chrono::{DateTime, Utc};
use crudcrate::{CRUDResource, EntityToModels};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, DeriveEntityModel, EntityToModels)]
#[sea_orm(table_name = "layer")]
#[crudcrate(
    api_struct = "Layer",
    name_singular = "layer",
    name_plural = "layers",
    fn_delete_many = delete_many,
    generate_router,
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    #[crudcrate(primary_key, exclude(update, create), on_create = Uuid::new_v4())]
    pub id: Uuid,
    #[sea_orm(unique)]
    #[crudcrate(filterable)]
    pub layer_name: Option<String>,
    #[crudcrate(filterable)]
    pub crop: Option<String>,
    #[crudcrate(filterable)]
    pub water_model: Option<String>,
    #[crudcrate(filterable)]
    pub climate_model: Option<String>,
    #[crudcrate(filterable)]
    pub scenario: Option<String>,
    #[crudcrate(filterable)]
    pub variable: Option<String>,
    #[crudcrate(filterable)]
    pub year: Option<i32>,
    #[crudcrate(filterable, sortable)]
    pub last_updated: DateTime<Utc>,
    #[crudcrate(filterable)]
    pub enabled: bool,
    pub uploaded_at: DateTime<Utc>,
    #[sea_orm(column_type = "Double", nullable, sortable)]
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

pub async fn delete_many(
    db: &sea_orm::DatabaseConnection,
    ids: Vec<Uuid>,
) -> Result<Vec<Uuid>, sea_orm::DbErr> {
    println!("Called delete_many with IDs: {:?}", ids);
    let mut deleted_ids = Vec::new();

    for id in &ids {
        let _ = crate::routes::tiles::storage::delete_s3_object_by_db_id(db, id).await;

        if Entity::delete_by_id(*id).exec(db).await.is_ok() {
            deleted_ids.push(*id);
        }
    }

    Ok(deleted_ids)
}

// Extended layer response with cache and stats metadata
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct LayerWithMetadata {
    // Layer fields
    pub id: Uuid,
    pub layer_name: Option<String>,
    pub crop: Option<String>,
    pub water_model: Option<String>,
    pub climate_model: Option<String>,
    pub scenario: Option<String>,
    pub variable: Option<String>,
    pub year: Option<i32>,
    pub enabled: bool,
    pub uploaded_at: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
    pub global_average: Option<f64>,
    pub filename: Option<String>,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub style_id: Option<Uuid>,
    pub is_crop_specific: bool,
    // Additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_status: Option<CacheStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<LayerStats>,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct CacheStatus {
    pub cached: bool,
    pub cache_key: Option<String>,
    pub size_mb: Option<f64>,
    pub ttl_hours: Option<f64>,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct LayerStats {
    pub total_requests: i32,
    pub xyz_tile_count: i32,
    pub cog_download_count: i32,
    pub pixel_query_count: i32,
    pub stac_request_count: i32,
    pub other_request_count: i32,
    pub last_accessed_at: Option<DateTime<Utc>>,
}

/// Custom get_one function that includes cache and stats metadata
pub async fn get_one_with_metadata(
    db: &sea_orm::DatabaseConnection,
    id: Uuid,
) -> Result<LayerWithMetadata, sea_orm::DbErr> {
    use sea_orm::{EntityTrait, ModelTrait};

    // Fetch the layer
    let layer = Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(sea_orm::DbErr::RecordNotFound(
            "Layer not found".to_string(),
        ))?;

    // Fetch cache status from Redis
    let cache_status = if let Some(ref layer_name) = layer.layer_name {
        fetch_cache_status(layer_name).await.ok()
    } else {
        None
    };

    // Fetch stats from database
    let stats = fetch_layer_stats(db, id).await.ok().flatten();

    // Build response
    Ok(LayerWithMetadata {
        id: layer.id,
        layer_name: layer.layer_name,
        crop: layer.crop,
        water_model: layer.water_model,
        climate_model: layer.climate_model,
        scenario: layer.scenario,
        variable: layer.variable,
        year: layer.year,
        enabled: layer.enabled,
        uploaded_at: layer.uploaded_at,
        last_updated: layer.last_updated,
        global_average: layer.global_average,
        filename: layer.filename,
        min_value: layer.min_value,
        max_value: layer.max_value,
        style_id: layer.style_id,
        is_crop_specific: layer.is_crop_specific,
        cache_status,
        stats,
    })
}

/// Helper function to fetch cache status from Redis
async fn fetch_cache_status(layer_name: &str) -> anyhow::Result<CacheStatus> {
    use crate::routes::tiles::cache;

    let redis_client = cache::get_redis_client();
    let mut con = redis_client.get_multiplexed_async_connection().await?;

    // Try to find the cache key - check with and without .tif extension
    let cache_key = cache::build_cache_key(layer_name);
    let cache_key_tif = cache::build_cache_key(&format!("{}.tif", layer_name));

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

    if ttl_seconds >= 0 {
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
            ttl_hours: Some(ttl_seconds as f64 / 3600.0),
        })
    } else {
        // Cache doesn't exist
        Ok(CacheStatus {
            cached: false,
            cache_key: None,
            size_mb: None,
            ttl_hours: None,
        })
    }
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
