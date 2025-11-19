use dotenvy::dotenv;
use serde::Deserialize;
use std::env;
use tracing::{info, debug};

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub db_uri: Option<String>,
    pub tile_cache_uri: String,
    pub tile_cache_ttl: u64, // Cache TTL in seconds
    pub keycloak_client_id: String,
    pub keycloak_url: String,
    pub keycloak_realm: String,
    pub s3_bucket_id: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_region: String,
    pub s3_endpoint: String,
    pub s3_prefix: String,
    pub admin_role: String,
    pub app_name: String,
    pub deployment: String,
    pub overwrite_duplicate_layers: bool,
    pub crop_variables: Vec<String>,
    pub tests_running: bool, // Flag to indicate if tests are running
    // Rate limiting configuration
    pub rate_limit_per_ip: u32, // Rate limit per second per IP (0 = infinite)
    pub rate_limit_global: u32, // Global rate limit per second (all IPs combined, 0 = infinite)
}

impl Config {
    pub fn from_env() -> Self {
        dotenv().ok(); // Load from .env file if available

        let db_uri = env::var("DB_URL").ok().or_else(|| {
            Some(format!(
                "{}://{}:{}@{}:{}/{}",
                env::var("DB_PREFIX").unwrap_or_else(|_| "postgresql".to_string()),
                env::var("DB_USER").expect("DB_USER must be set"),
                env::var("DB_PASSWORD").expect("DB_PASSWORD must be set"),
                env::var("DB_HOST").expect("DB_HOST must be set"),
                env::var("DB_PORT").unwrap_or_else(|_| "5432".to_string()),
                env::var("DB_NAME").expect("DB_NAME must be set"),
            ))
        });

        let tile_cache_uri = env::var("TILE_CACHE_URI").unwrap_or_else(|_| {
            format!(
                "{}://{}:{}/{}",
                env::var("TILE_CACHE_PREFIX").unwrap_or_else(|_| "redis".to_string()),
                env::var("TILE_CACHE_URL").expect("TILE_CACHE_URL must be set"),
                env::var("TILE_CACHE_PORT").expect("TILE_CACHE_PORT must be set"),
                env::var("TILE_CACHE_DB").unwrap_or_else(|_| "0".to_string()),
            )
        });
        Config {
            db_uri,
            tile_cache_uri,
            tile_cache_ttl: env::var("TILE_CACHE_TTL")
                .unwrap_or_else(|_| "86400".to_string()) // Default: 24 hours = 86400 seconds
                .parse()
                .unwrap_or(86400),
            app_name: env::var("APP_NAME").expect("APP_NAME must be set"),
            s3_bucket_id: env::var("S3_BUCKET_ID").expect("S3_BUCKET_ID must be set"),
            s3_access_key: env::var("S3_ACCESS_KEY").expect("S3_ACCESS_KEY must be set"),
            s3_secret_key: env::var("S3_SECRET_KEY").expect("S3_SECRET_KEY must be set"),
            s3_region: env::var("S3_REGION").unwrap_or_else(|_| "eu-central-1".to_string()),
            s3_endpoint: env::var("S3_ENDPOINT")
                .unwrap_or_else(|_| "https://s3.epfl.ch".to_string()),
            s3_prefix: env::var("S3_PREFIX").unwrap_or_else(|_| "drop4crop".to_string()),
            keycloak_client_id: env::var("KEYCLOAK_CLIENT_ID").expect("KEYCLOAK_UI_ID must be set"),
            keycloak_url: env::var("KEYCLOAK_URL").expect("KEYCLOAK_URL must be set"),
            keycloak_realm: env::var("KEYCLOAK_REALM").expect("KEYCLOAK_REALM must be set"),
            deployment: env::var("DEPLOYMENT")
                .expect("DEPLOYMENT must be set, this can be local, dev, stage, or prod"),
            overwrite_duplicate_layers: env::var("OVERWRITE_DUPLICATE_LAYERS")
                .unwrap_or_else(|_| "true".to_string())
                .parse()
                .unwrap_or(true),
            admin_role: env::var("ADMIN_ROLE").unwrap_or_else(|_| "admin".to_string()),
            tests_running: false, // Always false if using Config from_env
            crop_variables: env::var("CROP_VARIABLES")
                .unwrap_or_else(|_| {
                    "mirca_area_irrigated,mirca_area_total,mirca_rainfed,yield,production"
                        .to_string()
                })
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
            // Rate limiting defaults (0 = infinite)
            rate_limit_per_ip: env::var("RATE_LIMIT_PER_IP")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(250),
            rate_limit_global: env::var("RATE_LIMIT_GLOBAL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1000),
        }
    }

    #[cfg(test)]
    pub fn for_tests() -> Self {
        // Set default test environment variables if not already set
        let db_uri = Some(format!(
            "{}://{}:{}@{}:{}/{}",
            env::var("DB_PREFIX").unwrap_or_else(|_| "postgresql".to_string()),
            env::var("DB_USER").unwrap_or_else(|_| "postgres".to_string()),
            env::var("DB_PASSWORD").unwrap_or_else(|_| "psql".to_string()),
            env::var("DB_HOST").unwrap_or_else(|_| "localhost".to_string()),
            env::var("DB_PORT").unwrap_or_else(|_| "5432".to_string()),
            env::var("DB_NAME").unwrap_or_else(|_| "drop4crop_test".to_string())
        ));

        let tile_cache_uri = env::var("TILE_CACHE_URI").unwrap_or_else(|_| {
            format!(
                "{}://{}:{}/{}",
                env::var("TILE_CACHE_PREFIX").unwrap_or_else(|_| "drop4crop_test".to_string()),
                env::var("TILE_CACHE_URL").unwrap_or_else(|_| "test".to_string()),
                env::var("TILE_CACHE_PORT").unwrap_or_else(|_| "6379".to_string()),
                env::var("TILE_CACHE_DB").unwrap_or_else(|_| "1".to_string()),
            )
        });

        Config {
            app_name: "drop4crop-api-test".to_string(),
            keycloak_client_id: "test-ui".to_string(),
            keycloak_url: "http://localhost:8080".to_string(),
            keycloak_realm: "test-realm".to_string(),
            deployment: "test".to_string(),
            admin_role: "admin".to_string(),
            s3_access_key: "test-access-key".to_string(),
            s3_secret_key: "test-secret-key".to_string(),
            s3_bucket_id: "test-bucket".to_string(),
            s3_endpoint: "http://localhost:9000".to_string(),
            tests_running: true, // Set to true for test configurations
            db_uri,
            tile_cache_uri,
            tile_cache_ttl: 86400, // 24 hours for tests too
            s3_region: "us-east-1".to_string(),
            s3_prefix: "local".to_string(),
            overwrite_duplicate_layers: true,
            crop_variables: vec![
                "mirca_area_irrigated".to_string(),
                "mirca_area_total".to_string(),
                "mirca_rainfed".to_string(),
                "yield".to_string(),
                "production".to_string(),
            ],
            rate_limit_per_ip: 100,
            rate_limit_global: 1000,
        }
    }
}

#[cfg(test)]
pub mod test_helpers {
    use super::*;
    use crate::routes::build_router;
    use axum::Router;
    use migration::{Migrator, MigratorTrait};
    use sea_orm::{ConnectionTrait, Database, DatabaseConnection};

    pub fn init_test_env() {
        // No need for Once since each test gets its own database
        Config::for_tests();
    }

    pub async fn setup_test_db() -> DatabaseConnection {
        init_test_env();

        // Use proper SQLite in-memory database connection string
        // Each connection to :memory: creates a separate database instance
        let database_url = "sqlite::memory:";

        debug!("Creating new in-memory SQLite database: {database_url}");

        let db = Database::connect(database_url)
            .await
            .expect("Failed to connect to SQLite test database");

        // Test the connection
        if let Err(e) = db.ping().await {
            panic!("SQLite database connection failed: {e:?}");
        }

        // Enable foreign key constraints for SQLite (they are disabled by default)
        db.execute_unprepared("PRAGMA foreign_keys = ON")
            .await
            .expect("Failed to enable SQLite foreign key constraints");

        // Run migrations to create all tables
        Migrator::up(&db, None)
            .await
            .expect("Failed to run database migrations");

        info!("SQLite test database ready with all tables created");
        db
    }

    pub async fn setup_test_app() -> Router {
        let mut config = Config::for_tests();
        let db = setup_test_db().await;
        // Disable Keycloak for tests by setting the URL to empty
        config.keycloak_url = String::new();
        build_router(&db, &config)
    }
}
