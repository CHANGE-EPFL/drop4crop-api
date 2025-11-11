pub mod common;
pub mod config;
pub mod routes;

use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::prelude::*;
use lazy_limit::{init_rate_limiter, Duration, RuleConfig};

#[tokio::main]
async fn main() {
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
        println!("Connected to the database");

        // Run database migrations
        println!("Running database migrations...");
        match migration::Migrator::up(&db, None).await {
            Ok(_) => println!("Migrations completed successfully"),
            Err(e) => {
                eprintln!("Migration failed: {:?}", e);
                panic!("Failed to run database migrations");
            }
        }
    } else {
        println!("Could not connect to the database");
        panic!("Failed to connect to database");
    }

    // Spawn background task for syncing statistics from Redis to PostgreSQL
    println!("Starting statistics sync background task (every 5 minutes)...");
    routes::stats_sync::spawn_stats_sync_task(db.clone());

    let addr: std::net::SocketAddr = "0.0.0.0:3000".parse().unwrap();
    println!("Listening on {}", addr);

    let router = routes::build_router(&db, &config);
    axum::serve(
        tokio::net::TcpListener::bind(addr).await.unwrap(),
        router.into_make_service(),
    )
    .await
    .unwrap();
}
