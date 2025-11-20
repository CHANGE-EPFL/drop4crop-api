// Database test utilities

use sea_orm::{Database, DatabaseConnection, DbErr, ConnectionTrait};
use migration::{Migrator, MigratorTrait};

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
