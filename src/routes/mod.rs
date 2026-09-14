pub mod admin;
pub mod climate_models;
pub mod crops;
pub mod layers;
pub mod projects;
pub mod scenarios;
pub mod showcase_items;
pub mod styles;
pub mod tiles;
pub mod variable_groups;
pub mod variables;
pub mod water_models;
pub mod site_settings;
pub mod stats_sync;

use crate::{common::state::AppState, config::Config};
use axum::{Router, extract::DefaultBodyLimit, extract::Request, middleware::{self, Next}, response::Response};
use axum_keycloak_auth::{Url, instance::KeycloakAuthInstance, instance::KeycloakConfig};
use sea_orm::DatabaseConnection;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable};
use axum_governor::GovernorLayer;
use real::{RealIpLayer, RealIp};
use tower::ServiceBuilder;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::net::IpAddr;
use chrono::{DateTime, Utc, Duration};
use tracing::info;

#[derive(Clone)]
struct RateLimitConfig {
    per_ip: u32,
    global: u32,
}

struct RateLimitTracker {
    global_count: u64,
    per_ip_counts: HashMap<IpAddr, IpRateInfo>,
    last_reset: DateTime<Utc>,
}

struct IpRateInfo {
    count: u64,
    last_reset: DateTime<Utc>,
}

struct TileRequestAggregator {
    ok_count: u64,
    err_count: u64,
    last_flush: DateTime<Utc>,
}

impl TileRequestAggregator {
    fn new() -> Self {
        Self {
            ok_count: 0,
            err_count: 0,
            last_flush: Utc::now(),
        }
    }

    fn record(&mut self, status: u16) {
        if status < 400 {
            self.ok_count += 1;
        } else {
            self.err_count += 1;
        }
    }

    fn flush_if_due(&mut self) -> Option<(u64, u64, f64)> {
        let now = Utc::now();
        let elapsed = now.signed_duration_since(self.last_flush);
        let total = self.ok_count + self.err_count;
        let time_due = elapsed >= Duration::seconds(30) && total > 0;
        let volume_due = total >= 10_000;
        if time_due || volume_due {
            let result = (self.ok_count, self.err_count, elapsed.num_milliseconds() as f64 / 1000.0);
            self.ok_count = 0;
            self.err_count = 0;
            self.last_flush = now;
            Some(result)
        } else {
            None
        }
    }
}

impl RateLimitTracker {
    fn new() -> Self {
        Self {
            global_count: 0,
            per_ip_counts: HashMap::new(),
            last_reset: Utc::now(),
        }
    }

    fn record_request(&mut self, ip: IpAddr) -> (u64, u64) {
        let now = Utc::now();

        // Reset global counter every second
        if now.signed_duration_since(self.last_reset) >= Duration::seconds(1) {
            self.global_count = 0;
            self.last_reset = now;
        }

        self.global_count += 1;

        // Reset or update per-IP counter
        let ip_info = self.per_ip_counts.entry(ip).or_insert(IpRateInfo {
            count: 0,
            last_reset: now,
        });

        if now.signed_duration_since(ip_info.last_reset) >= Duration::seconds(1) {
            ip_info.count = 0;
            ip_info.last_reset = now;
        }

        ip_info.count += 1;

        (self.global_count, ip_info.count)
    }

    fn cleanup_old_entries(&mut self) {
        let now = Utc::now();
        self.per_ip_counts.retain(|_, info| {
            now.signed_duration_since(info.last_reset) < Duration::seconds(5)
        });
    }
}

/// The layer a request touched and the counter it belongs to, or `None` when the
/// request counts towards no layer. `path` and `query` are the request's own.
fn classify_layer_request<'a>(uri_path: &'a str, query_string: &'a str) -> Option<(&'a str, &'a str)> {
    let layer_query = || {
        query_string
            .split('&')
            .find(|p| p.starts_with("layer="))
            .and_then(|p| p.strip_prefix("layer="))
    };

    // Every tile path carries the layer it renders in the query string.
    if is_tile_request(uri_path) {
        return layer_query().map(|layer| (layer, "xyz"));
    }

    if !uri_path.starts_with("/api/layers") && !uri_path.starts_with("/api/stac") {
        return None;
    }

    // The layer segment of a path is only a layer when it is a UUID; the named
    // endpoints under /api/layers ("recalculate-stats") share the shape.
    let uuid_segment = |path: &'a str| {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 4
            && parts[1] == "api"
            && parts[2] == "layers"
            && uuid::Uuid::parse_str(parts[3]).is_ok()
        {
            Some(parts[3])
        } else {
            None
        }
    };

    let (layer, stat_type) = if uri_path.starts_with("/api/layers/cog/") {
        let filename = uri_path.strip_prefix("/api/layers/cog/").unwrap_or("");
        (filename.strip_suffix(".tif"), "cog")
    } else if uri_path.contains("/value") {
        (uuid_segment(uri_path), "pixel")
    } else if uri_path.starts_with("/api/stac") {
        // A STAC collection is a project and a STAC item is a layer, so only the
        // item path names something a counter can belong to.
        let item = uri_path
            .strip_prefix("/api/stac/collections/")
            .and_then(|rest| rest.split_once("/items/"))
            .map(|(_, item)| item)
            .filter(|item| !item.is_empty() && !item.contains('/'));
        (item, "stac")
    } else {
        return None;
    };

    layer.map(|l| (l, stat_type))
}

/// Increments the statistics counter a request belongs to, if any.
fn track_layer_statistics(uri_path: &str, query_string: &str, config: &Config) {
    let Some((layer_id, stat_type)) = classify_layer_request(uri_path, query_string) else {
        return;
    };

    // Fire-and-forget statistics increment
    let config = config.clone();
    let layer_id = layer_id.to_string();
    let stat_type = stat_type.to_string();
    tokio::spawn(async move {
        tiles::cache::increment_stats(config, layer_id, stat_type).await;
    });
}

/// A rendered tile, whichever page asked for it: the map, the splash globe or a
/// project card. The one definition of a tile path, read by the classifier below
/// and by the request log.
fn is_tile_request(path: &str) -> bool {
    path.starts_with("/api/layers/xyz/")
        || path.starts_with("/api/site-settings/globe-tile/")
        || path.contains("/card-tile/")
}

async fn log_request_ip(
    axum::extract::State(tracker): axum::extract::State<Arc<Mutex<RateLimitTracker>>>,
    axum::extract::State(rate_limit_config): axum::extract::State<RateLimitConfig>,
    axum::extract::State(config): axum::extract::State<Config>,
    axum::extract::State(tile_agg): axum::extract::State<Arc<Mutex<TileRequestAggregator>>>,
    request: Request,
    next: Next,
) -> Response {
    let start_time = Utc::now();
    let method = request.method().clone();
    // Nested routers see their own prefix stripped, so the path a layer sees depends
    // on where it is mounted. OriginalUri is what the client asked for.
    let uri_path = request
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map_or_else(|| request.uri().path().to_string(), |uri| uri.0.path().to_string());
    let query_string = request.uri().query().unwrap_or("");

    let ip_opt = request.extensions().get::<RealIp>().map(|r| r.ip());

    let per_ip_limit = rate_limit_config.per_ip;
    let global_limit = rate_limit_config.global;

    let (global_count, ip_count) = if let Some(ip) = ip_opt {
        let mut tracker = tracker.lock().unwrap();
        tracker.cleanup_old_entries();
        tracker.record_request(ip)
    } else {
        (0, 0)
    };

    track_layer_statistics(&uri_path, query_string, &config);

    let response = next.run(request).await;
    let status = response.status().as_u16();

    if is_tile_request(&uri_path) {
        // Aggregate tile requests — log summary every 30s instead of per-request
        let mut agg = tile_agg.lock().unwrap();
        agg.record(status);
        if let Some((ok, err, secs)) = agg.flush_if_due() {
            info!(
                ok_count = ok,
                err_count = err,
                period_secs = format!("{:.0}", secs),
                "Tile requests"
            );
        }
        // Always log tile errors individually
        if status >= 400 {
            info!(status, method = %method, uri = %uri_path, "Tile request error");
        }
    } else if let Some(ip) = ip_opt {
        let global_status = if global_limit != 0 && global_count > global_limit.into() { "X" } else { " " };
        let ip_status = if per_ip_limit != 0 && ip_count > per_ip_limit.into() { "X" } else { " " };
        let global_limit_str = if global_limit == 0 { "∞   ".to_string() } else { format!("{:4}", global_limit) };
        let ip_limit_str = if per_ip_limit == 0 { "∞  ".to_string() } else { format!("{:3}", per_ip_limit) };

        info!(
            timestamp = %start_time.format("%Y-%m-%d %H:%M:%S"),
            ip = %format!("{}", ip),
            global_count = global_count,
            global_limit = %global_limit_str,
            global_status = global_status,
            ip_count = ip_count,
            ip_limit = %ip_limit_str,
            ip_status = ip_status,
            status = status,
            method = %method,
            uri = %uri_path,
            "HTTP request"
        );
    } else {
        info!(
            timestamp = %start_time.format("%Y-%m-%d %H:%M:%S"),
            ip = "unknown",
            status = status,
            method = %method,
            uri = %uri_path,
            "HTTP request"
        );
    }

    response
}

pub fn build_router(db: &DatabaseConnection, config: &Config) -> Router {
    #[derive(OpenApi)]
    #[openapi(
        modifiers(&SecurityAddon),
        security(
            ("bearerAuth" = [])
        )
    )]
    struct ApiDoc;

    struct SecurityAddon;

    impl utoipa::Modify for SecurityAddon {
        fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
            if let Some(components) = openapi.components.as_mut() {
                components.add_security_scheme(
                    "bearerAuth",
                    utoipa::openapi::security::SecurityScheme::Http(
                        utoipa::openapi::security::HttpBuilder::new()
                            .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                            .bearer_format("JWT")
                            .build(),
                    ),
                );
            }
        }
    }

    let keycloak_instance: Option<Arc<KeycloakAuthInstance>> = if config.keycloak_url.is_empty() {
        // Fail-closed: require Keycloak in production deployments
        if config.deployment == "prod" {
            panic!("SECURITY ERROR: Keycloak authentication is required in production deployments. Please configure KEYCLOAK_URL, KEYCLOAK_REALM, and KEYCLOAK_CLIENT_ID environment variables.");
        }
        // Skip Keycloak initialization for dev/test environments only
        None
    } else {
        Some(Arc::new(KeycloakAuthInstance::new(
            KeycloakConfig::builder()
                .server(Url::parse(&config.keycloak_url).unwrap())
                .realm(String::from(&config.keycloak_realm))
                .build(),
        )))
    };

    let app_state: AppState = AppState::new(db.clone(), config.clone(), keycloak_instance);

    // Create rate limit tracking state from config
    let rate_limit_config = RateLimitConfig {
        per_ip: config.rate_limit_per_ip,
        global: config.rate_limit_global,
    };
    let rate_limit_tracker = Arc::new(Mutex::new(RateLimitTracker::new()));
    let tile_aggregator = Arc::new(Mutex::new(TileRequestAggregator::new()));

    // Build rate-limited middleware stack
    // Middleware order (outer to inner):
    //   1. RealIpLayer - Extracts client IP and stores in request extensions
    //   2. log_request_ip - Logs IP, method, and URI for each request
    //   3. GovernorLayer - Applies rate limiting based on IP
    let rate_limit_stack = ServiceBuilder::new()
        .layer(RealIpLayer::default())
        .layer(middleware::from_fn_with_state((rate_limit_tracker.clone(), rate_limit_config.clone(), config.clone(), tile_aggregator.clone()),
            |axum::extract::State((tracker, rate_limit_config, config, tile_agg)): axum::extract::State<(Arc<Mutex<RateLimitTracker>>, RateLimitConfig, Config, Arc<Mutex<TileRequestAggregator>>)>,
             request: Request,
             next: Next| async move {
                log_request_ip(
                    axum::extract::State(tracker),
                    axum::extract::State(rate_limit_config),
                    axum::extract::State(config),
                    axum::extract::State(tile_agg),
                    request,
                    next
                ).await
            }
        ))
        .layer(GovernorLayer::default());

    // Build the router with routes from the plots module
    // Apply rate limiting to API routes, but NOT to health check endpoints
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest("/api/statistics", admin::views::stats_router(&app_state))
        .nest("/api/cache", admin::views::cache_router(&app_state))
        .nest("/api/layers", layers::views::router(&app_state))
        .nest("/api/layers/xyz", tiles::views::xyz_router(&app_state)) // XYZ tiles
        .nest("/api/layers/cog", layers::views::cog_router(&app_state)) // S3-compatible COG endpoint
        .nest("/api/layers/tilejson", layers::tilejson::router(&app_state))
        .nest("/api/crops", crops::views::router(&app_state))
        .nest("/api/water-models", water_models::views::router(&app_state))
        .nest("/api/climate-models", climate_models::views::router(&app_state))
        .nest("/api/scenarios", scenarios::views::router(&app_state))
        .nest("/api/variable-groups", variable_groups::views::router(&app_state))
        .nest("/api/variables", variables::views::router(&app_state))
        .nest("/api/projects", projects::views::router(&app_state))
        .nest("/api/showcase-items", showcase_items::views::router(&app_state))
        .nest("/api/site-settings", site_settings::views::router(&app_state))
        .nest("/api/styles", styles::views::router(&app_state))
        .layer(DefaultBodyLimit::max(250 * 1024 * 1024)) // 250MB to match Uppy configuration
        .layer(rate_limit_stack.clone()) // Apply rate limiting to API routes
        .split_for_parts();

    // Merge health check routes (NO rate limiting), STAC router (with rate limiting), and docs
    router
        .merge(crate::common::views::router(&app_state)) // Health check routes - no rate limiting
        .nest("/api/stac", tiles::stac_router::router(&app_state).layer(rate_limit_stack)) // STAC with rate limiting
        .merge(Scalar::with_url("/api/docs", api))
}

#[cfg(test)]
mod tests {
    use super::classify_layer_request;

    const LAYER_ID: &str = "0c8a2a5e-1e2a-4f8e-9a6d-2b7c3d4e5f60";

    #[test]
    fn test_classify_xyz_tile_reads_the_layer_query_parameter() {
        assert_eq!(
            classify_layer_request("/api/layers/xyz/3/4/2", "layer=wheat_cwatm_wf_2030"),
            Some(("wheat_cwatm_wf_2030", "xyz"))
        );
        // The parameter is found wherever it sits in the query
        assert_eq!(
            classify_layer_request("/api/layers/xyz/3/4/2", "style_id=7&layer=barley"),
            Some(("barley", "xyz"))
        );
    }

    #[test]
    fn test_classify_xyz_tile_without_a_layer_counts_nothing() {
        assert_eq!(classify_layer_request("/api/layers/xyz/3/4/2", ""), None);
    }

    #[test]
    fn test_classify_cog_download_strips_the_extension() {
        assert_eq!(
            classify_layer_request("/api/layers/cog/wheat_cwatm_wf_2030.tif", ""),
            Some(("wheat_cwatm_wf_2030", "cog"))
        );
        assert_eq!(classify_layer_request("/api/layers/cog/wheat", ""), None);
    }

    #[test]
    fn test_classify_pixel_query_needs_a_uuid() {
        assert_eq!(
            classify_layer_request(&format!("/api/layers/{LAYER_ID}/value"), "lat=1&lon=2"),
            Some((LAYER_ID, "pixel"))
        );
        assert_eq!(
            classify_layer_request("/api/layers/recalculate-stats/value", ""),
            None
        );
    }

    // A STAC collection is a project and a STAC item is a layer.
    #[test]
    fn test_classify_stac_counts_the_item_not_the_collection() {
        assert_eq!(
            classify_layer_request(
                "/api/stac/collections/crop-water-use/items/wheat_cwatm_wf_2030",
                ""
            ),
            Some(("wheat_cwatm_wf_2030", "stac"))
        );
        assert_eq!(
            classify_layer_request("/api/stac/collections/crop-water-use", ""),
            None
        );
        assert_eq!(
            classify_layer_request("/api/stac/collections/crop-water-use/items", ""),
            None
        );
        assert_eq!(classify_layer_request("/api/stac/search", "limit=1"), None);
    }

    // Q2: a tile is a tile whichever page asked for it.
    #[test]
    fn test_classify_counts_globe_and_card_tiles_as_tiles() {
        assert_eq!(
            classify_layer_request("/api/site-settings/globe-tile/3/4/3", "layer=barley_production"),
            Some(("barley_production", "xyz"))
        );
        assert_eq!(
            classify_layer_request(
                "/api/projects/crop-water-use/card-tile/4/8/5",
                "layer=wheat_cwatm_wf_2030"
            ),
            Some(("wheat_cwatm_wf_2030", "xyz"))
        );
    }

    #[test]
    fn test_classify_globe_and_card_tiles_without_a_layer_count_nothing() {
        assert_eq!(
            classify_layer_request("/api/site-settings/globe-tile/3/4/3", ""),
            None
        );
        assert_eq!(
            classify_layer_request("/api/projects/crop-water-use/card-tile/4/8/5", ""),
            None
        );
    }

    // The request log and the counters read one definition of a tile path, so they
    // cannot drift apart again.
    #[test]
    fn test_every_tile_path_classifies_as_a_tile() {
        for path in [
            "/api/layers/xyz/3/4/2",
            "/api/site-settings/globe-tile/3/4/3",
            "/api/projects/crop-water-use/card-tile/4/8/5",
        ] {
            assert!(super::is_tile_request(path), "{path} is a tile path");
            assert_eq!(
                classify_layer_request(path, "layer=barley_production"),
                Some(("barley_production", "xyz")),
                "{path} counts as a tile"
            );
        }

        for path in [
            "/api/layers/cog/barley_production.tif",
            "/api/stac/collections/crop-water-use/items/barley_production",
        ] {
            assert!(!super::is_tile_request(path), "{path} is not a tile path");
        }
    }

    #[test]
    fn test_classify_admin_layer_reads_as_no_traffic() {
        assert_eq!(
            classify_layer_request(&format!("/api/layers/{LAYER_ID}"), ""),
            None
        );
        assert_eq!(
            classify_layer_request(&format!("/api/layers/{LAYER_ID}/uploads"), ""),
            None
        );
        assert_eq!(classify_layer_request("/api/layers/recalculate-stats", ""), None);
    }

    #[test]
    fn test_classify_ignores_paths_that_render_no_tile() {
        assert_eq!(classify_layer_request("/api/projects/active", ""), None);
        assert_eq!(classify_layer_request("/api/site-settings/config", ""), None);
        assert_eq!(classify_layer_request("/healthz", ""), None);
    }
}
