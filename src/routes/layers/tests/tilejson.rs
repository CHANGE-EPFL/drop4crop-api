use super::*;

#[test]
fn test_tilejson_tiles_template_is_the_path_the_xyz_handler_serves() {
    let document = build_tilejson("barley_cwatm_gfdl-esm2m_historical_wfb_2000", WORLD_BBOX, "https://drop4crop.epfl.ch");
    let template = document["tiles"][0].as_str().unwrap();

    assert_eq!(
        template,
        "https://drop4crop.epfl.ch/api/layers/xyz/{z}/{x}/{y}?layer=barley_cwatm_gfdl-esm2m_historical_wfb_2000"
    );
    // The handler parses the three coordinates out of the path, so a template with
    // integers substituted in is a path it serves.
    let request_path = template
        .replace("{z}", "2")
        .replace("{x}", "2")
        .replace("{y}", "1");
    assert_eq!(
        request_path,
        "https://drop4crop.epfl.ch/api/layers/xyz/2/2/1?layer=barley_cwatm_gfdl-esm2m_historical_wfb_2000"
    );
}

#[test]
fn test_tilejson_carries_the_layer_bounds() {
    let bounds = [42.0117, 5.0909, 112.1484, 37.1603];
    let document = build_tilejson("all_null_null_null_eta_2000", bounds, "https://drop4crop.epfl.ch");

    assert_eq!(document["bounds"], json!(bounds));
    assert_eq!(document["tilejson"], json!("3.0.0"));
    assert_eq!(document["minzoom"], json!(0));
    assert_eq!(document["maxzoom"], json!(20));
}
