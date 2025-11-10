pub mod common;
pub mod config;
pub mod routes;

use sea_orm::{Database, DatabaseConnection};
use sea_orm_migration::prelude::*;
use lazy_limit::{init_rate_limiter, Duration, RuleConfig};

#[tokio::main]
async fn main() {
    // Initialize rate limiter with sensible defaults
    // 100 requests per second for general endpoints
    // 30 requests per second for download endpoint (more resource intensive)
    init_rate_limiter!(
        default: RuleConfig::new(Duration::seconds(1), 100),
        routes: [
            ("/api/layers/download", RuleConfig::new(Duration::seconds(1), 30)),
        ]
    )
    .await;

    // Load config to validate runtime environment used later in app
    let config = config::Config::from_env();
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
