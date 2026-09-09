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

/// Zoom levels each warmed target covers.
const GLOBE_ZOOMS: std::ops::RangeInclusive<u32> = 0..=3;
const SHOWCASE_ZOOMS: std::ops::RangeInclusive<u32> = 3..=4;
const CARD_GRID_RADIUS: u32 = 1;

/// Cache key for one warmed globe tile.
fn globe_tile_key(config: &Config, layer_name: &str, z: u32, x: u32, y: u32) -> String {
    cache::build_cache_key(config, &format!("png-globe/{}/{}/{}/{}", layer_name, z, x, y))
}

/// Cache key for one warmed card tile.
fn card_tile_key(
    config: &Config,
    slug: &str,
    layer_name: &str,
    z: u32,
    x: u32,
    y: u32,
) -> String {
    cache::build_cache_key(
        config,
        &format!("png-card/{}/{}/{}/{}/{}", slug, layer_name, z, x, y),
    )
}

/// Every globe tile warming writes, with its tile coordinate.
fn globe_tile_set(config: &Config, layer_name: &str) -> Vec<(u32, u32, u32, String)> {
    let mut set = Vec::new();
    for z in GLOBE_ZOOMS {
        for (x, y) in tiles_for_zoom(z) {
            set.push((z, x, y, globe_tile_key(config, layer_name, z, x, y)));
        }
    }
    set
}

/// Every card tile warming writes for a project, with its tile coordinate.
fn card_tile_set(
    config: &Config,
    slug: &str,
    layer_name: &str,
    z: u32,
    cx: u32,
    cy: u32,
) -> Vec<(u32, u32, u32, String)> {
    tiles_around(cx, cy, z, CARD_GRID_RADIUS)
        .into_iter()
        .map(|(x, y)| (z, x, y, card_tile_key(config, slug, layer_name, z, x, y)))
        .collect()
}

/// Every showcase tile warming writes for a layer, with its tile coordinate. Keyed as the
/// xyz handler reads them, which is why the layer's style does not enter the key.
fn showcase_tile_set(
    config: &Config,
    layer_name: &str,
    layer_style_id: Option<uuid::Uuid>,
) -> Vec<(u32, u32, u32, String)> {
    let mut set = Vec::new();
    for z in SHOWCASE_ZOOMS {
        for (x, y) in tiles_for_zoom(z) {
            set.push((
                z,
                x,
                y,
                cache::warmed_tile_key(config, layer_name, layer_style_id, z, x, y),
            ));
        }
    }
    set
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

    for (z, x, y, key) in globe_tile_set(config, &layer_name) {
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

    for (z, x, y, key) in card_tile_set(config, &project.slug, &layer_name, z, cx, cy) {
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
pub async fn warm_showcase_tiles(config: &Config, db: &DatabaseConnection) {
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
        for (z, x, y, key) in showcase_tile_set(config, &layer_name, style_id) {
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

        // Check the globe tiles warming wrote — if any of the sample is missing, re-warm all
        let settings = site_settings::Entity::find().one(&db).await.ok().flatten();
        if let Some(ref s) = settings {
            if let Some(layer_id) = s.globe_layer_id {
                if let Ok(Some(l)) = layer::Entity::find_by_id(layer_id).one(&db).await {
                    if let Some(ref name) = l.layer_name {
                        let keys = probe_keys(globe_tile_set(&config, name));
                        if any_key_missing(&config, &keys).await {
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
                        let keys = probe_keys(card_tile_set(&config, &p.slug, name, z, cx, cy));
                        if any_key_missing(&config, &keys).await {
                            info!(project = %p.slug, "Card tile missing from cache, re-warming");
                            warm_card_tiles_for_project(&config, &db, p).await;
                        }
                    }
                }
            }
        }

        // Showcase tiles expire like any other and nothing else re-warms them
        let showcase_items = showcase::Entity::find()
            .filter(showcase::Column::Enabled.eq(true))
            .all(&db)
            .await
            .unwrap_or_default();

        for item in &showcase_items {
            if let Ok(Some(l)) = layer::Entity::find_by_id(item.layer_id).one(&db).await {
                if let Some(ref name) = l.layer_name {
                    let keys = probe_keys(showcase_tile_set(&config, name, l.style_id));
                    if any_key_missing(&config, &keys).await {
                        info!(showcase_item = %item.title, "Showcase tile missing from cache, re-warming");
                        warm_showcase_tiles(&config, &db).await;
                        break;
                    }
                }
            }
        }
    }
}

/// The keys the watchdog checks for a warmed set: one per zoom level, plus the corners of
/// each level, so a set cannot expire around a probe that happens to survive.
fn probe_keys(warmed: Vec<(u32, u32, u32, String)>) -> Vec<String> {
    let mut probes = Vec::new();
    let zooms: std::collections::BTreeSet<u32> = warmed.iter().map(|(z, _, _, _)| *z).collect();

    for zoom in zooms {
        let level: Vec<&(u32, u32, u32, String)> =
            warmed.iter().filter(|(z, _, _, _)| *z == zoom).collect();
        if let Some(first) = level.first() {
            probes.push(first.3.clone());
        }
        if let Some(last) = level.last()
            && level.len() > 1
        {
            probes.push(last.3.clone());
        }
    }

    probes
}

/// True when any of the keys is gone from the cache.
async fn any_key_missing(config: &Config, keys: &[String]) -> bool {
    for key in keys {
        if !check_key_exists(config, key).await {
            return true;
        }
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        let mut config = Config::for_tests();
        config.app_name = "drop4crop".to_string();
        config.deployment = "prod".to_string();
        config
    }

    fn style() -> uuid::Uuid {
        uuid::Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap()
    }

    fn keys(set: Vec<(u32, u32, u32, String)>) -> Vec<String> {
        set.into_iter().map(|(_, _, _, key)| key).collect()
    }

    #[test]
    fn test_showcase_keys_are_the_keys_a_styleless_request_reads() {
        let config = config();
        let set = showcase_tile_set(&config, "wheat", Some(style()));
        for (z, x, y, key) in &set {
            assert_eq!(
                *key,
                cache::rendered_tile_key_for_request(&config, "wheat", None, *z, *x, *y)
            );
        }
        // z3 is a 8x8 world grid, z4 a 16x16 one
        assert_eq!(set.len(), 64 + 256);
    }

    #[test]
    fn test_globe_probe_covers_every_warmed_zoom() {
        let config = config();
        let set = globe_tile_set(&config, "wheat");
        let warmed = keys(set.clone());
        let probes = probe_keys(set.clone());

        for probe in &probes {
            assert!(warmed.contains(probe), "probe {probe} is not a warmed key");
        }
        for zoom in GLOBE_ZOOMS {
            assert!(
                probes.iter().any(|p| p.contains(&format!("/{zoom}/"))),
                "no probe at zoom {zoom}"
            );
        }
    }

    #[test]
    fn test_showcase_probe_is_not_empty() {
        let config = config();
        let probes = probe_keys(showcase_tile_set(&config, "wheat", None));
        assert!(!probes.is_empty());
        for zoom in SHOWCASE_ZOOMS {
            assert!(probes.iter().any(|p| p.contains(&format!("/{zoom}/"))));
        }
    }

    #[test]
    fn test_card_probe_covers_more_than_the_centre_tile() {
        let config = config();
        let set = card_tile_set(&config, "crop-water-use", "wheat", 3, 4, 3);
        assert_eq!(set.len(), 9);

        let probes = probe_keys(set.clone());
        let centre = card_tile_key(&config, "crop-water-use", "wheat", 3, 4, 3);
        assert!(probes.len() > 1, "a single probe cannot represent a 3x3 grid");
        assert!(keys(set).contains(&centre));
    }

    #[test]
    fn test_probe_keys_of_one_tile_is_that_tile() {
        let config = config();
        let one = vec![(0, 0, 0, globe_tile_key(&config, "wheat", 0, 0, 0))];
        assert_eq!(probe_keys(one.clone()), keys(one));
    }

    #[test]
    fn test_probe_keys_of_nothing_is_nothing() {
        assert!(probe_keys(Vec::new()).is_empty());
    }
}
