// Database test utilities

use sea_orm::{Database, DatabaseConnection, DbErr, ConnectionTrait};
use migration::{Migrator, MigratorTrait};
use axum::Router;
use drop4crop_api::config::Config;

/// Create a test PostgreSQL database
pub async fn create_test_db() -> Result<DatabaseConnection, DbErr> {
    // Connect to test PostgreSQL database
    let db_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:psql@localhost:5444/drop4crop_test".to_string());

    let db = Database::connect(&db_url).await?;

    // Run migrations to set up schema
    Migrator::up(&db, None).await
        .map_err(|e| DbErr::Custom(format!("Migration failed: {}", e)))?;

    Ok(db)
}

/// Clean up test database (drop all data)
pub async fn cleanup_test_db(db: &DatabaseConnection) -> Result<(), DbErr> {
    // Truncate all tables (in reverse order to handle foreign keys)
    let backend = db.get_database_backend();

    // Execute each TRUNCATE separately to avoid "multiple commands in prepared statement" error
    let cleanup_statements = vec![
        "TRUNCATE TABLE layer_statistics CASCADE",
        "TRUNCATE TABLE layer CASCADE",
        "TRUNCATE TABLE style CASCADE",
    ];

    for sql in cleanup_statements {
        db.execute(sea_orm::Statement::from_string(
            backend,
            sql.to_owned(),
        ))
        .await?;
    }

    Ok(())
}

/// Create test app with PostgreSQL database and test configuration
/// Note: Authentication is disabled for tests (keycloak_auth_instance = None)
pub async fn create_test_app() -> Router {
    crate::common::init();

    // Create test database and run migrations
    let db = create_test_db().await.unwrap();

    // Clean up any existing data
    cleanup_test_db(&db).await.unwrap();

    // Seed test data
    seed_test_data(&db).await.unwrap();

    // Create test config - read from environment variables with fallbacks
    let s3_bucket = std::env::var("S3_BUCKET_ID")
        .unwrap_or_else(|_| "drop4crop".to_string());
    let s3_access_key = std::env::var("S3_ACCESS_KEY")
        .unwrap_or_else(|_| "minioadmin".to_string());
    let s3_endpoint = std::env::var("S3_ENDPOINT")
        .unwrap_or_else(|_| "http://drop4crop-s3:9000".to_string());

    eprintln!("DEBUG: S3 Config in tests:");
    eprintln!("  Bucket: {}", s3_bucket);
    eprintln!("  Access Key: {}", s3_access_key);
    eprintln!("  Endpoint: {}", s3_endpoint);

    let config = Config {
        db_uri: Some("sqlite::memory:".to_string()),
        tile_cache_uri: std::env::var("TILE_CACHE_URI")
            .unwrap_or_else(|_| "redis://:defaultpassword@tile-cache:6379/0".to_string()),
        tile_cache_ttl: std::env::var("TILE_CACHE_TTL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(86400),
        keycloak_client_id: std::env::var("KEYCLOAK_CLIENT_ID")
            .unwrap_or_else(|_| "test-client".to_string()),
        keycloak_url: std::env::var("KEYCLOAK_URL")
            .unwrap_or_else(|_| "".to_string()), // Empty to skip Keycloak in tests
        keycloak_realm: std::env::var("KEYCLOAK_REALM")
            .unwrap_or_else(|_| "test-realm".to_string()),
        s3_bucket_id: s3_bucket,
        s3_access_key: s3_access_key.clone(),
        s3_secret_key: std::env::var("S3_SECRET_KEY")
            .unwrap_or_else(|_| "minioadmin".to_string()),
        s3_region: std::env::var("S3_REGION")
            .unwrap_or_else(|_| "us-east-1".to_string()),
        s3_endpoint: s3_endpoint,
        s3_prefix: std::env::var("S3_PREFIX")
            .unwrap_or_else(|_| "test".to_string()),
        admin_role: std::env::var("ADMIN_ROLE")
            .unwrap_or_else(|_| "admin".to_string()),
        app_name: std::env::var("APP_NAME")
            .unwrap_or_else(|_| "drop4crop-test".to_string()),
        deployment: std::env::var("DEPLOYMENT")
            .unwrap_or_else(|_| "test".to_string()),
        overwrite_duplicate_layers: std::env::var("OVERWRITE_DUPLICATE_LAYERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(true),
        crop_variables: std::env::var("CROP_VARIABLES")
            .ok()
            .map(|v| v.split(',').map(|s| s.to_string()).collect())
            .unwrap_or_else(|| vec!["yield".to_string(), "production".to_string()]),
        tests_running: std::env::var("TESTS_RUNNING")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(true),
        rate_limit_per_ip: std::env::var("RATE_LIMIT_PER_IP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        rate_limit_global: std::env::var("RATE_LIMIT_GLOBAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    };

    // Build test router (without rate limiting)
    super::test_router::build_test_router(&db, &config)
}

/// Seed the database with test fixtures
pub async fn seed_test_data(db: &DatabaseConnection) -> Result<(), DbErr> {
    use super::fixtures;

    // Insert styles
    for sql in fixtures::STYLE_FIXTURES {
        db.execute(sea_orm::Statement::from_string(
            db.get_database_backend(),
            sql.to_string(),
        ))
        .await?;
    }

    // Insert layers
    for sql in fixtures::LAYER_FIXTURES {
        db.execute(sea_orm::Statement::from_string(
            db.get_database_backend(),
            sql.to_string(),
        ))
        .await?;
    }

    // Insert statistics
    for sql in fixtures::STATS_FIXTURES {
        db.execute(sea_orm::Statement::from_string(
            db.get_database_backend(),
            sql.to_string(),
        ))
        .await?;
    }

    Ok(())
}
