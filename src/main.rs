mod config;
// pub mod redis;
pub mod s3;
pub mod tiles;
pub mod views;

use axum::{routing::get, routing::Router};

#[tokio::main]
async fn main() {
    let app = Router::new().route("/tiles/{z}/{x}/{y}", get(views::tile_handler));

    let addr: std::net::SocketAddr = "0.0.0.0:3000".parse().unwrap();

    println!("Listening on {}", addr);

    // Run the server (correct axum usage without `hyper::Server`)
    axum::serve(tokio::net::TcpListener::bind(addr).await.unwrap(), app)
        .await
        .unwrap();
}
