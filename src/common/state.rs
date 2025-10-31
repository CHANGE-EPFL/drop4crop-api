use crate::config::Config;
use axum_keycloak_auth::instance::KeycloakAuthInstance;
use chrono::{DateTime, Utc};
use sea_orm::DatabaseConnection;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct DownloadToken {
    pub asset_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub experiment_id: Option<Uuid>, // For experiment downloads
}

#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub config: Config,
    pub keycloak_auth_instance: Option<Arc<KeycloakAuthInstance>>,
    pub download_tokens: Arc<RwLock<HashMap<String, DownloadToken>>>,
}

impl AppState {
    pub fn new(
        db: DatabaseConnection,
        config: Config,
        keycloak_auth_instance: Option<Arc<KeycloakAuthInstance>>,
    ) -> Self {
        Self {
            db,
            config,
            keycloak_auth_instance,
            download_tokens: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a download token for assets
    pub async fn create_download_token(&self, asset_ids: Vec<Uuid>) -> String {
        let token = Uuid::new_v4().to_string();
        let download_token = DownloadToken {
            asset_ids,
            created_at: Utc::now(),
            experiment_id: None,
        };

        let mut tokens = self.download_tokens.write().await;
        tokens.insert(token.clone(), download_token);

        // Clean up old tokens (older than 5 minutes)
        let cutoff = Utc::now() - chrono::Duration::minutes(5);
        tokens.retain(|_, t| t.created_at > cutoff);

        token
    }

    /// Create a download token for an experiment
    pub async fn create_experiment_download_token(&self, experiment_id: Uuid) -> String {
        let token = Uuid::new_v4().to_string();
        let download_token = DownloadToken {
            asset_ids: Vec::new(),
            created_at: Utc::now(),
            experiment_id: Some(experiment_id),
        };

        let mut tokens = self.download_tokens.write().await;
        tokens.insert(token.clone(), download_token);

        // Clean up old tokens
        let cutoff = Utc::now() - chrono::Duration::minutes(5);
        tokens.retain(|_, t| t.created_at > cutoff);

        token
    }

    /// Consume a download token (removes it and returns the data)
    pub async fn consume_download_token(&self, token: &str) -> Option<DownloadToken> {
        let mut tokens = self.download_tokens.write().await;

        // Clean up old tokens
        let cutoff = Utc::now() - chrono::Duration::minutes(5);
        tokens.retain(|_, t| t.created_at > cutoff);

        tokens.remove(token)
    }
}
