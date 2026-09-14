use super::*;

fn layer(width: Option<i32>, height: Option<i32>, resolution: Option<f64>, data_type: Option<&str>) -> layer::Model {
    layer::Model {
        id: Uuid::nil(),
        layer_name: Some("all_null_null_null_eta_2000".to_string()),
        crop_id: None,
        water_model_id: None,
        climate_model_id: None,
        scenario_id: None,
        variable_id: None,
        year: Some(2000),
        last_updated: chrono::Utc::now(),
        enabled: true,
        uploaded_at: chrono::Utc::now(),
        global_average: Some(2.0),
        filename: Some("all_null_null_null_eta_2000.tif".to_string()),
        min_value: Some(1.0),
        max_value: Some(3.0),
        project_id: None,
        style_id: None,
        total_views: 0,
        stats_status: None,
        stats_status_value: None,
        file_size: None,
        raster_width: width,
        raster_height: height,
        raster_resolution: resolution,
        raster_data_type: data_type.map(str::to_string),
        cache_status: None,
        stats: None,
    }
}

fn item(record: &layer::Model, bbox: [f64; 4]) -> Value {
    let empty = HashMap::new();
    build_item(
        record, None, "india", bbox, "https://drop4crop.epfl.ch",
        &empty, &empty, &empty, &empty, &empty,
    )
}

#[test]
fn test_build_item_data_asset_reports_the_rasters_own_shape() {
    let record = layer(Some(58), Some(57), Some(0.5), Some("float64"));
    let asset = item(&record, [42.0117, 5.0909, 112.1484, 37.1603])["assets"]["data"].clone();

    // proj:shape is [height, width]
    assert_eq!(asset["proj:shape"], json!([57, 58]));
    assert_eq!(asset["bands"][0]["data_type"], json!("float64"));
}

// The stored resolution is the pixel width in degrees, while the raster extension defines
// spatial_resolution in metres, so the value is published under a drop4crop: property instead.
#[test]
fn test_build_item_publishes_the_pixel_size_in_degrees_not_as_a_band_resolution() {
    let record = layer(Some(58), Some(57), Some(0.5), Some("float64"));
    let value = item(&record, [42.0117, 5.0909, 112.1484, 37.1603]);

    assert_eq!(value["properties"]["drop4crop:raster_resolution"], json!(0.5));
    assert!(value["assets"]["data"]["bands"][0].get("raster:spatial_resolution").is_none());
}

#[test]
fn test_build_item_carries_band_metadata_in_the_asset_bands_array() {
    let record = layer(Some(58), Some(57), Some(0.5), Some("float64"));
    let value = item(&record, [-180.0, -90.0, 180.0, 90.0]);
    let asset = value["assets"]["data"].clone();

    assert!(asset.get("raster:bands").is_none());
    assert_eq!(asset["bands"][0]["unit"], json!("unknown"));
    assert_eq!(asset["bands"][0]["statistics"], json!({"minimum": 1.0, "maximum": 3.0, "mean": 2.0}));

    let declared = value["stac_extensions"].as_array().unwrap();
    assert!(declared.contains(&json!("https://stac-extensions.github.io/raster/v2.0.0/schema.json")));
}

#[test]
fn test_build_item_proj_bbox_matches_the_item_bbox() {
    let record = layer(Some(4320), Some(2160), Some(0.0833), Some("float64"));
    let bbox = [42.0117, 5.0909, 112.1484, 37.1603];
    let value = item(&record, bbox);

    assert_eq!(value["bbox"], json!(bbox));
    assert_eq!(value["assets"]["data"]["proj:bbox"], json!(bbox));
}

#[test]
fn test_build_item_omits_raster_fields_a_layer_has_no_values_for() {
    let record = layer(None, None, None, None);
    let asset = item(&record, [-180.0, -90.0, 180.0, 90.0])["assets"]["data"].clone();

    assert!(asset.get("proj:shape").is_none());
    assert!(asset["bands"][0].get("data_type").is_none());
}

fn project() -> project::Model {
    project::Model {
        id: Uuid::nil(),
        slug: "crop-water-use".to_string(),
        title: "Crop water use".to_string(),
        description: Some("Global crop water footprints".to_string()),
        enabled: true,
        sort_order: 0,
        year_axis: None,
        historical_year: None,
        tab_config: None,
        card_layer_id: None,
        card_style_id: None,
        extent: None,
        citation: None,
        unavailable_message: None,
        license: Some("CC-BY-4.0".to_string()),
        providers: None,
        keywords: None,
    }
}

fn collection_data() -> CollectionData {
    CollectionData {
        crop_slugs: vec!["barley".to_string()],
        water_model_slugs: vec![],
        climate_model_slugs: vec![],
        scenario_slugs: vec![],
        variable_slugs: vec!["eta".to_string()],
        min_year: Some(2000),
        max_year: Some(2090),
    }
}

#[test]
fn test_build_collection_item_assets_carry_no_href() {
    let collection = build_collection(&project(), &collection_data(), "https://drop4crop.epfl.ch");
    let item_assets = collection["item_assets"].as_object().unwrap();

    assert!(!item_assets.is_empty());
    for (name, asset) in item_assets {
        assert!(asset.get("href").is_none(), "{} carries an href", name);
    }
}

#[test]
fn test_build_collection_declares_no_item_assets_extension() {
    let collection = build_collection(&project(), &collection_data(), "https://drop4crop.epfl.ch");

    for ext in collection["stac_extensions"].as_array().unwrap() {
        assert!(
            !ext.as_str().unwrap().contains("item-assets"),
            "{} is declared for a field the core Collection spec defines",
            ext
        );
    }
}

#[test]
fn test_build_collection_emits_no_undeclared_fields() {
    let collection = build_collection(&project(), &collection_data(), "https://drop4crop.epfl.ch");
    let tiles = &collection["item_assets"]["tiles"];

    assert!(collection.get("item_count").is_none());
    assert!(tiles.get("tile:scheme").is_none());
    assert!(tiles.get("tile:min_zoom").is_none());
    assert!(tiles.get("tile:max_zoom").is_none());
}

#[test]
fn test_build_collection_tiles_link_is_a_png_template_over_item_ids() {
    let collection = build_collection(&project(), &collection_data(), "https://drop4crop.epfl.ch");
    let links = collection["links"].as_array().unwrap();
    let tiles = links.iter().find(|l| l["rel"] == json!("tiles")).unwrap();

    assert_eq!(tiles["type"], json!("image/png"));
    assert_eq!(
        tiles["href"],
        json!("https://drop4crop.epfl.ch/api/layers/xyz/{z}/{x}/{y}?layer={item_id}")
    );
}

#[test]
fn test_the_declared_drop4crop_extension_is_served_by_this_api() {
    let record = layer(Some(58), Some(57), Some(0.5), Some("float64"));
    let value = item(&record, [-180.0, -90.0, 180.0, 90.0]);
    let declared = value["stac_extensions"].as_array().unwrap();
    let schema = drop4crop_extension_schema("https://drop4crop.epfl.ch");

    assert!(declared.contains(&schema["$id"]));
    assert_eq!(
        schema["$id"],
        json!("https://drop4crop.epfl.ch/api/stac/extensions/drop4crop/v1.0.0/schema.json")
    );
}

#[test]
fn test_every_drop4crop_property_an_item_emits_is_in_the_schema() {
    let record = layer(Some(58), Some(57), Some(0.5), Some("float64"));
    let value = item(&record, [-180.0, -90.0, 180.0, 90.0]);
    let schema = drop4crop_extension_schema("https://drop4crop.epfl.ch");
    let described = &schema["properties"]["properties"]["properties"];

    let emitted: Vec<&String> = value["properties"].as_object().unwrap().keys()
        .filter(|k| k.starts_with("drop4crop:"))
        .collect();
    assert!(!emitted.is_empty());
    for key in emitted {
        assert!(described.get(key).is_some(), "{} is not in the schema", key);
    }
}

#[test]
fn test_the_schema_path_items_declare_is_the_path_the_router_serves() {
    assert_eq!(DROP4CROP_EXT_PATH, format!("/api/stac{}", DROP4CROP_EXT_ROUTE));
}

#[test]
fn test_item_description_uses_no_em_dash() {
    let mut record = layer(None, None, None, None);
    record.crop_id = Some(Uuid::nil());
    let mut crops = HashMap::new();
    crops.insert(Uuid::nil(), "barley".to_string());
    let empty = HashMap::new();
    let value = build_item(
        &record, None, "crop-water-use", [-180.0, -90.0, 180.0, 90.0], "https://drop4crop.epfl.ch",
        &crops, &empty, &empty, &empty, &empty,
    );
    let description = value["properties"]["description"].as_str().unwrap();

    assert!(description.contains("crop: barley"));
    assert!(!description.contains('\u{2014}'), "{}", description);
}

#[test]
fn test_build_collection_states_no_licence_the_project_row_does_not_carry() {
    let mut proj = project();
    proj.license = None;
    let collection = build_collection(&proj, &collection_data(), "https://drop4crop.epfl.ch");

    assert_eq!(collection["license"], json!("other"));
}

#[test]
fn test_build_collection_serves_the_licence_on_the_project_row() {
    let mut proj = project();
    proj.license = Some("CC-BY-4.0".to_string());
    let collection = build_collection(&proj, &collection_data(), "https://drop4crop.epfl.ch");

    assert_eq!(collection["license"], json!("CC-BY-4.0"));
}

#[test]
fn test_build_collection_lists_no_providers_the_project_row_does_not_carry() {
    let mut proj = project();
    proj.providers = None;
    let collection = build_collection(&proj, &collection_data(), "https://drop4crop.epfl.ch");

    assert!(collection.get("providers").is_none());
}

#[test]
fn test_build_collection_serves_the_providers_on_the_project_row() {
    let mut proj = project();
    proj.providers = Some(json!([{"name": "CHANGE Lab - EPFL", "roles": ["producer"]}]));
    let collection = build_collection(&proj, &collection_data(), "https://drop4crop.epfl.ch");

    assert_eq!(collection["providers"][0]["name"], json!("CHANGE Lab - EPFL"));
}

#[test]
fn test_build_item_asset_hrefs_are_fetchable_urls() {
    let record = layer(Some(4320), Some(2160), Some(0.0833), Some("float64"));
    let assets = item(&record, [-180.0, -90.0, 180.0, 90.0])["assets"].clone();

    for (name, asset) in assets.as_object().unwrap() {
        let href = asset["href"].as_str().unwrap();
        assert!(!href.contains('{'), "asset {name} href is a template: {href}");
    }
}

#[test]
fn test_item_publishes_the_tiles_as_a_tilejson_asset() {
    let record = layer(None, None, None, None);
    let asset = item(&record, [-180.0, -90.0, 180.0, 90.0])["assets"]["tilejson"].clone();

    assert_eq!(asset["type"], json!("application/json"));
    assert_eq!(
        asset["href"],
        json!(crate::routes::layers::tilejson::tilejson_href(
            "https://drop4crop.epfl.ch",
            "all_null_null_null_eta_2000"
        ))
    );
}

#[test]
fn test_every_item_assets_key_is_an_asset_the_items_carry() {
    let collection = build_collection(&project(), &collection_data(), "https://drop4crop.epfl.ch");
    let record = layer(Some(58), Some(57), Some(0.5), Some("float64"));
    let assets = item(&record, [-180.0, -90.0, 180.0, 90.0])["assets"].clone();

    let declared = collection["item_assets"].as_object().unwrap();
    assert!(!declared.is_empty());
    for (key, definition) in declared {
        let asset = assets.get(key).unwrap_or_else(|| panic!("no item asset {}", key));
        assert_eq!(asset["type"], definition["type"], "{} type", key);
        assert_eq!(asset["roles"], definition["roles"], "{} roles", key);
    }
}

#[test]
fn test_every_document_declares_the_same_stac_version() {
    let collection = build_collection(&project(), &collection_data(), "https://drop4crop.epfl.ch");
    let item = item(&layer(Some(58), Some(57), Some(0.5), Some("float64")), [0.0, 0.0, 1.0, 1.0]);

    assert_eq!(collection["stac_version"], json!(STAC_VERSION));
    assert_eq!(item["stac_version"], json!(STAC_VERSION));
    assert_eq!(
        Catalog::new("drop4crop", "Drop4Crop").version.to_string(),
        STAC_VERSION,
        "the stac crate stamps the root, and it no longer agrees with the children"
    );
}
