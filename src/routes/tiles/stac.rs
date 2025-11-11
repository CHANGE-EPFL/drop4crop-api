use axum::{
    extract::{Query, State},
    http::{StatusCode, HeaderMap, header},
    Json,
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use crate::routes::layers::db as layer;
use serde::Deserialize;
use serde_json::{json, Value};
use stac::{Catalog, Collection, Link};
use stac_api::{Conformance, ItemCollection, Context};

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
pub async fn stac_root(headers: HeaderMap) -> Json<Catalog> {
    let base_url = get_base_url(&headers);

    let mut catalog = Catalog::new("drop4crop", "Drop4Crop: Agricultural Water Stress and Crop Yield Data");
    catalog.description = "Spatiotemporal Asset Catalog providing global agricultural water stress and crop yield projections. Data and content provided by F. Bassani, Q. Sun, and S. Bonetti from the CHANGE Lab at EPFL.".to_string();

    catalog.links.push(Link {
        href: format!("{}/api/stac", base_url),
        rel: "self".to_string(),
        r#type: Some("application/json".to_string()),
        title: None,
        method: None,
        headers: None,
        body: None,
        merge: None,
        additional_fields: Default::default(),
    });

    catalog.links.push(Link {
        href: format!("{}/api/stac", base_url),
        rel: "root".to_string(),
        r#type: Some("application/json".to_string()),
        title: None,
        method: None,
        headers: None,
        body: None,
        merge: None,
        additional_fields: Default::default(),
    });

    catalog.links.push(Link {
        href: format!("{}/api/stac/collections", base_url),
        rel: "data".to_string(),
        r#type: Some("application/json".to_string()),
        title: None,
        method: None,
        headers: None,
        body: None,
        merge: None,
        additional_fields: Default::default(),
    });

    catalog.links.push(Link {
        href: format!("{}/api/stac/search", base_url),
        rel: "search".to_string(),
        r#type: Some("application/json".to_string()),
        title: None,
        method: None,
        headers: None,
        body: None,
        merge: None,
        additional_fields: Default::default(),
    });

    catalog.links.push(Link {
        href: format!("{}/api/stac/conformance", base_url),
        rel: "conformance".to_string(),
        r#type: Some("application/json".to_string()),
        title: None,
        method: None,
        headers: None,
        body: None,
        merge: None,
        additional_fields: Default::default(),
    });

    // Add conformsTo field
    catalog.additional_fields.insert(
        "conformsTo".to_string(),
        json!([
            "https://api.stacspec.org/v1.0.0/core",
            "https://api.stacspec.org/v1.0.0/collections",
            "https://api.stacspec.org/v1.0.0/item-search"
        ])
    );

    Json(catalog)
}

/// STAC conformance endpoint
pub async fn stac_conformance() -> Json<Conformance> {
    let conformance = Conformance {
        conforms_to: vec![
            "https://api.stacspec.org/v1.0.0/core".to_string(),
            "https://api.stacspec.org/v1.0.0/collections".to_string(),
            "https://api.stacspec.org/v1.0.0/item-search".to_string(),
            "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/core".to_string(),
            "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/geojson".to_string(),
        ],
    };
    Json(conformance)
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

    // Create Collection using STAC types
    let mut collection = Collection::new("drop4crop-tiles", "Drop4Crop: Global Agricultural Impact Projections");
    collection.description = "Global agricultural water stress and crop yield projections from multiple climate and water models. Data includes historical and future scenarios (SSP2-4.5, SSP5-8.5) for major crops including wheat, maize, rice, and soy. Provided as XYZ tile layers and downloadable GeoTIFFs.".to_string();
    collection.license = "CC-BY-4.0".to_string();

    // Set extent (using additional_fields since the types are complex)
    collection.additional_fields.insert(
        "extent".to_string(),
        json!({
            "spatial": {
                "bbox": [[-180.0, -90.0, 180.0, 90.0]]
            },
            "temporal": {
                "interval": [["2010-01-01T00:00:00Z", "2100-12-31T23:59:59Z"]]
            }
        })
    );

    // Add links
    collection.links.push(Link {
        href: format!("{}/api/stac/collections/drop4crop-tiles", base_url),
        rel: "self".to_string(),
        r#type: Some("application/json".to_string()),
        title: None,
        method: None,
        headers: None,
        body: None,
        merge: None,
        additional_fields: Default::default(),
    });

    collection.links.push(Link {
        href: format!("{}/api/stac", base_url),
        rel: "root".to_string(),
        r#type: Some("application/json".to_string()),
        title: None,
        method: None,
        headers: None,
        body: None,
        merge: None,
        additional_fields: Default::default(),
    });

    collection.links.push(Link {
        href: format!("{}/api/stac/collections/drop4crop-tiles/items", base_url),
        rel: "items".to_string(),
        r#type: Some("application/geo+json".to_string()),
        title: None,
        method: None,
        headers: None,
        body: None,
        merge: None,
        additional_fields: Default::default(),
    });

    // Add tiles link for QGIS/other clients to discover XYZ tile endpoint
    collection.links.push(Link {
        href: format!("{}/api/layers/xyz/{{z}}/{{x}}/{{y}}?layer={{layer}}", base_url),
        rel: "tiles".to_string(),
        r#type: Some("application/vnd.mapbox-vector-tile".to_string()),
        title: Some("XYZ Tile Template".to_string()),
        method: None,
        headers: None,
        body: None,
        merge: None,
        additional_fields: Default::default(),
    });

    // Add item_assets template - tells QGIS how to access tiles for each item
    collection.additional_fields.insert(
        "item_assets".to_string(),
        json!({
            "tiles": {
                "type": "image/png",
                "roles": ["visual", "tiles"],
                "title": "XYZ Tiles",
                "description": "Rendered PNG tiles in XYZ (Slippy Map) format",
                "href": format!("{}/api/layers/xyz/{{z}}/{{x}}/{{y}}?layer={{item_id}}", base_url),
                "proj:epsg": 3857,
                "tile:scheme": "xyz",
                "tile:min_zoom": 0,
                "tile:max_zoom": 18
            },
            "download": {
                "type": "image/tiff; application=geotiff; profile=cloud-optimized",
                "roles": ["data"],
                "title": "Cloud Optimized GeoTIFF",
                "description": "Full resolution Cloud Optimized GeoTIFF with HTTP Range support for streaming",
                "href": format!("{}/api/layers/cog/{{item_id}}.tif", base_url)
            }
        })
    );

    collection.additional_fields.insert(
        "summaries".to_string(),
        json!({
            "platform": ["CHANGE Lab - EPFL"],
            "instruments": ["LPJmL"],
            "gsd": [0.5],  // 0.5 degree resolution
            "crop": ["wheat", "maize", "rice", "soy"],
            "scenario": ["historical", "ssp245", "ssp585"],
            "climate_model": ["gfdl-esm4", "ipsl-cm6a-lr", "mpi-esm1-2-hr", "mri-esm2-0", "ukesm1-0-ll"],
            "water_model": ["lpjml"],
            "datetime": ["2010-01-01T00:00:00Z", "2100-12-31T23:59:59Z"]
        })
    );

    collection.keywords = Some(vec![
        "agriculture".to_string(),
        "water stress".to_string(),
        "crop yield".to_string(),
        "climate change".to_string(),
        "climate projections".to_string(),
        "LPJmL".to_string(),
        "irrigation".to_string(),
        "food security".to_string(),
        "CMIP6".to_string(),
        "SSP scenarios".to_string(),
    ]);

    collection.additional_fields.insert(
        "providers".to_string(),
        json!([
            {
                "name": "CHANGE Lab - EPFL",
                "description": "Data and content provided by the CHANGE lab at EPFL",
                "roles": ["producer", "processor", "host"],
                "url": "https://www.epfl.ch/labs/change/"
            },
            {
                "name": "Francesca Bassani",
                "roles": ["producer"],
                "url": "https://people.epfl.ch/francesca.bassani"
            },
            {
                "name": "Qiming Sun",
                "roles": ["producer"],
                "url": "https://people.epfl.ch/qiming.sun"
            },
            {
                "name": "Sara Bonetti",
                "roles": ["producer"],
                "url": "https://people.epfl.ch/sara.bonetti"
            }
        ])
    );

    collection.additional_fields.insert(
        "item_count".to_string(),
        json!(count)
    );

    // Add documentation and citation links
    collection.additional_fields.insert(
        "sci:citation".to_string(),
        json!("Bassani, F., Sun, Q., Bonetti, S. (2025). Drop4Crop: Global Agricultural Water Stress and Crop Yield Projections. CHANGE Lab, EPFL.")
    );

    collection.additional_fields.insert(
        "sci:doi".to_string(),
        json!("10.5281/zenodo.XXXXXXX")  // Placeholder - update with actual DOI when available
    );

    // Return response with collections array and links
    Ok(Json(json!({
        "collections": [collection],
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

    // Build query with filters - join with style table
    let mut query = layer::Entity::find()
        .find_also_related(crate::routes::styles::db::Entity)
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
        if let Some(year_str) = datetime.split('-').next()
            && let Ok(year) = year_str.parse::<i32>() {
                query = query.filter(layer::Column::Year.eq(year));
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
        .map(|(layer_record, style_opt)| {
            let unknown = "unknown".to_string();
            let layer_name = layer_record.layer_name.as_ref().unwrap_or(&unknown);
            let year = layer_record.year.unwrap_or(2010);

            // Convert style to JSON if present
            let style_json = style_opt.as_ref().and_then(|style| {
                style.style.as_ref()
            });

            json!({
                "stac_version": "1.0.0",
                "stac_extensions": [
                    "https://stac-extensions.github.io/projection/v1.1.0/schema.json",
                    "https://stac-extensions.github.io/raster/v1.1.0/schema.json"
                ],
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
                    "max_value": layer_record.max_value,
                    "style": style_json,
                    "country_values": null  // Not yet implemented in database
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
                    },
                    {
                        "rel": "alternate",
                        "type": "application/vnd.mapbox-vector-tile",
                        "title": "XYZ Tiles (Web Mercator)",
                        "href": format!("{}/api/layers/xyz/{{z}}/{{x}}/{{y}}?layer={}", base_url, layer_name)
                    },
                    {
                        "rel": "preview",
                        "type": "image/png",
                        "title": "Visual Preview",
                        "href": format!("{}/api/tiles/{{z}}/{{x}}/{{y}}?layer={}", base_url, layer_name)
                    }
                ],
                "assets": {
                    "rendered_preview": {
                        "href": format!("{}/api/tiles/{{z}}/{{x}}/{{y}}?layer={}", base_url, layer_name),
                        "type": "image/png",
                        "roles": ["visual", "overview"],
                        "title": "XYZ Tiles (EPSG:3857)",
                        "description": "Pre-rendered PNG tiles for web mapping",
                        "proj:epsg": 3857,
                        "proj:shape": [256, 256],
                        "proj:bbox": [-20037508.34, -20037508.34, 20037508.34, 20037508.34],
                        "proj:transform": [156543.03392804097, 0.0, -20037508.34, 0.0, -156543.03392804097, 20037508.34],
                        "tile:tile_matrix_set": "WebMercatorQuad",
                        "raster:bands": [{
                            "data_type": "uint8",
                            "spatial_resolution": 156543.03392804097,
                            "nodata": 0
                        }]
                    },
                    "data": {
                        "href": format!("{}/api/layers/cog/{}.tif", base_url, layer_name),
                        "type": "image/tiff; application=geotiff; profile=cloud-optimized",
                        "roles": ["data"],
                        "title": "Cloud Optimized GeoTIFF (EPSG:4326)",
                        "description": "Full resolution data in WGS84",
                        "proj:epsg": 4326,
                        "proj:shape": [360, 720],
                        "proj:bbox": [-180.0, -90.0, 180.0, 90.0],
                        "raster:bands": [{
                            "data_type": "float32",
                            "spatial_resolution": 0.5,
                            "unit": layer_record.variable.as_ref().unwrap_or(&"unknown".to_string()),
                            "statistics": {
                                "minimum": layer_record.min_value.unwrap_or(0.0),
                                "maximum": layer_record.max_value.unwrap_or(1.0),
                                "mean": layer_record.global_average
                            }
                        }]
                    }
                }
            })
        })
        .collect();

    // Create ItemCollection using stac-api types
    // Convert JSON Values to Maps for ItemCollection
    let item_count = features.len();
    let items: Vec<serde_json::Map<String, Value>> = features.into_iter().filter_map(|f| {
        f.as_object().cloned()
    }).collect();

    let mut item_collection = ItemCollection::new(items)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    item_collection.links.push(
        Link::new(
            format!("{}/api/stac/search?limit={}", base_url, limit),
            "self",
        ).r#type(Some("application/geo+json".to_string()))
    );

    item_collection.links.push(
        Link::new(
            format!("{}/api/stac", base_url),
            "root",
        ).r#type(Some("application/json".to_string()))
    );

    item_collection.context = Some(Context {
        returned: item_count as u64,
        limit: Some(limit as u64),
        matched: None,
        additional_fields: Default::default(),
    });

    Ok(Json(serde_json::to_value(item_collection)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
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
