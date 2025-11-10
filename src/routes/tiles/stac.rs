use axum::{
    extract::{Query, State},
    http::{StatusCode, HeaderMap, header},
    Json,
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use crate::routes::layers::db as layer;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
pub struct SearchParams {
    limit: Option<usize>,
    #[serde(rename = "bbox")]
    _bbox: Option<String>,
    datetime: Option<String>,
    // STAC query parameters
    crop: Option<String>,
    water_model: Option<String>,
    climate_model: Option<String>,
    scenario: Option<String>,
    variable: Option<String>,
}

/// STAC API root endpoint (landing page)
pub async fn stac_root(headers: HeaderMap) -> Json<Value> {
    let base_url = get_base_url(&headers);

    Json(json!({
        "stac_version": "1.0.0",
        "type": "Catalog",
        "id": "drop4crop",
        "title": "Drop4Crop Agricultural Impact Data",
        "description": "XYZ tile service for global agricultural water stress and crop yield data",
        "links": [
            {
                "rel": "self",
                "type": "application/json",
                "href": format!("{}/api/stac", base_url)
            },
            {
                "rel": "root",
                "type": "application/json",
                "href": format!("{}/api/stac", base_url)
            },
            {
                "rel": "data",
                "type": "application/json",
                "href": format!("{}/api/stac/collections", base_url)
            },
            {
                "rel": "search",
                "type": "application/json",
                "href": format!("{}/api/stac/search", base_url),
                "method": "GET"
            },
            {
                "rel": "conformance",
                "type": "application/json",
                "href": format!("{}/api/stac/conformance", base_url)
            }
        ],
        "conformsTo": [
            "https://api.stacspec.org/v1.0.0/core",
            "https://api.stacspec.org/v1.0.0/collections",
            "https://api.stacspec.org/v1.0.0/item-search"
        ]
    }))
}

/// STAC conformance endpoint
pub async fn stac_conformance() -> Json<Value> {
    Json(json!({
        "conformsTo": [
            "https://api.stacspec.org/v1.0.0/core",
            "https://api.stacspec.org/v1.0.0/collections",
            "https://api.stacspec.org/v1.0.0/item-search",
            "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/core",
            "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/geojson"
        ]
    }))
}

/// STAC collections endpoint - returns a single collection for all Drop4Crop data
pub async fn stac_collections(
    headers: HeaderMap,
    State(db): State<DatabaseConnection>,
) -> Result<Json<Value>, StatusCode> {
    let base_url = get_base_url(&headers);

    // Get count of enabled layers
    use sea_orm::EntityTrait;
    let count = layer::Entity::find()
        .filter(layer::Column::Enabled.eq(true))
        .count(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "collections": [
            {
                "stac_version": "1.0.0",
                "type": "Collection",
                "id": "drop4crop-tiles",
                "title": "Drop4Crop XYZ Tiles",
                "description": "Global agricultural water stress and crop yield projections as XYZ tile layers",
                "license": "proprietary",
                "extent": {
                    "spatial": {
                        "bbox": [[-180.0, -90.0, 180.0, 90.0]]
                    },
                    "temporal": {
                        "interval": [["2010-01-01T00:00:00Z", "2100-12-31T23:59:59Z"]]
                    }
                },
                "links": [
                    {
                        "rel": "self",
                        "type": "application/json",
                        "href": format!("{}/api/stac/collections/drop4crop-tiles", base_url)
                    },
                    {
                        "rel": "root",
                        "type": "application/json",
                        "href": format!("{}/api/stac", base_url)
                    },
                    {
                        "rel": "items",
                        "type": "application/geo+json",
                        "href": format!("{}/api/stac/collections/drop4crop-tiles/items", base_url)
                    }
                ],
                "item_assets": {
                    "tiles": {
                        "type": "image/png",
                        "roles": ["visual"],
                        "title": "XYZ Tiles",
                        "description": "Tiled web map in XYZ format"
                    }
                },
                "summaries": {
                    "platform": ["drop4crop"],
                    "instruments": ["model"]
                },
                "keywords": ["agriculture", "water stress", "crop yield", "climate"],
                "providers": [
                    {
                        "name": "CHANGE Lab - EPFL",
                        "roles": ["producer", "processor", "host"]
                    }
                ],
                "item_count": count
            }
        ],
        "links": [
            {
                "rel": "self",
                "type": "application/json",
                "href": format!("{}/api/stac/collections", base_url)
            },
            {
                "rel": "root",
                "type": "application/json",
                "href": format!("{}/api/stac", base_url)
            }
        ]
    })))
}

/// STAC single collection endpoint
pub async fn stac_collection(
    headers: HeaderMap,
    State(db): State<DatabaseConnection>,
) -> Result<Json<Value>, StatusCode> {
    let response = stac_collections(headers, State(db)).await?;
    let collections = response.0["collections"].as_array().unwrap();
    Ok(Json(collections[0].clone()))
}

/// STAC items endpoint - returns all layers as STAC items
pub async fn stac_items(
    headers: HeaderMap,
    Query(params): Query<SearchParams>,
    State(db): State<DatabaseConnection>,
) -> Result<Json<Value>, StatusCode> {
    search_items(headers, params, db).await
}

/// STAC search endpoint
pub async fn stac_search(
    headers: HeaderMap,
    Query(params): Query<SearchParams>,
    State(db): State<DatabaseConnection>,
) -> Result<Json<Value>, StatusCode> {
    search_items(headers, params, db).await
}

/// Common search logic for items and search endpoints
async fn search_items(
    headers: HeaderMap,
    params: SearchParams,
    db: DatabaseConnection,
) -> Result<Json<Value>, StatusCode> {
    let base_url = get_base_url(&headers);

    // Build query with filters
    let mut query = layer::Entity::find()
        .filter(layer::Column::Enabled.eq(true));

    if let Some(crop) = &params.crop {
        query = query.filter(layer::Column::Crop.eq(crop));
    }
    if let Some(water_model) = &params.water_model {
        query = query.filter(layer::Column::WaterModel.eq(water_model));
    }
    if let Some(climate_model) = &params.climate_model {
        query = query.filter(layer::Column::ClimateModel.eq(climate_model));
    }
    if let Some(scenario) = &params.scenario {
        query = query.filter(layer::Column::Scenario.eq(scenario));
    }
    if let Some(variable) = &params.variable {
        query = query.filter(layer::Column::Variable.eq(variable));
    }
    if let Some(datetime) = &params.datetime {
        // Extract year from datetime string (e.g., "2010-01-01" -> 2010)
        if let Some(year_str) = datetime.split('-').next() {
            if let Ok(year) = year_str.parse::<i32>() {
                query = query.filter(layer::Column::Year.eq(year));
            }
        }
    }

    query = query.order_by_asc(layer::Column::LayerName);

    // Apply limit (default 10, max 10000)
    let limit = params.limit.unwrap_or(10).min(10000);

    let layers = query
        .limit(limit as u64)
        .all(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Convert layers to STAC items
    let features: Vec<Value> = layers
        .iter()
        .map(|layer_record| {
            let unknown = "unknown".to_string();
            let layer_name = layer_record.layer_name.as_ref().unwrap_or(&unknown);
            let year = layer_record.year.unwrap_or(2010);

            json!({
                "stac_version": "1.0.0",
                "stac_extensions": [],
                "type": "Feature",
                "id": layer_name,
                "collection": "drop4crop-tiles",
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[
                        [-180.0, -90.0],
                        [180.0, -90.0],
                        [180.0, 90.0],
                        [-180.0, 90.0],
                        [-180.0, -90.0]
                    ]]
                },
                "bbox": [-180.0, -90.0, 180.0, 90.0],
                "properties": {
                    "datetime": format!("{}-01-01T00:00:00Z", year),
                    "start_datetime": format!("{}-01-01T00:00:00Z", year),
                    "end_datetime": format!("{}-12-31T23:59:59Z", year),
                    "title": format!("{} - {} {} {} {} ({})",
                        layer_record.crop.as_ref().unwrap_or(&"".to_string()),
                        layer_record.water_model.as_ref().unwrap_or(&"".to_string()),
                        layer_record.climate_model.as_ref().unwrap_or(&"".to_string()),
                        layer_record.scenario.as_ref().unwrap_or(&"".to_string()),
                        year,
                        layer_record.variable.as_ref().unwrap_or(&"".to_string())
                    ),
                    "description": format!("Agricultural impact data for {} using {} water model and {} climate model under {} scenario",
                        layer_record.crop.as_ref().unwrap_or(&"unknown".to_string()),
                        layer_record.water_model.as_ref().unwrap_or(&"unknown".to_string()),
                        layer_record.climate_model.as_ref().unwrap_or(&"unknown".to_string()),
                        layer_record.scenario.as_ref().unwrap_or(&"unknown".to_string())
                    ),
                    "crop": layer_record.crop,
                    "water_model": layer_record.water_model,
                    "climate_model": layer_record.climate_model,
                    "scenario": layer_record.scenario,
                    "variable": layer_record.variable,
                    "year": year,
                    "global_average": layer_record.global_average,
                    "min_value": layer_record.min_value,
                    "max_value": layer_record.max_value
                },
                "links": [
                    {
                        "rel": "self",
                        "type": "application/geo+json",
                        "href": format!("{}/api/stac/collections/drop4crop-tiles/items/{}", base_url, layer_name)
                    },
                    {
                        "rel": "collection",
                        "type": "application/json",
                        "href": format!("{}/api/stac/collections/drop4crop-tiles", base_url)
                    },
                    {
                        "rel": "root",
                        "type": "application/json",
                        "href": format!("{}/api/stac", base_url)
                    }
                ],
                "assets": {
                    "tiles": {
                        "href": format!("{}/api/tiles/{{z}}/{{x}}/{{y}}?layer={}", base_url, layer_name),
                        "type": "image/png",
                        "roles": ["visual"],
                        "title": "XYZ Tiles",
                        "xyz:scheme": "xyz",
                        "xyz:min_zoom": 0,
                        "xyz:max_zoom": 18
                    },
                    "download": {
                        "href": format!("{}/api/layers/{}/download", base_url, layer_name),
                        "type": "image/tiff; application=geotiff",
                        "roles": ["data"],
                        "title": "Download full GeoTIFF"
                    }
                }
            })
        })
        .collect();

    Ok(Json(json!({
        "type": "FeatureCollection",
        "features": features,
        "links": [
            {
                "rel": "self",
                "type": "application/geo+json",
                "href": format!("{}/api/stac/search?limit={}", base_url, limit)
            },
            {
                "rel": "root",
                "type": "application/json",
                "href": format!("{}/api/stac", base_url)
            }
        ],
        "context": {
            "returned": features.len(),
            "limit": limit
        }
    })))
}

fn get_base_url(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:88");

    if host.contains("localhost") || host.starts_with("127.0.0.1") {
        format!("http://{}", host)
    } else {
        format!("https://{}", host)
    }
}
