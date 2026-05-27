use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, HeaderMap, header},
    Json,
};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use crate::common::state::AppState;
use crate::routes::layers::db as layer;
use crate::routes::crops::db as crop;
use crate::routes::water_models::db as water_model;
use crate::routes::climate_models::db as climate_model;
use crate::routes::scenarios::db as scenario;
use crate::routes::variables::db as variable;
use crate::routes::projects::db as project;
use serde::Deserialize;
use serde_json::{json, Value};
use stac::{Catalog, Link};
use stac_api::{Conformance, ItemCollection, Context};
use std::collections::HashMap;
use uuid::Uuid;

const DROP4CROP_EXT: &str = "https://drop4crop.epfl.ch/stac/drop4crop-extension/v1.0.0/schema.json";
const SCIENTIFIC_EXT: &str = "https://stac-extensions.github.io/scientific/v1.0.0/schema.json";
const PROJECTION_EXT: &str = "https://stac-extensions.github.io/projection/v1.1.0/schema.json";
const RASTER_EXT: &str = "https://stac-extensions.github.io/raster/v1.1.0/schema.json";

fn make_link(href: String, rel: &str, mime: &str) -> Link {
    Link {
        href,
        rel: rel.to_string(),
        r#type: Some(mime.to_string()),
        title: None,
        method: None,
        headers: None,
        body: None,
        merge: None,
        additional_fields: Default::default(),
    }
}

fn make_link_titled(href: String, rel: &str, mime: &str, title: &str) -> Link {
    let mut link = make_link(href, rel, mime);
    link.title = Some(title.to_string());
    link
}

#[derive(Deserialize)]
pub struct SearchParams {
    limit: Option<usize>,
    offset: Option<u64>,
    #[serde(rename = "bbox")]
    _bbox: Option<String>,
    datetime: Option<String>,
    crop: Option<String>,
    water_model: Option<String>,
    climate_model: Option<String>,
    scenario: Option<String>,
    variable: Option<String>,
    project: Option<String>,
}

fn default_providers() -> Value {
    json!([{
        "name": "CHANGE Lab - EPFL",
        "description": "Data and content provided by the CHANGE lab at EPFL",
        "roles": ["producer", "processor", "host"],
        "url": "https://www.epfl.ch/labs/change/"
    }])
}

fn extract_doi(paper_url: &str) -> Option<String> {
    if let Some(rest) = paper_url.strip_prefix("https://doi.org/") {
        Some(rest.to_string())
    } else if let Some(rest) = paper_url.strip_prefix("http://doi.org/") {
        Some(rest.to_string())
    } else if paper_url.starts_with("10.") {
        Some(paper_url.to_string())
    } else {
        None
    }
}

fn project_extent_to_bbox(extent: &Value) -> Option<[f64; 4]> {
    let arr = extent.as_array()?;
    if arr.len() != 2 { return None; }
    let sw = arr[0].as_array()?;
    let ne = arr[1].as_array()?;
    if sw.len() != 2 || ne.len() != 2 { return None; }
    let sw_lat = sw[0].as_f64()?;
    let sw_lng = sw[1].as_f64()?;
    let ne_lat = ne[0].as_f64()?;
    let ne_lng = ne[1].as_f64()?;
    Some([sw_lng, sw_lat, ne_lng, ne_lat])
}

struct CollectionData {
    crop_slugs: Vec<String>,
    water_model_slugs: Vec<String>,
    climate_model_slugs: Vec<String>,
    scenario_slugs: Vec<String>,
    variable_slugs: Vec<String>,
    min_year: Option<i32>,
    max_year: Option<i32>,
    layer_count: u64,
}

async fn gather_collection_data(
    project_id: Uuid,
    db: &sea_orm::DatabaseConnection,
) -> Result<CollectionData, StatusCode> {
    let err = |_| StatusCode::INTERNAL_SERVER_ERROR;

    let crop_junctions = crate::routes::projects::project_crop::Entity::find()
        .filter(crate::routes::projects::project_crop::Column::ProjectId.eq(project_id))
        .order_by_asc(crate::routes::projects::project_crop::Column::SortOrder)
        .all(db).await.map_err(err)?;
    let mut crop_slugs = Vec::new();
    for j in &crop_junctions {
        if let Some(c) = crop::Entity::find_by_id(j.crop_id).one(db).await.map_err(err)? {
            crop_slugs.push(c.slug);
        }
    }

    let wm_junctions = crate::routes::projects::project_water_model::Entity::find()
        .filter(crate::routes::projects::project_water_model::Column::ProjectId.eq(project_id))
        .order_by_asc(crate::routes::projects::project_water_model::Column::SortOrder)
        .all(db).await.map_err(err)?;
    let mut water_model_slugs = Vec::new();
    for j in &wm_junctions {
        if let Some(w) = water_model::Entity::find_by_id(j.water_model_id).one(db).await.map_err(err)? {
            water_model_slugs.push(w.slug);
        }
    }

    let cm_junctions = crate::routes::projects::project_climate_model::Entity::find()
        .filter(crate::routes::projects::project_climate_model::Column::ProjectId.eq(project_id))
        .order_by_asc(crate::routes::projects::project_climate_model::Column::SortOrder)
        .all(db).await.map_err(err)?;
    let mut climate_model_slugs = Vec::new();
    for j in &cm_junctions {
        if let Some(c) = climate_model::Entity::find_by_id(j.climate_model_id).one(db).await.map_err(err)? {
            climate_model_slugs.push(c.slug);
        }
    }

    let sc_junctions = crate::routes::projects::project_scenario::Entity::find()
        .filter(crate::routes::projects::project_scenario::Column::ProjectId.eq(project_id))
        .order_by_asc(crate::routes::projects::project_scenario::Column::SortOrder)
        .all(db).await.map_err(err)?;
    let mut scenario_slugs = Vec::new();
    for j in &sc_junctions {
        if let Some(s) = scenario::Entity::find_by_id(j.scenario_id).one(db).await.map_err(err)? {
            scenario_slugs.push(s.slug);
        }
    }

    let var_junctions = crate::routes::projects::project_variable::Entity::find()
        .filter(crate::routes::projects::project_variable::Column::ProjectId.eq(project_id))
        .order_by_asc(crate::routes::projects::project_variable::Column::SortOrder)
        .all(db).await.map_err(err)?;
    let mut variable_slugs = Vec::new();
    for j in &var_junctions {
        if let Some(v) = variable::Entity::find_by_id(j.variable_id).one(db).await.map_err(err)? {
            variable_slugs.push(v.slug);
        }
    }

    let layer_count = layer::Entity::find()
        .filter(layer::Column::Enabled.eq(true))
        .filter(layer::Column::ProjectId.eq(project_id))
        .count(db).await.map_err(err)?;

    let years: Vec<Option<i32>> = layer::Entity::find()
        .filter(layer::Column::Enabled.eq(true))
        .filter(layer::Column::ProjectId.eq(project_id))
        .select_only()
        .column(layer::Column::Year)
        .into_tuple()
        .all(db).await.map_err(err)?;

    let valid_years: Vec<i32> = years.into_iter().flatten().collect();
    let min_year = valid_years.iter().copied().min();
    let max_year = valid_years.iter().copied().max();

    Ok(CollectionData {
        crop_slugs,
        water_model_slugs,
        climate_model_slugs,
        scenario_slugs,
        variable_slugs,
        min_year,
        max_year,
        layer_count,
    })
}

fn build_collection(
    proj: &project::Model,
    data: &CollectionData,
    base_url: &str,
) -> Value {
    let slug = &proj.slug;
    let license = proj.license.as_deref().unwrap_or("CC-BY-4.0");
    let providers = proj.providers.as_ref().cloned().unwrap_or_else(default_providers);

    let bbox = proj.extent.as_ref()
        .and_then(|e| project_extent_to_bbox(e))
        .unwrap_or([-180.0, -90.0, 180.0, 90.0]);

    let temporal_start = data.min_year
        .map(|y| format!("{}-01-01T00:00:00Z", y))
        .unwrap_or_else(|| "null".to_string());
    let temporal_end = data.max_year
        .map(|y| format!("{}-12-31T23:59:59Z", y))
        .unwrap_or_else(|| "null".to_string());

    let citation_text = proj.citation.as_ref()
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str());
    let paper_url = proj.citation.as_ref()
        .and_then(|c| c.get("paper_url"))
        .and_then(|u| u.as_str());
    let doi = paper_url.and_then(extract_doi);

    let mut stac_extensions = vec![];
    if citation_text.is_some() || doi.is_some() {
        stac_extensions.push(SCIENTIFIC_EXT);
    }

    let mut links = vec![
        make_link(format!("{}/api/stac/collections/{}", base_url, slug), "self", "application/json"),
        make_link(format!("{}/api/stac", base_url), "root", "application/json"),
        make_link(format!("{}/api/stac", base_url), "parent", "application/json"),
        make_link(format!("{}/api/stac/collections/{}/items", base_url, slug), "items", "application/geo+json"),
        make_link_titled(
            format!("{}/api/layers/xyz/{{z}}/{{x}}/{{y}}?layer={{layer}}", base_url),
            "tiles", "application/vnd.mapbox-vector-tile", "XYZ Tile Template",
        ),
    ];

    if let Some(url) = paper_url {
        links.push(make_link_titled(url.to_string(), "cite-as", "text/html", "Publication"));
    }

    let mut collection_json = json!({
        "type": "Collection",
        "id": slug,
        "stac_version": "1.0.0",
        "stac_extensions": stac_extensions,
        "title": proj.title,
        "description": proj.description.as_deref().unwrap_or(""),
        "license": license,
        "extent": {
            "spatial": { "bbox": [bbox] },
            "temporal": { "interval": [[temporal_start, temporal_end]] }
        },
        "links": links,
        "providers": providers,
        "item_assets": {
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
        },
        "summaries": {
            "drop4crop:crop": data.crop_slugs,
            "drop4crop:water_model": data.water_model_slugs,
            "drop4crop:climate_model": data.climate_model_slugs,
            "drop4crop:scenario": data.scenario_slugs,
            "drop4crop:variable": data.variable_slugs,
        },
        "item_count": data.layer_count,
    });

    if let Some(kw) = &proj.keywords {
        if let Some(arr) = kw.as_array() {
            if !arr.is_empty() {
                collection_json["keywords"] = kw.clone();
            }
        }
    }

    if let Some(text) = citation_text {
        collection_json["sci:citation"] = json!(text);
    }
    if let Some(d) = &doi {
        collection_json["sci:doi"] = json!(d);
    }

    collection_json
}

pub async fn stac_root(
    headers: HeaderMap,
    State(app_state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let base_url = get_base_url(&headers);
    let db = &app_state.db;

    let projects = project::Entity::find()
        .filter(project::Column::Enabled.eq(true))
        .order_by_asc(project::Column::SortOrder)
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut catalog = Catalog::new("drop4crop", "Drop4Crop: Agricultural Water Stress and Crop Yield Data");
    catalog.description = "Spatiotemporal Asset Catalog providing global agricultural water stress and crop yield projections. Data and content provided by the CHANGE Lab at EPFL.".to_string();

    catalog.links.push(make_link(format!("{}/api/stac", base_url), "self", "application/json"));
    catalog.links.push(make_link(format!("{}/api/stac", base_url), "root", "application/json"));
    catalog.links.push(make_link(format!("{}/api/stac/collections", base_url), "data", "application/json"));
    catalog.links.push(make_link(format!("{}/api/stac/search", base_url), "search", "application/json"));
    catalog.links.push(make_link(format!("{}/api/stac/conformance", base_url), "conformance", "application/json"));

    for p in &projects {
        catalog.links.push(make_link_titled(
            format!("{}/api/stac/collections/{}", base_url, p.slug),
            "child", "application/json", &p.title,
        ));
    }

    catalog.additional_fields.insert(
        "conformsTo".to_string(),
        json!([
            "https://api.stacspec.org/v1.0.0/core",
            "https://api.stacspec.org/v1.0.0/collections",
            "https://api.stacspec.org/v1.0.0/item-search",
            "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/core",
            "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/geojson"
        ])
    );

    Ok(Json(serde_json::to_value(catalog).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

pub async fn stac_conformance() -> Json<Conformance> {
    Json(Conformance {
        conforms_to: vec![
            "https://api.stacspec.org/v1.0.0/core".to_string(),
            "https://api.stacspec.org/v1.0.0/collections".to_string(),
            "https://api.stacspec.org/v1.0.0/item-search".to_string(),
            "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/core".to_string(),
            "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/geojson".to_string(),
        ],
    })
}

pub async fn stac_collections(
    headers: HeaderMap,
    State(app_state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let base_url = get_base_url(&headers);
    let db = &app_state.db;

    let projects = project::Entity::find()
        .filter(project::Column::Enabled.eq(true))
        .order_by_asc(project::Column::SortOrder)
        .all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut collections = Vec::new();
    for proj in &projects {
        let data = gather_collection_data(proj.id, db).await?;
        collections.push(build_collection(proj, &data, &base_url));
    }

    Ok(Json(json!({
        "collections": collections,
        "links": [
            make_link(format!("{}/api/stac/collections", base_url), "self", "application/json"),
            make_link(format!("{}/api/stac", base_url), "root", "application/json"),
        ]
    })))
}

pub async fn stac_collection(
    headers: HeaderMap,
    Path(collection_id): Path<String>,
    State(app_state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, Json<String>)> {
    let base_url = get_base_url(&headers);
    let db = &app_state.db;

    let proj = project::Entity::find()
        .filter(project::Column::Slug.eq(&collection_id))
        .one(db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json("Collection not found".to_string())))?;

    let data = gather_collection_data(proj.id, db).await
        .map_err(|s| (s, Json("Failed to gather collection data".to_string())))?;

    Ok(Json(build_collection(&proj, &data, &base_url)))
}

pub async fn stac_items(
    headers: HeaderMap,
    Path(collection_id): Path<String>,
    Query(mut params): Query<SearchParams>,
    State(app_state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    params.project = Some(collection_id);
    search_items(headers, params, &app_state.db).await
}

pub async fn stac_item(
    headers: HeaderMap,
    Path((collection_id, item_id)): Path<(String, String)>,
    State(app_state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, Json<String>)> {
    let base_url = get_base_url(&headers);
    let db = &app_state.db;

    let proj = project::Entity::find()
        .filter(project::Column::Slug.eq(&collection_id))
        .one(db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json("Collection not found".to_string())))?;

    let layer_with_style = layer::Entity::find()
        .find_also_related(crate::routes::styles::db::Entity)
        .filter(layer::Column::Enabled.eq(true))
        .filter(layer::Column::ProjectId.eq(proj.id))
        .filter(layer::Column::LayerName.eq(&item_id))
        .one(db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json("Item not found".to_string())))?;

    let crop_map: HashMap<Uuid, String> = crop::Entity::find().all(db).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?
        .into_iter().map(|m| (m.id, m.slug)).collect();
    let water_model_map: HashMap<Uuid, String> = water_model::Entity::find().all(db).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?
        .into_iter().map(|m| (m.id, m.slug)).collect();
    let climate_model_map: HashMap<Uuid, String> = climate_model::Entity::find().all(db).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?
        .into_iter().map(|m| (m.id, m.slug)).collect();
    let scenario_map: HashMap<Uuid, String> = scenario::Entity::find().all(db).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?
        .into_iter().map(|m| (m.id, m.slug)).collect();
    let variable_map: HashMap<Uuid, String> = variable::Entity::find().all(db).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(e.to_string())))?
        .into_iter().map(|m| (m.id, m.slug)).collect();

    let bbox = proj.extent.as_ref()
        .and_then(|e| project_extent_to_bbox(e))
        .unwrap_or([-180.0, -90.0, 180.0, 90.0]);

    let item = build_item(
        &layer_with_style.0,
        layer_with_style.1.as_ref(),
        &proj.slug,
        bbox,
        &base_url,
        &crop_map,
        &water_model_map,
        &climate_model_map,
        &scenario_map,
        &variable_map,
    );

    Ok(Json(item))
}

pub async fn stac_search(
    headers: HeaderMap,
    Query(params): Query<SearchParams>,
    State(app_state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    search_items(headers, params, &app_state.db).await
}

async fn search_items(
    headers: HeaderMap,
    params: SearchParams,
    db: &sea_orm::DatabaseConnection,
) -> Result<Json<Value>, StatusCode> {
    let base_url = get_base_url(&headers);
    let err = |_| StatusCode::INTERNAL_SERVER_ERROR;

    let mut query = layer::Entity::find()
        .find_also_related(crate::routes::styles::db::Entity)
        .filter(layer::Column::Enabled.eq(true));

    if let Some(crop_slug) = &params.crop {
        let id = crop::Entity::find().filter(crop::Column::Slug.eq(crop_slug.as_str()))
            .one(db).await.map_err(err)?.map(|m| m.id);
        query = query.filter(layer::Column::CropId.eq(id.unwrap_or(Uuid::nil())));
    }
    if let Some(wm_slug) = &params.water_model {
        let id = water_model::Entity::find().filter(water_model::Column::Slug.eq(wm_slug.as_str()))
            .one(db).await.map_err(err)?.map(|m| m.id);
        query = query.filter(layer::Column::WaterModelId.eq(id.unwrap_or(Uuid::nil())));
    }
    if let Some(cm_slug) = &params.climate_model {
        let id = climate_model::Entity::find().filter(climate_model::Column::Slug.eq(cm_slug.as_str()))
            .one(db).await.map_err(err)?.map(|m| m.id);
        query = query.filter(layer::Column::ClimateModelId.eq(id.unwrap_or(Uuid::nil())));
    }
    if let Some(sc_slug) = &params.scenario {
        let id = scenario::Entity::find().filter(scenario::Column::Slug.eq(sc_slug.as_str()))
            .one(db).await.map_err(err)?.map(|m| m.id);
        query = query.filter(layer::Column::ScenarioId.eq(id.unwrap_or(Uuid::nil())));
    }
    if let Some(var_slug) = &params.variable {
        let id = variable::Entity::find().filter(variable::Column::Slug.eq(var_slug.as_str()))
            .one(db).await.map_err(err)?.map(|m| m.id);
        query = query.filter(layer::Column::VariableId.eq(id.unwrap_or(Uuid::nil())));
    }
    if let Some(project_slug) = &params.project {
        let p = project::Entity::find().filter(project::Column::Slug.eq(project_slug.as_str()))
            .one(db).await.map_err(err)?;
        query = query.filter(layer::Column::ProjectId.eq(p.map(|p| p.id).unwrap_or(Uuid::nil())));
    }
    if let Some(datetime) = &params.datetime {
        if let Some(year_str) = datetime.split('-').next()
            && let Ok(year) = year_str.parse::<i32>() {
                query = query.filter(layer::Column::Year.eq(year));
            }
    }

    query = query.order_by_asc(layer::Column::LayerName);
    let limit = params.limit.unwrap_or(100).min(10000);
    let offset = params.offset.unwrap_or(0);

    let matched = query.clone().count(db).await.map_err(err)?;

    let layers = query
        .offset(offset)
        .limit(limit as u64)
        .all(db).await.map_err(err)?;

    let crop_map: HashMap<Uuid, String> = crop::Entity::find().all(db).await.map_err(err)?
        .into_iter().map(|m| (m.id, m.slug)).collect();
    let water_model_map: HashMap<Uuid, String> = water_model::Entity::find().all(db).await.map_err(err)?
        .into_iter().map(|m| (m.id, m.slug)).collect();
    let climate_model_map: HashMap<Uuid, String> = climate_model::Entity::find().all(db).await.map_err(err)?
        .into_iter().map(|m| (m.id, m.slug)).collect();
    let scenario_map: HashMap<Uuid, String> = scenario::Entity::find().all(db).await.map_err(err)?
        .into_iter().map(|m| (m.id, m.slug)).collect();
    let variable_map: HashMap<Uuid, String> = variable::Entity::find().all(db).await.map_err(err)?
        .into_iter().map(|m| (m.id, m.slug)).collect();

    let project_map: HashMap<Uuid, (String, [f64; 4])> = project::Entity::find()
        .all(db).await.map_err(err)?
        .into_iter()
        .map(|p| {
            let bbox = p.extent.as_ref()
                .and_then(|e| project_extent_to_bbox(e))
                .unwrap_or([-180.0, -90.0, 180.0, 90.0]);
            (p.id, (p.slug, bbox))
        })
        .collect();

    let features: Vec<Value> = layers.iter().map(|(layer_record, style_opt)| {
        let (proj_slug, proj_bbox) = layer_record.project_id
            .and_then(|pid| project_map.get(&pid))
            .map(|(s, b)| (s.as_str(), *b))
            .unwrap_or(("unknown", [-180.0, -90.0, 180.0, 90.0]));

        build_item(
            layer_record, style_opt.as_ref(), proj_slug, proj_bbox, &base_url,
            &crop_map, &water_model_map, &climate_model_map, &scenario_map, &variable_map,
        )
    }).collect();

    let item_count = features.len();
    let items: Vec<serde_json::Map<String, Value>> = features.into_iter()
        .filter_map(|f| f.as_object().cloned()).collect();

    let mut item_collection = ItemCollection::new(items)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    item_collection.links.push(
        Link::new(format!("{}/api/stac/search?limit={}&offset={}", base_url, limit, offset), "self")
            .r#type(Some("application/geo+json".to_string()))
    );
    item_collection.links.push(
        Link::new(format!("{}/api/stac", base_url), "root")
            .r#type(Some("application/json".to_string()))
    );

    let next_offset = offset + item_count as u64;
    if next_offset < matched {
        item_collection.links.push(
            Link::new(format!("{}/api/stac/search?limit={}&offset={}", base_url, limit, next_offset), "next")
                .r#type(Some("application/geo+json".to_string()))
        );
    }
    if offset > 0 {
        let prev_offset = offset.saturating_sub(limit as u64);
        item_collection.links.push(
            Link::new(format!("{}/api/stac/search?limit={}&offset={}", base_url, limit, prev_offset), "prev")
                .r#type(Some("application/geo+json".to_string()))
        );
    }

    item_collection.context = Some(Context {
        returned: item_count as u64,
        limit: Some(limit as u64),
        matched: Some(matched),
        additional_fields: Default::default(),
    });

    let mut response = serde_json::to_value(item_collection).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    response["numberMatched"] = json!(matched);
    response["numberReturned"] = json!(item_count);
    Ok(Json(response))
}

fn build_item(
    layer_record: &layer::Model,
    style_opt: Option<&crate::routes::styles::db::Model>,
    collection_slug: &str,
    bbox: [f64; 4],
    base_url: &str,
    crop_map: &HashMap<Uuid, String>,
    water_model_map: &HashMap<Uuid, String>,
    climate_model_map: &HashMap<Uuid, String>,
    scenario_map: &HashMap<Uuid, String>,
    variable_map: &HashMap<Uuid, String>,
) -> Value {
    let unknown = "unknown".to_string();
    let layer_name = layer_record.layer_name.as_ref().unwrap_or(&unknown);
    let year = layer_record.year.unwrap_or(2010);

    let crop_slug = layer_record.crop_id.and_then(|id| crop_map.get(&id).cloned()).unwrap_or_default();
    let wm_slug = layer_record.water_model_id.and_then(|id| water_model_map.get(&id).cloned()).unwrap_or_default();
    let cm_slug = layer_record.climate_model_id.and_then(|id| climate_model_map.get(&id).cloned()).unwrap_or_default();
    let sc_slug = layer_record.scenario_id.and_then(|id| scenario_map.get(&id).cloned()).unwrap_or_default();
    let var_slug = layer_record.variable_id.and_then(|id| variable_map.get(&id).cloned()).unwrap_or_default();

    let style_json = style_opt.and_then(|s| s.style.as_ref());
    let interpolation_type = style_opt.map(|s| s.interpolation_type.as_str()).unwrap_or("linear");
    let label_display_mode = style_opt.map(|s| s.label_display_mode.as_str()).unwrap_or("auto");
    let label_count = style_opt.and_then(|s| s.label_count);

    let var_d = if var_slug.is_empty() { "unknown" } else { &var_slug };

    let title_parts: Vec<&str> = [
        if crop_slug.is_empty() { None } else { Some(crop_slug.as_str()) },
        if wm_slug.is_empty() { None } else { Some(wm_slug.as_str()) },
        if cm_slug.is_empty() { None } else { Some(cm_slug.as_str()) },
        if sc_slug.is_empty() { None } else { Some(sc_slug.as_str()) },
    ].into_iter().flatten().collect();

    let title = if title_parts.is_empty() {
        format!("{} ({})", year, var_d)
    } else {
        format!("{} {} ({})", title_parts.join(" "), year, var_d)
    };

    let desc_parts: Vec<String> = [
        if crop_slug.is_empty() { None } else { Some(format!("crop: {}", crop_slug)) },
        if wm_slug.is_empty() { None } else { Some(format!("water model: {}", wm_slug)) },
        if cm_slug.is_empty() { None } else { Some(format!("climate model: {}", cm_slug)) },
        if sc_slug.is_empty() { None } else { Some(format!("scenario: {}", sc_slug)) },
    ].into_iter().flatten().collect();

    let description = if desc_parts.is_empty() {
        format!("{} for year {}", var_d, year)
    } else {
        format!("{} — {}", var_d, desc_parts.join(", "))
    };

    let [west, south, east, north] = bbox;
    let geometry = json!({
        "type": "Polygon",
        "coordinates": [[[west, south], [east, south], [east, north], [west, north], [west, south]]]
    });

    json!({
        "stac_version": "1.0.0",
        "stac_extensions": [
            PROJECTION_EXT,
            RASTER_EXT,
            DROP4CROP_EXT,
        ],
        "type": "Feature",
        "id": layer_name,
        "collection": collection_slug,
        "geometry": geometry,
        "bbox": [west, south, east, north],
        "properties": {
            "datetime": format!("{}-01-01T00:00:00Z", year),
            "start_datetime": format!("{}-01-01T00:00:00Z", year),
            "end_datetime": format!("{}-12-31T23:59:59Z", year),
            "title": title,
            "description": description,
            "drop4crop:crop": crop_slug,
            "drop4crop:water_model": wm_slug,
            "drop4crop:climate_model": cm_slug,
            "drop4crop:scenario": sc_slug,
            "drop4crop:variable": var_slug,
            "drop4crop:year": year,
            "drop4crop:global_average": layer_record.global_average,
            "drop4crop:min_value": layer_record.min_value,
            "drop4crop:max_value": layer_record.max_value,
            "drop4crop:style": style_json,
            "drop4crop:interpolation_type": interpolation_type,
            "drop4crop:label_display_mode": label_display_mode,
            "drop4crop:label_count": label_count,
        },
        "links": [
            {
                "rel": "self",
                "type": "application/geo+json",
                "href": format!("{}/api/stac/collections/{}/items/{}", base_url, collection_slug, layer_name)
            },
            {
                "rel": "collection",
                "type": "application/json",
                "href": format!("{}/api/stac/collections/{}", base_url, collection_slug)
            },
            {
                "rel": "parent",
                "type": "application/json",
                "href": format!("{}/api/stac/collections/{}", base_url, collection_slug)
            },
            {
                "rel": "root",
                "type": "application/json",
                "href": format!("{}/api/stac", base_url)
            },
        ],
        "assets": {
            "rendered_preview": {
                "href": format!("{}/api/layers/xyz/{{z}}/{{x}}/{{y}}?layer={}", base_url, layer_name),
                "type": "image/png",
                "roles": ["visual", "overview"],
                "title": "XYZ Tiles (EPSG:3857)",
                "description": "Pre-rendered PNG tiles for web mapping",
                "proj:epsg": 3857,
                "proj:shape": [256, 256],
                "proj:bbox": [-20037508.34, -20037508.34, 20037508.34, 20037508.34],
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
                    "unit": var_d,
                    "statistics": {
                        "minimum": layer_record.min_value.unwrap_or(0.0),
                        "maximum": layer_record.max_value.unwrap_or(1.0),
                        "mean": layer_record.global_average
                    }
                }]
            }
        }
    })
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
