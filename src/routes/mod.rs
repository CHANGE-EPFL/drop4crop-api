pub mod admin;
mod countries;
pub mod layers;
pub mod styles;
pub mod tiles;
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

/// Tracks layer access statistics based on the request path and query string.
/// Extracts layer name and determines the access type (xyz, cog, pixel, stac, other).
fn track_layer_statistics(uri_path: &str, query_string: &str) {
    // Skip non-layer requests
    if !uri_path.starts_with("/api/layers") && !uri_path.starts_with("/api/stac") {
        return;
    }

    let (layer_name, stat_type) = if uri_path.starts_with("/api/layers/xyz/") {
        // XYZ tile request: /api/layers/xyz/{z}/{x}/{y}?layer={name}
        let layer = query_string
            .split('&')
            .find(|p| p.starts_with("layer="))
            .and_then(|p| p.strip_prefix("layer="));
        (layer, "xyz")
    } else if uri_path.starts_with("/api/layers/cog/") {
        // COG download: /api/layers/cog/{filename}.tif
        let filename = uri_path.strip_prefix("/api/layers/cog/").unwrap_or("");
        let layer = filename.strip_suffix(".tif");
        (layer, "cog")
    } else if uri_path.contains("/value") {
        // Pixel value query: /api/layers/{id}/value?lat={}&lon={}
        let parts: Vec<&str> = uri_path.split('/').collect();
        let layer = if parts.len() >= 4 && parts[1] == "api" && parts[2] == "layers" {
            Some(parts[3])
        } else {
            None
        };
        (layer, "pixel")
    } else if uri_path.starts_with("/api/stac") {
        // STAC requests: /api/stac/collections/{name} or /api/stac/search
        if uri_path.contains("/collections/") {
            let layer = uri_path
                .strip_prefix("/api/stac/collections/")
                .and_then(|s| s.split('/').next());
            (layer, "stac")
        } else {
            // STAC search or catalog - skip individual tracking
            return;
        }
    } else if uri_path.starts_with("/api/layers/") && !uri_path.ends_with("/uploads") {
        // Other layer requests (e.g., GET /api/layers/{id})
        let parts: Vec<&str> = uri_path.split('/').collect();
        let layer = if parts.len() >= 4 && parts[1] == "api" && parts[2] == "layers" {
            let segment = parts[3];
            // Skip non-layer endpoints
            if matches!(segment, "groups" | "xyz" | "cog" | "uploads") {
                None
            } else {
                Some(segment)
            }
        } else {
            None
        };
        (layer, "other")
    } else {
        return;
    };

    if let Some(layer_id) = layer_name {
        // Fire-and-forget statistics increment
        let config = Config::from_env();
        let layer_id = layer_id.to_string();
        let stat_type = stat_type.to_string();
        tokio::spawn(async move {
            tiles::cache::increment_stats(config, layer_id, stat_type).await;
        });
    }
}

async fn log_request_ip(
    axum::extract::State(tracker): axum::extract::State<Arc<Mutex<RateLimitTracker>>>,
    axum::extract::State(config): axum::extract::State<RateLimitConfig>,
    request: Request,
    next: Next,
) -> Response {
    let start_time = Utc::now();
    let method = request.method().clone();
    let uri_path = request.uri().path().to_string();
    let query_string = request.uri().query().unwrap_or("");

    // Extract the real IP from the request extensions (set by RealIpLayer)
    let ip_opt = request.extensions().get::<RealIp>().map(|r| r.ip());

    // Use single rate limit for all endpoints
    let per_ip_limit = config.per_ip;
    let global_limit = config.global;

    // Record request and get counts
    let (global_count, ip_count) = if let Some(ip) = ip_opt {
        let mut tracker = tracker.lock().unwrap();
        tracker.cleanup_old_entries();
        tracker.record_request(ip)
    } else {
        (0, 0)
    };

    // Track statistics for layer access
    track_layer_statistics(&uri_path, query_string);

    // Execute the request
    let response = next.run(request).await;
    let status = response.status().as_u16();

    if let Some(ip) = ip_opt {
        // Check if over limit (0 means infinite)
        // Show "X" only if over limit, otherwise blank
        let global_status = if global_limit != 0 && global_count > global_limit.into() {
            "X"
        } else {
            " "
        };

        let ip_status = if per_ip_limit != 0 && ip_count > per_ip_limit.into() {
            "X"
        } else {
            " "
        };

        // Format limits (0 = ∞)
        let global_limit_str = if global_limit == 0 { "∞   ".to_string() } else { format!("{:4}", global_limit) };
        let ip_limit_str = if per_ip_limit == 0 { "∞  ".to_string() } else { format!("{:3}", per_ip_limit) };

        // Format: [YYYY-MM-DD HH:MM:SS | IP_ADDRESS | G:COUNT/LIMIT X | IP:COUNT/LIMIT X | CODE]
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

    // Build rate-limited middleware stack
    // Middleware order (outer to inner):
    //   1. RealIpLayer - Extracts client IP and stores in request extensions
    //   2. log_request_ip - Logs IP, method, and URI for each request
    //   3. GovernorLayer - Applies rate limiting based on IP
    let rate_limit_stack = ServiceBuilder::new()
        .layer(RealIpLayer::default())
        .layer(middleware::from_fn_with_state((rate_limit_tracker.clone(), rate_limit_config.clone()),
            |axum::extract::State((tracker, config)): axum::extract::State<(Arc<Mutex<RateLimitTracker>>, RateLimitConfig)>,
             request: Request,
             next: Next| async move {
                log_request_ip(
                    axum::extract::State(tracker),
                    axum::extract::State(config),
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
        .nest("/api/countries", countries::views::router(&app_state))
        .nest("/api/layers", layers::views::router(&app_state))
        .nest("/api/layers/xyz", tiles::views::xyz_router(db)) // XYZ tiles
        .nest("/api/layers/cog", layers::views::cog_router(db)) // S3-compatible COG endpoint
        .nest("/api/styles", styles::views::router(&app_state))
        .layer(DefaultBodyLimit::max(250 * 1024 * 1024)) // 250MB to match Uppy configuration
        .layer(rate_limit_stack.clone()) // Apply rate limiting to API routes
        .split_for_parts();

    // Merge health check routes (NO rate limiting), STAC router (with rate limiting), and docs
    router
        .merge(crate::common::views::router(db)) // Health check routes - no rate limiting
        .nest("/api/stac", tiles::stac_router::router(db).layer(rate_limit_stack)) // STAC with rate limiting
        .merge(Scalar::with_url("/api/docs", api))
}
