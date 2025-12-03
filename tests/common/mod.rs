// Common test utilities and helpers

pub mod db;
pub mod fixtures;
pub mod client;
pub mod test_router;

use std::sync::Once;

static INIT: Once = Once::new();

/// Initialize test environment (logging, env vars, etc.)
pub fn init() {
    INIT.call_once(|| {
        // Initialize tracing for tests
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_max_level(tracing::Level::DEBUG)
            .try_init();

        // Set all env vars needed for Config::from_env() to work
        // (Some route handlers call Config::from_env() directly)
        // SAFETY: This is single-threaded test initialization, run once before any tests execute
        unsafe {
            // Database
            if std::env::var("DB_URL").is_err() {
                std::env::set_var("DB_URL", std::env::var("TEST_DATABASE_URL")
                    .unwrap_or_else(|_| "postgresql://postgres:psql@localhost:5444/drop4crop_test".to_string()));
            }
            // Redis/Cache
            if std::env::var("TILE_CACHE_URI").is_err() {
                std::env::set_var("TILE_CACHE_URI", "redis://localhost:6379/0");
            }
            // S3
            if std::env::var("S3_BUCKET_ID").is_err() {
                std::env::set_var("S3_BUCKET_ID", "drop4crop");
            }
            if std::env::var("S3_ACCESS_KEY").is_err() {
                std::env::set_var("S3_ACCESS_KEY", "minioadmin");
            }
            if std::env::var("S3_SECRET_KEY").is_err() {
                std::env::set_var("S3_SECRET_KEY", "minioadmin");
            }
            if std::env::var("S3_ENDPOINT").is_err() {
                std::env::set_var("S3_ENDPOINT", "http://localhost:9000");
            }
            if std::env::var("S3_REGION").is_err() {
                std::env::set_var("S3_REGION", "us-east-1");
            }
            // Keycloak
            if std::env::var("KEYCLOAK_CLIENT_ID").is_err() {
                std::env::set_var("KEYCLOAK_CLIENT_ID", "test-client");
            }
            if std::env::var("KEYCLOAK_URL").is_err() {
                std::env::set_var("KEYCLOAK_URL", "");  // Empty to skip
            }
            if std::env::var("KEYCLOAK_REALM").is_err() {
                std::env::set_var("KEYCLOAK_REALM", "test-realm");
            }
            // App config
            if std::env::var("APP_NAME").is_err() {
                std::env::set_var("APP_NAME", "drop4crop-test");
            }
            if std::env::var("DEPLOYMENT").is_err() {
                std::env::set_var("DEPLOYMENT", "test");
            }
        }
    });
}

/// Check if S3/MinIO is available by attempting a connection
/// Returns true if S3 is reachable, false otherwise
pub async fn is_s3_available() -> bool {
    // Ensure env vars are initialized
    init();

    use std::time::Duration;

    let endpoint = std::env::var("S3_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:9000".to_string());

    // Try to connect to the S3 endpoint health check
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    // MinIO has a health endpoint at /minio/health/live
    let health_url = format!("{}/minio/health/live", endpoint);
    match client.get(&health_url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// Check if Redis is available by attempting a connection
/// Returns true if Redis is reachable, false otherwise
pub async fn is_redis_available() -> bool {
    // Ensure env vars are initialized
    init();

    let redis_url = std::env::var("TILE_CACHE_URI")
        .unwrap_or_else(|_| "redis://localhost:6379/0".to_string());

    match redis::Client::open(redis_url) {
        Ok(client) => {
            match tokio::time::timeout(
                std::time::Duration::from_secs(2),
                client.get_multiplexed_async_connection()
            ).await {
                Ok(Ok(_)) => true,
                _ => false,
            }
        }
        Err(_) => false,
    }
}

/// Macro to skip test if S3 is not available
#[macro_export]
macro_rules! skip_if_no_s3 {
    () => {
        if !crate::common::is_s3_available().await {
            eprintln!("Skipping test: S3/MinIO not available");
            return;
        }
    };
}

/// Macro to skip test if Redis is not available
#[macro_export]
macro_rules! skip_if_no_redis {
    () => {
        if !crate::common::is_redis_available().await {
            eprintln!("Skipping test: Redis not available");
            return;
        }
    };
}

/// Check if Keycloak URL is configured (non-empty)
/// Returns true if Keycloak URL is set and non-empty
pub fn is_keycloak_configured() -> bool {
    // Ensure env vars are initialized
    init();

    match std::env::var("KEYCLOAK_URL") {
        Ok(url) => !url.is_empty(),
        Err(_) => false,
    }
}
