use dotenvy::dotenv;
use serde::Deserialize;
use std::env;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub db_uri: Option<String>,
    pub tile_cache_uri: String,
    pub keycloak_client_id: String,
    pub keycloak_url: String,
    pub keycloak_realm: String,
    pub s3_bucket_id: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_region: String,
    pub s3_endpoint: String,
    pub app_name: String,
    pub deployment: String,
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
            app_name: env::var("APP_NAME").expect("APP_NAME must be set"),
            s3_bucket_id: env::var("S3_BUCKET_ID").expect("S3_BUCKET_ID must be set"),
            s3_access_key: env::var("S3_ACCESS_KEY").expect("S3_ACCESS_KEY must be set"),
            s3_secret_key: env::var("S3_SECRET_KEY").expect("S3_SECRET_KEY must be set"),
            s3_region: env::var("S3_REGION").unwrap_or_else(|_| "eu-central-1".to_string()),
            s3_endpoint: env::var("S3_ENDPOINT")
                .unwrap_or_else(|_| "https://s3.epfl.ch".to_string()),
            keycloak_client_id: env::var("KEYCLOAK_CLIENT_ID").expect("KEYCLOAK_UI_ID must be set"),
            keycloak_url: env::var("KEYCLOAK_URL").expect("KEYCLOAK_URL must be set"),
            keycloak_realm: env::var("KEYCLOAK_REALM").expect("KEYCLOAK_REALM must be set"),
            deployment: env::var("DEPLOYMENT")
                .expect("DEPLOYMENT must be set, this can be local, dev, stage, or prod"),
        }
    }
}
