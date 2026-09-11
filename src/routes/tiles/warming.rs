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

fn center_from_extent(extent: &Option<serde_json::Value>) -> Option<(f64, f64)> {
    if let Some(ext) = extent {
        if let (Some(sw), Some(ne)) = (ext.get(0), ext.get(1)) {
            let sw_lat = sw.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let sw_lng = sw.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let ne_lat = ne.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let ne_lng = ne.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let lat = (sw_lat + ne_lat) / 2.0;
            let lon = (sw_lng + ne_lng) / 2.0;
            return Some((lat, lon));
        }
    }
    None
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
const GLOBE_FULL_ZOOMS: std::ops::RangeInclusive<u32> = 0..=3;
const SHOWCASE_ZOOMS: std::ops::RangeInclusive<u32> = 3..=4;
const CARD_ZOOMS: std::ops::RangeInclusive<u32> = 2..=4;
const CARD_GRID_RADIUS: u32 = 1;

/// Renders in flight per warmed group. Bounds the S3 and Redis connections and the
/// styling CPU the three groups take together.
const WARM_CONCURRENCY: usize = 8;

/// The splash globe's geometry, mirroring the constants in
/// `drop4crop-ui/src/pages/SplashPage.jsx`: the fraction of the viewport the globe fills,
/// the latitude it is centred on, the map's zoom cap and the raster source's tile size.
const GLOBE_FILL: f64 = 1.20;
const GLOBE_CENTER_LAT: f64 = 20.0;
const GLOBE_MAX_MAP_ZOOM: f64 = 4.0;
const GLOBE_SOURCE_TILE_SIZE: f64 = 256.0;

/// Cesium's imagery `maximumLevel` in the easter egg background
/// (`drop4crop-ui/src/pages/UniverseBackground.jsx`).
const GLOBE_EGG_MAX_ZOOM: u32 = 5;

/// The map zoom the splash sets for a viewport.
fn globe_map_zoom(width: f64, height: f64) -> f64 {
    let min_dim = width.min(height);
    let cos_lat = GLOBE_CENTER_LAT.to_radians().cos();
    ((GLOBE_FILL * min_dim * std::f64::consts::PI * cos_lat) / 512.0)
        .log2()
        .min(GLOBE_MAX_MAP_ZOOM)
}

/// The source zoom the overlay requests at a viewport. MapLibre tiles a 512-unit grid, so a
/// 256 px source is drawn from one zoom further in than the map's own, rounded.
fn globe_source_zoom(width: f64, height: f64) -> u32 {
    let overzoom = (512.0 / GLOBE_SOURCE_TILE_SIZE).log2() as u32;
    globe_map_zoom(width, height).round().max(0.0) as u32 + overzoom
}

/// The zooms warmed as a latitude band rather than in full: every zoom past the full levels,
/// up to the deepest either the splash overlay at its zoom cap or the easter egg can request.
fn globe_band_zooms() -> std::ops::RangeInclusive<u32> {
    let deepest = globe_source_zoom(f64::MAX, f64::MAX).max(GLOBE_EGG_MAX_ZOOM);
    (GLOBE_FULL_ZOOMS.end() + 1)..=deepest
}

/// The tile rows the rotating globe can reach at a zoom. The visible disc reaches
/// `asin(1 / GLOBE_FILL)` either side of the centre latitude; above its northern edge the
/// globe's coverage runs to the pole, so the band starts at row 0.
fn globe_band_rows(z: u32) -> std::ops::RangeInclusive<u32> {
    let n = 1u32 << z;
    let half_span = (1.0 / GLOBE_FILL).asin().to_degrees();
    let south = GLOBE_CENTER_LAT - half_span;
    let (_, south_row) = lat_lon_to_tile(south, 0.0, z);
    0..=(south_row + 1).min(n - 1)
}

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
    for z in GLOBE_FULL_ZOOMS {
        for (x, y) in tiles_for_zoom(z) {
            set.push((z, x, y, globe_tile_key(config, layer_name, z, x, y)));
        }
    }
    for z in globe_band_zooms() {
        for y in globe_band_rows(z) {
            for x in 0..(1u32 << z) {
                set.push((z, x, y, globe_tile_key(config, layer_name, z, x, y)));
            }
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

fn card_tile_set_for_extent(
    config: &Config,
    slug: &str,
    layer_name: &str,
    extent: &Option<serde_json::Value>,
) -> Vec<(u32, u32, u32, String)> {
    let Some((lat, lon)) = center_from_extent(extent) else {
        return tiles_for_zoom(2)
            .into_iter()
            .map(|(x, y)| (2, x, y, card_tile_key(config, slug, layer_name, 2, x, y)))
            .collect();
    };

    let mut set = Vec::new();
    for z in CARD_ZOOMS {
        let (cx, cy) = lat_lon_to_tile(lat, lon, z);
        set.extend(card_tile_set(config, slug, layer_name, z, cx, cy));
    }
    set
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

/// Run `render` over every item with at most `bound` in flight, counting the successes.
async fn render_bounded<T, F, Fut>(items: Vec<T>, bound: usize, render: F) -> u32
where
    T: Send + 'static,
    F: Fn(T) -> Fut,
    Fut: std::future::Future<Output = bool> + Send + 'static,
{
    let mut pending = items.into_iter();
    let mut running = tokio::task::JoinSet::new();
    for item in pending.by_ref().take(bound.max(1)) {
        running.spawn(render(item));
    }

    let mut warmed = 0u32;
    while let Some(finished) = running.join_next().await {
        if matches!(finished, Ok(true)) {
            warmed += 1;
        }
        if let Some(item) = pending.next() {
            running.spawn(render(item));
        }
    }
    warmed
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

/// Warm globe tiles: every tile of z0-z3, and the latitude band the rotating globe reaches
/// at z4 and z5, the zooms the splash and the Cesium easter egg request.
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
    let warmed = warm_tiles(
        config,
        db,
        layer_record.project_id,
        &layer_name,
        style_id,
        globe_tile_set(config, &layer_name),
    )
    .await;

    info!(warmed, layer = %layer_name, "Warmed globe tiles");
}

/// Warm the card tiles a project's extent can request.
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
    let warmed = warm_tiles(
        config,
        db,
        layer_record.project_id,
        &layer_name,
        style_id,
        card_tile_set_for_extent(config, &project.slug, &layer_name, &project.extent),
    )
    .await;

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
        let warmed = warm_tiles(
            config,
            db,
            layer_record.project_id,
            &layer_name,
            style_id,
            showcase_tile_set(config, &layer_name, style_id),
        )
        .await;

        if warmed > 0 {
            info!(warmed, showcase_item = %item.title, "Warmed showcase tiles");
        }
    }
}

/// Warm all important tiles. Called on startup.
pub async fn warm_all_important_tiles(config: &Config, db: &DatabaseConnection) {
    info!("Starting tile warming...");

    let globe = tokio::spawn({
        let config = config.clone();
        let db = db.clone();
        async move { warm_globe_tiles(&config, &db).await }
    });

    let cards = tokio::spawn({
        let config = config.clone();
        let db = db.clone();
        async move {
            let projects = project::Entity::find()
                .filter(project::Column::Enabled.eq(true))
                .order_by_asc(project::Column::SortOrder)
                .all(&db)
                .await
                .unwrap_or_default();

            for project in &projects {
                warm_card_tiles_for_project(&config, &db, project).await;
            }
        }
    });

    let showcase = tokio::spawn({
        let config = config.clone();
        let db = db.clone();
        async move { warm_showcase_tiles(&config, &db).await }
    });

    let _ = tokio::join!(globe, cards, showcase);

    info!("Tile warming complete");
}

/// Background loop that periodically checks if important tiles are still cached
/// and re-warms any that have gone missing. Runs every `interval_secs` seconds.
pub async fn spawn_warming_watchdog(config: Config, db: DatabaseConnection, interval_secs: u64) {
    info!(interval_secs, "Starting cache warming watchdog");
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;

        // Re-render the globe tiles that have expired, not the whole set
        let settings = site_settings::Entity::find().one(&db).await.ok().flatten();
        if let Some(ref s) = settings {
            if let Some(layer_id) = s.globe_layer_id {
                if let Ok(Some(l)) = layer::Entity::find_by_id(layer_id).one(&db).await {
                    if let Some(ref name) = l.layer_name {
                        let expired = expired_tiles(&config, &globe_tile_set(&config, name)).await;
                        if !expired.is_empty() {
                            info!(expired = expired.len(), "Globe tiles missing from cache, re-warming");
                            let style_id = s.globe_style_id.or(l.style_id);
                            warm_tiles(&config, &db, l.project_id, name, style_id, expired).await;
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
                        let set = card_tile_set_for_extent(&config, &p.slug, name, &p.extent);
                        let expired = expired_tiles(&config, &set).await;
                        if !expired.is_empty() {
                            info!(project = %p.slug, expired = expired.len(), "Card tiles missing from cache, re-warming");
                            let style_id = p.card_style_id.or(l.style_id);
                            warm_tiles(&config, &db, l.project_id, name, style_id, expired).await;
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
                    let set = showcase_tile_set(&config, name, l.style_id);
                    let expired = expired_tiles(&config, &set).await;
                    if !expired.is_empty() {
                        info!(showcase_item = %item.title, expired = expired.len(), "Showcase tiles missing from cache, re-warming");
                        warm_tiles(&config, &db, l.project_id, name, l.style_id, expired).await;
                    }
                }
            }
        }
    }
}

/// The tiles of a warmed set whose key is no longer cached.
fn missing_tiles(
    warmed: &[(u32, u32, u32, String)],
    present: &std::collections::HashSet<String>,
) -> Vec<(u32, u32, u32, String)> {
    warmed
        .iter()
        .filter(|(_, _, _, key)| !present.contains(key))
        .cloned()
        .collect()
}

const EXISTS_BATCH: usize = 500;

/// Which of the keys are still in the cache. A cache that cannot be reached reports
/// every key present: there is nothing to re-warm into.
async fn cached_keys(config: &Config, keys: &[String]) -> std::collections::HashSet<String> {
    let all = || keys.iter().cloned().collect();

    let client = match redis::Client::open(config.tile_cache_uri.clone()) {
        Ok(c) => c,
        Err(_) => return all(),
    };
    let mut con = match client.get_multiplexed_async_connection().await {
        Ok(c) => c,
        Err(_) => return all(),
    };

    let mut present = std::collections::HashSet::new();
    for batch in keys.chunks(EXISTS_BATCH) {
        let mut pipe = redis::pipe();
        for key in batch {
            pipe.cmd("EXISTS").arg(key);
        }
        match pipe.query_async::<Vec<i32>>(&mut con).await {
            Ok(exists) => {
                for (key, found) in batch.iter().zip(exists) {
                    if found > 0 {
                        present.insert(key.clone());
                    }
                }
            }
            Err(_) => return all(),
        }
    }
    present
}

/// The tiles of a warmed set that have expired and have to be rendered again.
async fn expired_tiles(
    config: &Config,
    warmed: &[(u32, u32, u32, String)],
) -> Vec<(u32, u32, u32, String)> {
    let keys: Vec<String> = warmed.iter().map(|(_, _, _, key)| key.clone()).collect();
    missing_tiles(warmed, &cached_keys(config, &keys).await)
}

/// Renders each tile of a set and caches it, returning how many succeeded.
async fn warm_tiles(
    config: &Config,
    db: &DatabaseConnection,
    project_id: Option<uuid::Uuid>,
    layer_name: &str,
    style_id: Option<uuid::Uuid>,
    tiles: Vec<(u32, u32, u32, String)>,
) -> u32 {
    render_bounded(tiles, WARM_CONCURRENCY, |(z, x, y, key)| {
        let config = config.clone();
        let db = db.clone();
        let layer_name = layer_name.to_string();
        async move {
            render_and_cache_tile(
                &config, project_id, &layer_name, style_id, &db, z, x, y, &key,
            )
            .await
        }
    })
    .await
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

    fn all_but(set: &[(u32, u32, u32, String)], gone: &str) -> std::collections::HashSet<String> {
        keys(set.to_vec())
            .into_iter()
            .filter(|key| key != gone)
            .collect()
    }

    /// Viewports measured against the live splash, with the source zoom and the tile rows
    /// the globe requested at each.
    const MEASURED_VIEWPORTS: [(f64, f64, u32, u32); 3] = [
        (1366.0, 768.0, 3, 5),
        (1920.0, 1080.0, 4, 10),
        (2560.0, 1440.0, 4, 10),
    ];

    #[test]
    fn test_globe_source_zoom_matches_the_measured_viewports() {
        for (width, height, zoom, _) in MEASURED_VIEWPORTS {
            assert_eq!(globe_source_zoom(width, height), zoom, "{width}x{height}");
        }
    }

    #[test]
    fn test_globe_warming_covers_the_zooms_the_splash_requests() {
        let config = config();
        let coordinates: std::collections::HashSet<(u32, u32, u32)> =
            globe_tile_set(&config, "wheat")
                .into_iter()
                .map(|(z, x, y, _)| (z, x, y))
                .collect();

        for (width, height, _, deepest_row) in MEASURED_VIEWPORTS {
            let z = globe_source_zoom(width, height);
            for y in 0..=deepest_row {
                for x in 0..(1u32 << z) {
                    assert!(
                        coordinates.contains(&(z, x, y)),
                        "globe warming misses the tile {z}/{x}/{y} that {width}x{height} requests"
                    );
                }
            }
        }
    }

    #[test]
    fn test_globe_band_rows_cover_the_measured_rows() {
        for (width, height, _, deepest_row) in MEASURED_VIEWPORTS {
            let rows = globe_band_rows(globe_source_zoom(width, height));
            assert_eq!(*rows.start(), 0);
            assert!(*rows.end() >= deepest_row, "{width}x{height}");
        }
    }

    #[test]
    fn test_globe_warming_reaches_the_easter_egg_zoom() {
        let config = config();
        let zooms: std::collections::BTreeSet<u32> = globe_tile_set(&config, "wheat")
            .into_iter()
            .map(|(z, _, _, _)| z)
            .collect();

        assert!(zooms.contains(&GLOBE_EGG_MAX_ZOOM));
        assert_eq!(*zooms.iter().max().unwrap(), GLOBE_EGG_MAX_ZOOM);
    }

    #[test]
    fn test_globe_tile_set_is_the_full_levels_plus_the_band() {
        let config = config();
        let set = globe_tile_set(&config, "wheat");
        let band: usize = globe_band_zooms()
            .map(|z| globe_band_rows(z).count() * (1usize << z))
            .sum();

        // z0-z3 in full is 1 + 4 + 16 + 64
        assert_eq!(set.len(), 85 + band);
        assert_eq!(band, 11 * 16 + 21 * 32);
    }

    #[test]
    fn test_only_the_expired_globe_tile_is_rewarmed() {
        let config = config();
        let set = globe_tile_set(&config, "wheat");
        let corner = globe_tile_key(&config, "wheat", 3, 0, 0);
        assert_eq!(
            missing_tiles(&set, &all_but(&set, &corner)),
            vec![(3, 0, 0, corner)]
        );
    }

    #[test]
    fn test_an_expired_interior_globe_tile_is_seen() {
        let config = config();
        let set = globe_tile_set(&config, "wheat");
        let interior = globe_tile_key(&config, "wheat", 3, 4, 4);
        assert_eq!(
            missing_tiles(&set, &all_but(&set, &interior)),
            vec![(3, 4, 4, interior)]
        );
    }

    #[test]
    fn test_a_fully_cached_set_is_rewarmed_not_at_all() {
        let config = config();
        let set = card_tile_set(&config, "crop-water-use", "wheat", 3, 4, 3);
        let present: std::collections::HashSet<String> = keys(set.clone()).into_iter().collect();
        assert!(missing_tiles(&set, &present).is_empty());
    }

    #[test]
    fn test_an_empty_cache_rewarms_the_whole_showcase_set() {
        let config = config();
        let set = showcase_tile_set(&config, "wheat", None);
        assert_eq!(
            missing_tiles(&set, &std::collections::HashSet::new()),
            set
        );
    }

    #[test]
    fn test_card_tile_set_covers_leaflet_zoom_for_project_extent() {
        let config = config();
        let extent = Some(serde_json::json!([[5.0909, 42.0117], [37.1603, 112.1484]]));
        let set = card_tile_set_for_extent(&config, "project", "wheat", &extent);
        let coordinates: std::collections::HashSet<_> =
            set.into_iter().map(|(z, x, y, _)| (z, x, y)).collect();

        for x in 4..=6 {
            for y in 2..=3 {
                assert!(coordinates.contains(&(3, x, y)));
            }
        }
    }

    #[test]
    fn test_card_tile_set_without_extent_covers_the_z2_world() {
        let config = config();
        let set = card_tile_set_for_extent(&config, "project", "wheat", &None);
        let coordinates: std::collections::HashSet<_> = set
            .into_iter()
            .map(|(z, x, y, _)| (z, x, y))
            .collect();

        assert_eq!(coordinates.len(), 16);
        for x in 0..4 {
            for y in 0..4 {
                assert!(coordinates.contains(&(2, x, y)));
            }
        }
    }

    #[tokio::test]
    async fn test_render_bounded_holds_the_concurrency_bound() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let tiles: Vec<u32> = (0..40).collect();

        let warmed = render_bounded(tiles, 4, |_| {
            let in_flight = in_flight.clone();
            let peak = peak.clone();
            async move {
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                true
            }
        })
        .await;

        assert_eq!(warmed, 40);
        let peak = peak.load(Ordering::SeqCst);
        assert!(peak <= 4, "{peak} renders in flight, bound is 4");
        assert!(peak > 1, "the renders ran one at a time");
    }

    #[tokio::test]
    async fn test_render_bounded_counts_only_the_renders_that_succeeded() {
        let tiles: Vec<u32> = (0..10).collect();
        let warmed = render_bounded(tiles, 3, |tile| async move { tile % 2 == 0 }).await;
        assert_eq!(warmed, 5);
    }
}
