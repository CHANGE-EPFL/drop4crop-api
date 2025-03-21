pub mod common;
pub mod config;
pub mod routes;

use sea_orm::{Database, DatabaseConnection};

#[tokio::main]
async fn main() {
    // Load config to validate runtime environment used later in app
    let config = config::Config::from_env();
    // let app = Router::new().route("/tiles/{z}/{x}/{y}", get(views::tile_handler));
    let db: DatabaseConnection = Database::connect(config.db_uri.as_ref().unwrap())
        .await
        .unwrap();

    if db.ping().await.is_ok() {
        println!("Connected to the database");
    } else {
        println!("Could not connect to the database");
    }

    let addr: std::net::SocketAddr = "0.0.0.0:3000".parse().unwrap();
    println!("Listening on {}", addr);

    let router = routes::build_router(&db);
    axum::serve(
        tokio::net::TcpListener::bind(addr).await.unwrap(),
        router.into_make_service(),
    )
    .await
    .unwrap();
}
