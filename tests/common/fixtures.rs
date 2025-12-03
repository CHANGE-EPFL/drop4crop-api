// Test fixtures with realistic data

use chrono::Utc;
use uuid::Uuid;

// Pre-generated UUIDs for consistent testing
pub const STYLE_1_ID: &str = "550e8400-e29b-41d4-a716-446655440001";
pub const STYLE_2_ID: &str = "550e8400-e29b-41d4-a716-446655440002";

pub const LAYER_1_ID: &str = "650e8400-e29b-41d4-a716-446655440001";
pub const LAYER_2_ID: &str = "650e8400-e29b-41d4-a716-446655440002";
pub const LAYER_3_ID: &str = "650e8400-e29b-41d4-a716-446655440003";

pub const STAT_1_ID: &str = "750e8400-e29b-41d4-a716-446655440001";
pub const STAT_2_ID: &str = "750e8400-e29b-41d4-a716-446655440002";

// Style fixtures (PostgreSQL format with UUID casting)
pub const STYLE_FIXTURES: &[&str] = &[
    // Style 1: Default style (linear interpolation)
    r#"INSERT INTO style (id, name, style, interpolation_type) VALUES (
        '550e8400-e29b-41d4-a716-446655440001'::uuid,
        'default_blue',
        '{"type": "raster", "colormap": "viridis"}'::jsonb,
        'linear'
    )"#,

    // Style 2: Heat map style (discrete interpolation)
    r#"INSERT INTO style (id, name, style, interpolation_type) VALUES (
        '550e8400-e29b-41d4-a716-446655440002'::uuid,
        'heatmap_red',
        '{"type": "raster", "colormap": "hot", "min": 0, "max": 100}'::jsonb,
        'discrete'
    )"#,
];

// Layer fixtures (PostgreSQL format with UUID casting and timestamps)
pub const LAYER_FIXTURES: &[&str] = &[
    // Layer 1: Maize yield data
    r#"INSERT INTO layer (
        id, layer_name, crop, water_model, climate_model, scenario, variable, year,
        last_updated, enabled, uploaded_at, global_average, filename,
        min_value, max_value, style_id, is_crop_specific
    ) VALUES (
        '650e8400-e29b-41d4-a716-446655440001'::uuid,
        'maize_yield_2020',
        'maize',
        'rainfed',
        'GFDL-ESM4',
        'ssp245',
        'yield',
        2020,
        '2024-01-15T10:30:00+00:00'::timestamptz,
        true,
        '2024-01-15T10:00:00+00:00'::timestamptz,
        4.5,
        'maize_yield_2020.tif',
        0.5,
        8.2,
        '550e8400-e29b-41d4-a716-446655440001'::uuid,
        true
    )"#,

    // Layer 2: Wheat temperature data
    r#"INSERT INTO layer (
        id, layer_name, crop, water_model, climate_model, scenario, variable, year,
        last_updated, enabled, uploaded_at, global_average, filename,
        min_value, max_value, style_id, is_crop_specific
    ) VALUES (
        '650e8400-e29b-41d4-a716-446655440002'::uuid,
        'wheat_temp_2025',
        'wheat',
        'irrigated',
        'UKESM1-0-LL',
        'ssp585',
        'temperature',
        2025,
        '2024-02-20T14:15:00+00:00'::timestamptz,
        true,
        '2024-02-20T14:00:00+00:00'::timestamptz,
        22.3,
        'wheat_temp_2025.tif',
        -5.0,
        45.0,
        '550e8400-e29b-41d4-a716-446655440002'::uuid,
        true
    )"#,

    // Layer 3: Rice precipitation
    r#"INSERT INTO layer (
        id, layer_name, crop, water_model, climate_model, scenario, variable, year,
        last_updated, enabled, uploaded_at, global_average, filename,
        min_value, max_value, style_id, is_crop_specific
    ) VALUES (
        '650e8400-e29b-41d4-a716-446655440003'::uuid,
        'rice_precip_2030',
        'rice',
        'rainfed',
        'MIROC6',
        'ssp126',
        'precipitation',
        2030,
        '2024-03-10T09:45:00+00:00'::timestamptz,
        false,
        '2024-03-10T09:30:00+00:00'::timestamptz,
        850.5,
        'rice_precip_2030.tif',
        0.0,
        2500.0,
        NULL,
        true
    )"#,
];

// Statistics fixtures (PostgreSQL format with UUID casting and date types)
pub const STATS_FIXTURES: &[&str] = &[
    // Stats for Layer 1
    r#"INSERT INTO layer_statistics (
        id, layer_id, stat_date, last_accessed_at,
        xyz_tile_count, cog_download_count, pixel_query_count,
        stac_request_count, other_request_count
    ) VALUES (
        '750e8400-e29b-41d4-a716-446655440001'::uuid,
        '650e8400-e29b-41d4-a716-446655440001'::uuid,
        '2024-01-20'::date,
        '2024-01-20T15:30:00+00:00'::timestamptz,
        1250,
        45,
        120,
        30,
        15
    )"#,

    // Stats for Layer 2
    r#"INSERT INTO layer_statistics (
        id, layer_id, stat_date, last_accessed_at,
        xyz_tile_count, cog_download_count, pixel_query_count,
        stac_request_count, other_request_count
    ) VALUES (
        '750e8400-e29b-41d4-a716-446655440002'::uuid,
        '650e8400-e29b-41d4-a716-446655440002'::uuid,
        '2024-02-25'::date,
        '2024-02-25T11:20:00+00:00'::timestamptz,
        890,
        23,
        67,
        18,
        8
    )"#,
];

/// Helper to generate a new UUID string for tests
pub fn new_uuid() -> String {
    Uuid::new_v4().to_string()
}

/// Helper to get current timestamp string
pub fn now_timestamp() -> String {
    Utc::now().to_rfc3339()
}
