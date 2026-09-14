pub mod common;
pub mod config;
pub mod routes;

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use sea_orm_migration::prelude::*;
use lazy_limit::{init_rate_limiter, Duration, RuleConfig};
use tracing::{info, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Serialises the migrators of instances starting together. Advisory locks are held by
/// a session, so the pool that takes it has one connection and the migrator runs on it.
const MIGRATION_LOCK_ID: i64 = 0x6472_6f70_3463;

async fn run_migrations(db_uri: &str) -> Result<(), sea_orm::DbErr> {
    let mut options = ConnectOptions::new(db_uri);
    options.max_connections(1).min_connections(1);
    let db = Database::connect(options).await?;
    db.execute_unprepared(&format!("SELECT pg_advisory_lock({MIGRATION_LOCK_ID})"))
        .await?;
    let result = migration::Migrator::up(&db, None).await;
    let _ = db
        .execute_unprepared(&format!("SELECT pg_advisory_unlock({MIGRATION_LOCK_ID})"))
        .await;
    result
}

#[tokio::main]
async fn main() {
    // Initialize tracing subscriber
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load config to validate runtime environment used later in app
    let config = config::Config::from_env();

    // Initialize rate limiter with values from config
    init_rate_limiter!(
        default: RuleConfig::new(Duration::seconds(1), config.rate_limit_per_ip),
        routes: []
    )
    .await;
    // let app = Router::new().route("/tiles/{z}/{x}/{y}", get(views::tile_handler));
    let db: DatabaseConnection = Database::connect(config.db_uri.as_ref().unwrap())
        .await
        .unwrap();

    if db.ping().await.is_ok() {
        info!("Connected to the database");

        info!("Running database migrations...");
        match run_migrations(config.db_uri.as_ref().unwrap()).await {
            Ok(_) => info!("Migrations completed successfully"),
            Err(e) => {
                error!("Migration failed: {:?}", e);
                panic!("Failed to run database migrations");
            }
        }
    } else {
        error!("Could not connect to the database");
        panic!("Failed to connect to database");
    }

    // Spawn background task for syncing statistics from Redis to PostgreSQL
    info!("Starting statistics sync background task (every 30 seconds)...");
    routes::stats_sync::spawn_stats_sync_task(db.clone(), config.clone());

    // Spawn background worker for distributed layer recalculation jobs
    info!("Starting distributed recalculation worker (polling every 5 seconds)...");
    tokio::spawn(routes::layers::worker::start_worker(config.clone(), db.clone()));

    // Warm important tiles in the background (globe, project cards, showcase items)
    {
        let warm_config = config.clone();
        let warm_db = db.clone();
        tokio::spawn(async move {
            routes::tiles::warming::warm_all_important_tiles(&warm_config, &warm_db).await;
        });
    }

    // Periodic watchdog: checks every 5 minutes that important tiles are still cached
    info!("Starting cache warming watchdog (every 300 seconds)...");
    tokio::spawn(routes::tiles::warming::spawn_warming_watchdog(
        config.clone(),
        db.clone(),
        300,
    ));

    let addr: std::net::SocketAddr = "0.0.0.0:3000".parse().unwrap();
    info!("Server listening on {}", addr);

    let router = routes::build_router(&db, &config);
    axum::serve(
        tokio::net::TcpListener::bind(addr).await.unwrap(),
        router.into_make_service(),
    )
    .await
    .unwrap();
}
