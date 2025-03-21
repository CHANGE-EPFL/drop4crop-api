use crate::config::Config;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(ToSchema, Deserialize, Serialize, Default)]
pub struct Keycloak {
    pub client_id: String,
    pub realm: String,
    pub url: String,
}

#[derive(ToSchema, Deserialize, Serialize)]
pub struct HealthCheck {
    pub status: String,
}

#[derive(ToSchema, Deserialize, Serialize)]
pub struct ServiceStatus {
    pub s3_status: bool,
    pub kubernetes_status: bool,
}

#[derive(Deserialize)]
pub struct LowResolution {
    #[serde(default)]
    pub high_resolution: bool,
}
