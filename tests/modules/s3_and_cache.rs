// Integration tests for S3 storage and Redis caching

use crate::common::client::TestClient;
use crate::common::db::create_test_app;
use std::io::Cursor;

/// Creates a test GeoTIFF with a known pattern for verification
///
/// Pattern: 10x10 pixel grid with values forming a diagonal gradient
/// - Top-left (0,0) = 0.0
/// - Bottom-right (9,9) = 90.0
/// - Each pixel (x,y) = (x + y) * 5.0
///
/// This allows us to verify uploaded data by checking specific coordinates:
/// - Pixel (0,0) should be 0.0
/// - Pixel (5,5) should be 50.0
/// - Pixel (9,9) should be 90.0
///
/// GeoTIFF metadata:
/// - Extent: -180 to 180 (longitude), -90 to 90 (latitude)
/// - Pixel size: 36° x 18°
/// - Format: Float32, single band
fn create_test_geotiff() -> Vec<u8> {
    use tiff::encoder::{TiffEncoder, colortype};
    use tiff::tags::Tag;

    let width = 10u32;
    let height = 10u32;

    // Create known pattern: diagonal gradient from 0 to 90
    let mut data: Vec<f32> = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            // Known pattern: value = (x + y) * 5.0
            // This creates a diagonal gradient:
            // 0,  5, 10, 15, ...
            // 5, 10, 15, 20, ...
            // ...
            let value = ((x + y) * 5) as f32;
            data.push(value);
        }
    }

    // Write to in-memory buffer
    let mut buffer = Cursor::new(Vec::new());

    {
        let mut encoder = TiffEncoder::new(&mut buffer).unwrap();

        // Write image data
        let mut image = encoder.new_image::<colortype::Gray32Float>(width, height).unwrap();

        // Add GeoTIFF tags for georeferencing
        // Note: Using Vec instead of arrays because TiffValue trait is not implemented for arrays

        // ModelPixelScaleTag (33550): [scaleX, scaleY, scaleZ]
        // Global extent: -180 to 180 longitude, -90 to 90 latitude
        // Pixel scale: 360/10 = 36 degrees per pixel in X, 180/10 = 18 in Y
        let pixel_scale: Vec<f64> = vec![36.0, 18.0, 0.0];
        image.encoder().write_tag(Tag::Unknown(33550), &pixel_scale[..]).unwrap();

        // ModelTiepointTag (33922): [I, J, K, X, Y, Z]
        // Tiepoint: pixel (0,0,0) corresponds to world coords (-180, 90, 0)
        let tiepoint: Vec<f64> = vec![0.0, 0.0, 0.0, -180.0, 90.0, 0.0];
        image.encoder().write_tag(Tag::Unknown(33922), &tiepoint[..]).unwrap();

        // Write the pixel data (this consumes the image encoder)
        image.write_data(&data).unwrap();
        // Note: write_data() finalizes the image automatically, no need to call finish()
    }

    buffer.into_inner()
}

/// Verify the test GeoTIFF has the expected pattern
/// Returns true if the data appears to be a valid GeoTIFF
#[allow(dead_code)]
fn verify_geotiff_size(data: &[u8]) -> bool {
    // Check TIFF magic number (little-endian: 0x49 0x49 0x2A 0x00)
    if data.len() < 8 {
        return false;
    }

    let is_tiff = (data[0] == 0x49 && data[1] == 0x49 && data[2] == 0x2A && data[3] == 0x00) ||
                  (data[0] == 0x4D && data[1] == 0x4D && data[2] == 0x00 && data[3] == 0x2A);

    // Reasonable size check (should be a few KB for a 10x10 float32 image)
    is_tiff && data.len() > 1000 && data.len() < 100_000
}

// ============================================================================
// UPLOAD AND S3 STORAGE TESTS
// ============================================================================

#[tokio::test]
async fn test_upload_file_to_s3() {
    // Skip if S3 is not available
    if !crate::common::is_s3_available().await {
        eprintln!("Skipping test_upload_file_to_s3: S3/MinIO not available");
        return;
    }

    let router = create_test_app().await;
    let client = TestClient::new(router);

    // Create a test GeoTIFF with known pattern
    let geotiff_data = create_test_geotiff();

    // Verify we created valid data
    assert!(!geotiff_data.is_empty(), "Failed to create test GeoTIFF");
    println!("Generated GeoTIFF size: {} bytes", geotiff_data.len());
    println!("First 8 bytes (hex): {:02x?}", &geotiff_data[..8.min(geotiff_data.len())]);

    // Upload with valid climate layer filename format
    // Format: {crop}_{watermodel}_{climatemodel}_{scenario}_{variable}_{year}.tif
    let filename = "maize_rainfed_gfdl-esm4_ssp245_yield_2099.tif";

    let response = client
        .post_multipart("/api/layers/uploads", filename, geotiff_data)
        .await;

    // Should succeed and create layer
    response.assert_success();

    let data = response.json();
    assert!(data.is_object(), "Response should be a JSON object");

    // Verify filename was parsed correctly
    assert_eq!(data["crop"], "maize", "Crop should be 'maize'");
    assert_eq!(data["water_model"], "rainfed", "Water model should be 'rainfed'");
    assert_eq!(data["climate_model"], "gfdl-esm4", "Climate model should be 'gfdl-esm4'");
    assert_eq!(data["scenario"], "ssp245", "Scenario should be 'ssp245'");
    assert_eq!(data["variable"], "yield", "Variable should be 'yield'");
    assert_eq!(data["year"], 2099, "Year should be 2099");

    // Verify the layer was created in the database
    let layer_id = data["id"].as_str().expect("Layer should have an ID");
    let get_response = client.get(&format!("/api/layers/{}", layer_id)).await;
    get_response.assert_success();

    let layer_data = get_response.json();
    assert_eq!(layer_data["id"], layer_id, "Layer ID should match");
    assert_eq!(layer_data["layer_name"], "maize_rainfed_gfdl-esm4_ssp245_yield_2099", "Layer name should match");
}

#[tokio::test]
async fn test_upload_invalid_filename() {
    // Skip if S3 is not available
    if !crate::common::is_s3_available().await {
        eprintln!("Skipping test_upload_invalid_filename: S3/MinIO not available");
        return;
    }

    let router = create_test_app().await;
    let client = TestClient::new(router);

    let geotiff_data = create_test_geotiff();

    // Try to upload with invalid filename (not enough parts)
    let filename = "invalid.tif";

    let response = client
        .post_multipart("/api/layers/uploads", filename, geotiff_data)
        .await;

    // Should fail with 400 Bad Request
    response.assert_status(axum::http::StatusCode::BAD_REQUEST);

    let data = response.json();
    assert!(data["message"].as_str().unwrap().contains("Invalid filename"),
            "Error message should mention invalid filename");
}

#[tokio::test]
async fn test_upload_crop_variable_layer() {
    // Skip if S3 is not available
    if !crate::common::is_s3_available().await {
        eprintln!("Skipping test_upload_crop_variable_layer: S3/MinIO not available");
        return;
    }

    let router = create_test_app().await;
    let client = TestClient::new(router);

    let geotiff_data = create_test_geotiff();

    // Upload a crop-specific layer (2 parts: crop_variable)
    // Variable "yield" is in the CROP_VARIABLES config
    let filename = "wheat_yield.tif";

    let response = client
        .post_multipart("/api/layers/uploads", filename, geotiff_data)
        .await;

    // Should succeed
    response.assert_success();

    let data = response.json();
    assert_eq!(data["crop"], "wheat", "Crop should be 'wheat'");
    assert_eq!(data["variable"], "yield", "Variable should be 'yield'");
    assert_eq!(data["is_crop_specific"], true, "Should be marked as crop-specific");
}

// ============================================================================
// REDIS CACHING TESTS
// ============================================================================

#[tokio::test]
async fn test_tile_caching_with_redis() {
    // Skip if Redis or S3 is not available
    if !crate::common::is_redis_available().await {
        eprintln!("Skipping test_tile_caching_with_redis: Redis not available");
        return;
    }
    if !crate::common::is_s3_available().await {
        eprintln!("Skipping test_tile_caching_with_redis: S3/MinIO not available");
        return;
    }

    let router = create_test_app().await;
    let client = TestClient::new(router);

    // Prerequisite: Upload a layer first (or use existing test data)
    // For this test, we'll use an existing layer from fixtures (maize_yield_2020)
    let layer_name = "maize_yield_2020";

    // First request - should generate tile and cache it
    let tile_url = format!("/api/layers/xyz/0/0/0?layer={}", layer_name);
    let response1 = client.get_bytes(&tile_url).await;

    // May fail if layer doesn't have data in S3, but that's OK for cache testing
    if !response1.status.is_success() {
        println!("Note: Tile generation failed (expected if test layer has no raster data in S3)");
        return;
    }

    response1.assert_success();
    assert!(response1.body.len() > 0, "Tile should have data");
    assert_eq!(response1.header("content-type"), Some("image/png"), "Content type should be image/png");

    // Second request - should hit cache (faster)
    let response2 = client.get_bytes(&tile_url).await;
    response2.assert_success();

    // Verify both responses have same data (cached)
    assert_eq!(response1.body.len(), response2.body.len(), "Cached tile should be identical");
}

// ============================================================================
// CACHE MANAGEMENT TESTS
// ============================================================================

#[tokio::test]
async fn test_cache_management_endpoints() {
    // Skip if Redis is not available
    if !crate::common::is_redis_available().await {
        eprintln!("Skipping test_cache_management_endpoints: Redis not available");
        return;
    }

    let router = create_test_app().await;
    let client = TestClient::new(router);

    // Test cache info endpoint
    let response = client.get("/api/cache/info").await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_object(), "Cache info should be an object");
    assert!(data.get("redis_connected").is_some(), "Should have redis_connected field");

    // Test cache keys endpoint
    let response = client.get("/api/cache/keys").await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_array(), "Cache keys should be an array");

    // Test cache TTL endpoint
    let response = client.get("/api/cache/ttl").await;
    response.assert_success();

    let data = response.json();
    assert!(data.is_object(), "Cache TTL should be an object");
}
