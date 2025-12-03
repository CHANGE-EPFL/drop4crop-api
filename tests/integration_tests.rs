// Integration tests for drop4crop-api
// Run with: cargo test

mod common;

use axum::Router;
use common::client::TestClient;
use common::db::{create_test_db, seed_test_data};
use common::fixtures::*;
use drop4crop_api::config::Config;
use serde_json::json;
use axum::http::StatusCode;

/// Create test app state with PostgreSQL test database
/// Note: Authentication is disabled for tests (keycloak_auth_instance = None)
async fn create_test_app() -> Router {
    common::init();

    // Create test database and run migrations
    let db = create_test_db().await.unwrap();

    // Clean up any existing data
    common::db::cleanup_test_db(&db).await.unwrap();

    // Seed test data
    seed_test_data(&db).await.unwrap();

    // Create test config
    let config = Config {
        db_uri: Some("sqlite::memory:".to_string()),
        tile_cache_uri: "redis://localhost:6379/0".to_string(),
        tile_cache_ttl: 86400,
        keycloak_client_id: "test-client".to_string(),
        keycloak_url: "".to_string(), // Empty to skip Keycloak in tests
        keycloak_realm: "test-realm".to_string(),
        s3_bucket_id: "test-bucket".to_string(),
        s3_access_key: "test-key".to_string(),
        s3_secret_key: "test-secret".to_string(),
        s3_region: "us-east-1".to_string(),
        s3_endpoint: "http://localhost:9000".to_string(),
        s3_prefix: "test".to_string(),
        admin_role: "admin".to_string(),
        app_name: "drop4crop-test".to_string(),
        deployment: "test".to_string(),
        overwrite_duplicate_layers: true,
        crop_variables: vec!["yield".to_string(), "production".to_string()],
        tests_running: true,
        rate_limit_per_ip: 0,
        rate_limit_global: 0,
    };

    // Build test router (without rate limiting)
    common::test_router::build_test_router(&db, &config)
}

// ============================================================================
// STYLES CRUD TESTS
// ============================================================================

#[tokio::test]
async fn test_styles_get_list() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let response = client.get("/api/styles").await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_array(), "Response should be an array");
    assert_eq!(data.as_array().unwrap().len(), 2, "Should have 2 styles");

    // Check first style
    let style1 = &data[0];
    assert_eq!(style1["id"], STYLE_1_ID);
    assert_eq!(style1["name"], "default_blue");
}

#[tokio::test]
async fn test_styles_get_one() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let response = client.get(&format!("/api/styles/{}", STYLE_1_ID)).await;
    response.assert_success();

    let data = response.json();
    assert_eq!(data["id"], STYLE_1_ID);
    assert_eq!(data["name"], "default_blue");
    assert!(data["style"].is_object());
}

#[tokio::test]
async fn test_styles_get_one_not_found() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let fake_id = new_uuid();
    let response = client.get(&format!("/api/styles/{}", fake_id)).await;
    response.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_styles_create() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let new_style = json!({
        "name": "test_gradient",
        "style": {"type": "raster", "colormap": "rainbow"},
        "interpolation_type": "linear"
    });

    let response = client.post("/api/styles", &new_style).await;
    response.assert_success();

    let data = response.json();
    assert!(data["id"].is_string());
    assert_eq!(data["name"], "test_gradient");
    assert_eq!(data["style"]["colormap"], "rainbow");
    assert_eq!(data["interpolation_type"], "linear");
}

#[tokio::test]
async fn test_styles_update() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let update_data = json!({
        "name": "updated_name",
        "style": {"type": "raster", "colormap": "plasma"}
    });

    let response = client.put(&format!("/api/styles/{}", STYLE_2_ID), &update_data).await;
    response.assert_success();

    let data = response.json();
    assert_eq!(data["id"], STYLE_2_ID);
    assert_eq!(data["name"], "updated_name");
}

#[tokio::test]
async fn test_styles_delete() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    // Delete
    let response = client.delete(&format!("/api/styles/{}", STYLE_2_ID)).await;
    response.assert_success();

    // Verify deleted
    let response = client.get(&format!("/api/styles/{}", STYLE_2_ID)).await;
    response.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_styles_batch_delete() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let delete_data = json!([STYLE_1_ID, STYLE_2_ID]);

    let response = client.delete_with_body("/api/styles/batch", &delete_data).await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_array());
    assert_eq!(data.as_array().unwrap().len(), 2);
}

// ============================================================================
// LAYERS CRUD TESTS
// ============================================================================

#[tokio::test]
async fn test_layers_get_list() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let response = client.get("/api/layers").await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_array());
    assert_eq!(data.as_array().unwrap().len(), 3, "Should have 3 layers");
}

#[tokio::test]
async fn test_layers_get_list_with_filter() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    // Filter by crop (URL-encoded JSON)
    let response = client.get("/api/layers?filter=%7B%22crop%22%3A%22maize%22%7D").await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_array());
    let layers = data.as_array().unwrap();
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0]["crop"], "maize");
}

#[tokio::test]
async fn test_layers_get_list_with_pagination() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let response = client.get("/api/layers?page=1&per_page=2").await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_array());
    assert!(data.as_array().unwrap().len() <= 2);
}

#[tokio::test]
async fn test_layers_get_one() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let response = client.get(&format!("/api/layers/{}", LAYER_1_ID)).await;
    response.assert_success();

    let data = response.json();
    assert_eq!(data["id"], LAYER_1_ID);
    assert_eq!(data["crop"], "maize");
    assert_eq!(data["layer_name"], "maize_yield_2020");
}

#[tokio::test]
async fn test_layers_get_one_with_details() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    // Use standard get endpoint - metadata is now populated via hooks
    let response = client.get(&format!("/api/layers/{}", LAYER_1_ID)).await;
    response.assert_success();

    let data = response.json();
    assert_eq!(data["id"], LAYER_1_ID);
    assert!(data.is_object());
    // Note: cache_status and stats fields are optional metadata populated by hooks
    // They may not be present in tests without Redis/stats data
}

#[tokio::test]
async fn test_layers_create() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let new_layer = json!({
        "layer_name": "soybean_test_2024",
        "crop": "soybean",
        "water_model": "rainfed",
        "climate_model": "CESM2",
        "scenario": "ssp370",
        "variable": "yield",
        "year": 2024,
        "last_updated": "2024-06-01T10:00:00+00:00",
        "enabled": true,
        "uploaded_at": "2024-06-01T10:00:00+00:00",
        "global_average": 3.2,
        "filename": "soybean_test_2024.tif",
        "min_value": 0.0,
        "max_value": 10.0,
        "is_crop_specific": true
    });

    let response = client.post("/api/layers", &new_layer).await;
    response.assert_success();

    let data = response.json();
    assert!(data["id"].is_string());
    assert_eq!(data["crop"], "soybean");
    assert_eq!(data["year"], 2024);
}

#[tokio::test]
async fn test_layers_update() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let update_data = json!({
        "enabled": false,
        "global_average": 5.2
    });

    let response = client.put(&format!("/api/layers/{}", LAYER_1_ID), &update_data).await;
    response.assert_success();

    let data = response.json();
    assert_eq!(data["id"], LAYER_1_ID);
    assert_eq!(data["enabled"], false);
    assert_eq!(data["global_average"], 5.2);
}

#[tokio::test]
async fn test_layers_delete() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let response = client.delete(&format!("/api/layers/{}", LAYER_3_ID)).await;
    response.assert_success();

    // Verify deleted
    let response = client.get(&format!("/api/layers/{}", LAYER_3_ID)).await;
    response.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_layers_batch_delete() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let delete_data = json!([LAYER_2_ID, LAYER_3_ID]);

    let response = client.delete_with_body("/api/layers/batch", &delete_data).await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_array());
    // Note: S3 cleanup will fail in tests, but DB deletion should succeed
}

// ============================================================================
// LAYERS CUSTOM ENDPOINTS
// ============================================================================

#[tokio::test]
async fn test_layers_groups() {
    let router = create_test_app().await;
    let client = TestClient::new(router); // No auth required - public endpoint

    let response = client.get("/api/layers/groups").await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_object());

    // Check that it has the expected keys (singular form in API)
    assert!(data["crop"].is_array());
    assert!(data["water_model"].is_array());
    assert!(data["climate_model"].is_array());
    assert!(data["scenario"].is_array());
    assert!(data["variable"].is_array());
    assert!(data["year"].is_array());

    // Check values
    let crops = data["crop"].as_array().unwrap();
    assert!(crops.contains(&json!("maize")));
    assert!(crops.contains(&json!("wheat")));
    // Note: rice layer is disabled so it won't appear in results
}

// ============================================================================
// STATISTICS ENDPOINTS
// ============================================================================

#[tokio::test]
async fn test_statistics_list() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let response = client.get("/api/statistics").await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_array());
    assert_eq!(data.as_array().unwrap().len(), 2, "Should have 2 statistics");
}

#[tokio::test]
async fn test_statistics_get_one() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let response = client.get(&format!("/api/statistics/{}", STAT_1_ID)).await;
    response.assert_success();

    let data = response.json();
    assert_eq!(data["id"], STAT_1_ID);
    assert_eq!(data["layer_id"], LAYER_1_ID);
    assert_eq!(data["xyz_tile_count"], 1250);
}

#[tokio::test]
async fn test_statistics_summary() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let response = client.get("/api/statistics/summary").await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_object());
    // Should have summary fields
    assert!(data.get("total_requests_all_time").is_some());
    assert!(data.get("total_requests_today").is_some());
    assert!(data.get("total_layers").is_some());
}

// ============================================================================
// CACHE MANAGEMENT ENDPOINTS (Note: These will fail without Redis running)
// ============================================================================

#[tokio::test]
async fn test_cache_info() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let response = client.get("/api/cache/info").await;
    // Note: This may fail if Redis is not running, which is expected in CI
    // In that case, we expect a 500 error
    if response.status.is_success() {
        let data = response.json();
        assert!(data.is_object());
        // Should have cache info fields
        assert!(data.get("redis_connected").is_some());
        assert!(data.get("cache_size_mb").is_some());
    } else {
        // Redis not available - expected in test environment
        assert!(response.status.is_server_error());
    }
}

#[tokio::test]
async fn test_cache_keys() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let response = client.get("/api/cache/keys").await;
    // May fail without Redis
    if response.status.is_success() {
        let data = response.json();
        assert!(data.is_array());
    }
}

#[tokio::test]
async fn test_cache_ttl() {
    let router = create_test_app().await;
    let client = TestClient::new(router);

    let response = client.get("/api/cache/ttl").await;
    if response.status.is_success() {
        let data = response.json();
        assert!(data["ttl_seconds"].is_number());
    }
}

// ============================================================================
// PUBLIC ENDPOINTS
// ============================================================================

#[tokio::test]
async fn test_healthz() {
    let router = create_test_app().await;
    let client = TestClient::new(router); // No auth required

    let response = client.get("/healthz").await;
    response.assert_success();

    let data = response.json();
    assert_eq!(data["status"], "ok");
}

#[tokio::test]
async fn test_keycloak_config() {
    let router = create_test_app().await;
    let client = TestClient::new(router); // No auth required

    let response = client.get("/api/config/keycloak").await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_object());
    // Check that config fields are present (may be empty in tests)
    // Note: The API uses camelCase for these fields
    assert!(data.get("clientId").is_some());
    assert!(data.get("realm").is_some());
    assert!(data.get("url").is_some());
}
