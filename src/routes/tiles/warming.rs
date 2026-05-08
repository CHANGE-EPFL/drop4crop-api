use crate::config::Config;
use crate::routes::layers::db as layer;
use crate::routes::projects::db as project;
use crate::routes::showcase_items::db as showcase;
use crate::routes::site_settings::db as site_settings;
use crate::routes::styles::db as style;
use crate::routes::tiles::cache;
use crate::routes::tiles::utils::XYZTile;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use tokio_retry::strategy::FixedInterval;
use tokio_retry::RetryIf;
use tracing::info;

fn center_zoom_from_extent(extent: &Option<serde_json::Value>) -> (f64, f64, u32) {
    if let Some(ext) = extent {
        if let (Some(sw), Some(ne)) = (ext.get(0), ext.get(1)) {
            let sw_lat = sw.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let sw_lng = sw.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let ne_lat = ne.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let ne_lng = ne.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let lat = (sw_lat + ne_lat) / 2.0;
            let lon = (sw_lng + ne_lng) / 2.0;
            let span = (ne_lng - sw_lng).abs().max((ne_lat - sw_lat).abs());
            let z = if span > 0.0 {
                (360.0_f64 / span).log2().floor().max(1.0) as u32
            } else {
                4
            };
            return (lat, lon, z);
        }
    }
    (0.0, 0.0, 2)
}

fn lat_lon_to_tile(lat: f64, lon: f64, zoom: u32) -> (u32, u32) {
    let n = 2_u32.pow(zoom) as f64;
    let x = ((lon + 180.0) / 360.0 * n).floor() as u32;
    let lat_rad = lat.to_radians();
    let y = ((1.0
        - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI)
        / 2.0
        * n)
        .floor() as u32;
    (x.min(n as u32 - 1), y.min(n as u32 - 1))
}

fn tiles_for_zoom(z: u32) -> Vec<(u32, u32)> {
    let n = 1u32 << z;
    let mut tiles = Vec::with_capacity((n * n) as usize);
    for y in 0..n {
        for x in 0..n {
            tiles.push((x, y));
        }
    }
    tiles
}

fn tiles_around(cx: u32, cy: u32, z: u32, radius: u32) -> Vec<(u32, u32)> {
    let n = 1u32 << z;
    let mut tiles = Vec::new();
    let r = radius as i64;
    for dy in -r..=r {
        for dx in -r..=r {
            let x = (cx as i64 + dx).rem_euclid(n as i64) as u32;
            let y = (cy as i64 + dy).clamp(0, n as i64 - 1) as u32;
            tiles.push((x, y));
        }
    }
    tiles
}

async fn render_and_cache_tile(
    config: &Config,
    project_id: Option<uuid::Uuid>,
    layer_name: &str,
    style_id: Option<uuid::Uuid>,
    db: &DatabaseConnection,
    z: u32,
    x: u32,
    y: u32,
    cache_key: &str,
) -> bool {
    let xyz = XYZTile { x, y, z };
    let retry_strategy = FixedInterval::from_millis(200).take(3);
    let img = match RetryIf::spawn(
        retry_strategy,
        || xyz.get_one(config, project_id, layer_name),
        |_: &anyhow::Error| true,
    )
    .await
    {
        Ok(img) => img,
        Err(_) => return false,
    };

    let (dbstyle, interpolation_type) = if let Some(sid) = style_id {
        match style::Entity::find_by_id(sid).one(db).await {
            Ok(Some(s)) => (s.style, Some(s.interpolation_type)),
            _ => (None, None),
        }
    } else {
        (None, None)
    };

    let png_data = match crate::routes::tiles::styling::style_layer(
        img,
        dbstyle,
        interpolation_type.as_deref(),
    ) {
        Ok(d) => d,
        Err(_) => return false,
    };

    cache::push_cache_raw(config, cache_key, &png_data)
        .await
        .is_ok()
}

/// Warm globe tiles (z=0..3, 85 tiles total).
pub async fn warm_globe_tiles(config: &Config, db: &DatabaseConnection) {
    let settings = match site_settings::Entity::find().one(db).await {
        Ok(Some(s)) => s,
        _ => return,
    };

    let layer_id = match settings.globe_layer_id {
        Some(id) => id,
        None => return,
    };

    let layer_record = match layer::Entity::find_by_id(layer_id).one(db).await {
        Ok(Some(l)) => l,
        _ => return,
    };

    let layer_name = match &layer_record.layer_name {
        Some(n) => n.clone(),
        None => return,
    };

    // Warm the COG file
    let filename = format!("{}.tif", layer_name);
    let _ = crate::routes::tiles::storage::get_object(config, layer_record.project_id, &filename).await;

    let style_id = settings.globe_style_id.or(layer_record.style_id);
    let mut warmed = 0u32;

    for z in 0..=3 {
        for (x, y) in tiles_for_zoom(z) {
            let key = cache::build_cache_key(
                config,
                &format!("png-globe/{}/{}/{}/{}", layer_name, z, x, y),
            );
            if render_and_cache_tile(
                config,
                layer_record.project_id,
                &layer_name,
                style_id,
                db,
                z,
                x,
                y,
                &key,
            )
            .await
            {
                warmed += 1;
            }
        }
    }

    info!(warmed, layer = %layer_name, "Warmed globe tiles");
}

/// Warm card tiles for a single project (3x3 grid at the project's zoom level).
pub async fn warm_card_tiles_for_project(
    config: &Config,
    db: &DatabaseConnection,
    project: &project::Model,
) {
    let card_layer_id = match project.card_layer_id {
        Some(id) => id,
        None => return,
    };

    let layer_record = match layer::Entity::find_by_id(card_layer_id).one(db).await {
        Ok(Some(l)) => l,
        _ => return,
    };

    let layer_name = match &layer_record.layer_name {
        Some(n) => n.clone(),
        None => return,
    };

    // Warm the COG file
    let filename = format!("{}.tif", layer_name);
    let _ = crate::routes::tiles::storage::get_object(config, layer_record.project_id, &filename).await;

    let style_id = project.card_style_id.or(layer_record.style_id);
    let (lat, lon, z) = center_zoom_from_extent(&project.extent);
    let (cx, cy) = lat_lon_to_tile(lat, lon, z);
    let mut warmed = 0u32;

    for (x, y) in tiles_around(cx, cy, z, 1) {
        let key = cache::build_cache_key(
            config,
            &format!("png-card/{}/{}/{}/{}/{}", project.slug, layer_name, z, x, y),
        );
        if render_and_cache_tile(
            config,
            layer_record.project_id,
            &layer_name,
            style_id,
            db,
            z,
            x,
            y,
            &key,
        )
        .await
        {
            warmed += 1;
        }
    }

    if warmed > 0 {
        info!(warmed, project = %project.slug, "Warmed card tiles");
    }
}

/// Warm showcase item tiles at z3 and z4 (the zoom levels users see on
/// initial page load depending on viewport width).
async fn warm_showcase_tiles(config: &Config, db: &DatabaseConnection) {
    let items = match showcase::Entity::find()
        .filter(showcase::Column::Enabled.eq(true))
        .all(db)
        .await
    {
        Ok(items) => items,
        Err(_) => return,
    };

    for item in &items {
        let layer_record = match layer::Entity::find_by_id(item.layer_id).one(db).await {
            Ok(Some(l)) => l,
            _ => continue,
        };

        let layer_name = match &layer_record.layer_name {
            Some(n) => n.clone(),
            None => continue,
        };

        // Warm COG
        let filename = format!("{}.tif", layer_name);
        let _ = crate::routes::tiles::storage::get_object(config, layer_record.project_id, &filename).await;

        let style_id = layer_record.style_id;

        let mut warmed = 0u32;
        for z in 3..=4 {
            for (x, y) in tiles_for_zoom(z) {
                let key = cache::build_rendered_tile_key(config, &layer_name, style_id, z, x, y);
                if render_and_cache_tile(
                    config,
                    layer_record.project_id,
                    &layer_name,
                    style_id,
                    db,
                    z,
                    x,
                    y,
                    &key,
                )
                .await
                {
                    warmed += 1;
                }
            }
        }

        if warmed > 0 {
            info!(warmed, showcase_item = %item.title, "Warmed showcase tiles");
        }
    }
}

/// Warm all important tiles. Called on startup.
pub async fn warm_all_important_tiles(config: &Config, db: &DatabaseConnection) {
    info!("Starting tile warming...");

    warm_globe_tiles(config, db).await;

    let projects = project::Entity::find()
        .filter(project::Column::Enabled.eq(true))
        .order_by_asc(project::Column::SortOrder)
        .all(db)
        .await
        .unwrap_or_default();

    for project in &projects {
        warm_card_tiles_for_project(config, db, project).await;
    }

    warm_showcase_tiles(config, db).await;

    info!("Tile warming complete");
}

/// Background loop that periodically checks if important tiles are still cached
/// and re-warms any that have gone missing. Runs every `interval_secs` seconds.
pub async fn spawn_warming_watchdog(config: Config, db: DatabaseConnection, interval_secs: u64) {
    info!(interval_secs, "Starting cache warming watchdog");
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;

        // Check if globe tiles exist — if any are missing, re-warm all
        let settings = site_settings::Entity::find().one(&db).await.ok().flatten();
        if let Some(ref s) = settings {
            if let Some(layer_id) = s.globe_layer_id {
                if let Ok(Some(l)) = layer::Entity::find_by_id(layer_id).one(&db).await {
                    if let Some(ref name) = l.layer_name {
                        let key = cache::build_cache_key(&config, &format!("png-globe/{}/0/0/0", name));
                        let exists = check_key_exists(&config, &key).await;
                        if !exists {
                            info!("Globe tile missing from cache, re-warming");
                            warm_globe_tiles(&config, &db).await;
                        }
                    }
                }
            }
        }

        // Check card tiles for each enabled project
        let projects = project::Entity::find()
            .filter(project::Column::Enabled.eq(true))
            .all(&db)
            .await
            .unwrap_or_default();

        for p in &projects {
            if let Some(card_layer_id) = p.card_layer_id {
                if let Ok(Some(l)) = layer::Entity::find_by_id(card_layer_id).one(&db).await {
                    if let Some(ref name) = l.layer_name {
                        let (lat, lon, z) = center_zoom_from_extent(&p.extent);
                        let (cx, cy) = lat_lon_to_tile(lat, lon, z);
                        let key = cache::build_cache_key(
                            &config,
                            &format!("png-card/{}/{}/{}/{}/{}", p.slug, name, z, cx, cy),
                        );
                        if !check_key_exists(&config, &key).await {
                            info!(project = %p.slug, "Card tile missing from cache, re-warming");
                            warm_card_tiles_for_project(&config, &db, p).await;
                        }
                    }
                }
            }
        }
    }
}

async fn check_key_exists(config: &Config, key: &str) -> bool {
    let client = match redis::Client::open(config.tile_cache_uri.clone()) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let mut con = match client.get_multiplexed_async_connection().await {
        Ok(c) => c,
        Err(_) => return false,
    };
    redis::cmd("EXISTS")
        .arg(key)
        .query_async::<i32>(&mut con)
        .await
        .unwrap_or(0)
        > 0
}

/// Called after a style is updated — re-warms globe and card tiles if they use this style.
pub async fn warm_after_style_change(
    config: &Config,
    db: &DatabaseConnection,
    style_id: uuid::Uuid,
) {
    let settings = site_settings::Entity::find().one(db).await.ok().flatten();
    if let Some(ref s) = settings {
        if s.globe_style_id == Some(style_id) {
            warm_globe_tiles(config, db).await;
        }
    }

    let projects = project::Entity::find()
        .filter(project::Column::CardStyleId.eq(style_id))
        .filter(project::Column::Enabled.eq(true))
        .all(db)
        .await
        .unwrap_or_default();

    for project in &projects {
        warm_card_tiles_for_project(config, db, project).await;
    }
}
