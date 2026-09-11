use axum::http::StatusCode;

use crate::common::client::TestClient;
use crate::common::db::create_test_app;

#[tokio::test]
async fn activity_returns_each_layer_across_the_inclusive_range() {
    let client = TestClient::new(create_test_app().await);

    let response = client
        .get("/api/statistics/activity?start_date=2024-01-20&end_date=2024-02-25")
        .await;

    response.assert_status(StatusCode::OK);
    assert_eq!(response.body["earliest_date"], "2024-01-20");
    assert_eq!(response.body["latest_date"], "2024-02-25");
    assert_eq!(response.body["total_layers"], 2);
    let dates = response.body["dates"].as_array().unwrap();
    assert_eq!(dates.len(), 37);
    assert_eq!(dates.first().unwrap(), "2024-01-20");
    assert_eq!(dates.last().unwrap(), "2024-02-25");

    let layers = response.body["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2);
    assert_eq!(
        layers[0]["layer_name"],
        "maize_cwatm_gfdl-esm2m_rcp26_vwc_2020"
    );
    assert_eq!(layers[0]["total_requests"], 1654);
    let maize = layers[0]["daily_requests"].as_array().unwrap();
    assert_eq!(maize.len(), 37);
    assert_eq!(maize[0], 1445);
    assert_eq!(maize[1], 19);
    assert_eq!(maize[2], 190);
    assert!(maize[3..].iter().all(|value| value == 0));
    assert_eq!(
        layers[1]["layer_name"],
        "wheat_h08_hadgem2-es_rcp85_vwcb_2025"
    );
    assert_eq!(layers[1]["total_requests"], 998);
    let wheat = layers[1]["daily_requests"].as_array().unwrap();
    assert_eq!(wheat.len(), 37);
    assert!(wheat[..36].iter().all(|value| value == 0));
    assert_eq!(wheat[36], 998);
}

#[tokio::test]
async fn activity_describes_each_layer_by_its_reference_names() {
    let client = TestClient::new(create_test_app().await);

    let response = client.get("/api/statistics/activity").await;

    response.assert_status(StatusCode::OK);
    let maize = &response.body["layers"][0];
    assert_eq!(maize["layer_id"], "650e8400-e29b-41d4-a716-446655440001");
    assert_eq!(maize["crop"], "Maize");
    assert_eq!(maize["water_model"], "CWatM");
    assert_eq!(maize["climate_model"], "GFDL-ESM2M");
    assert_eq!(maize["scenario"], "RCP 2.6");
    assert_eq!(maize["year"], 2020);
    assert!(maize["project"].is_null());
}

#[tokio::test]
async fn activity_sums_every_day_between_the_bounds_for_the_period_picker() {
    let client = TestClient::new(create_test_app().await);

    let response = client
        .get("/api/statistics/activity?start_date=2024-02-01&end_date=2024-02-25")
        .await;

    response.assert_status(StatusCode::OK);
    // The picker series spans the full bounds even when a narrower period is selected
    let totals = response.body["daily_totals"].as_array().unwrap();
    assert_eq!(totals.len(), 37);
    assert_eq!(totals[0], 1445);
    assert_eq!(totals[1], 19);
    assert_eq!(totals[2], 190);
    assert!(totals[3..36].iter().all(|value| value == 0));
    assert_eq!(totals[36], 998);
    // Only the layer requested inside the period is listed
    assert_eq!(response.body["total_layers"], 1);
    let layers = response.body["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 1);
    assert_eq!(
        layers[0]["layer_name"],
        "wheat_h08_hadgem2-es_rcp85_vwcb_2025"
    );
}

#[tokio::test]
async fn activity_pages_layers_most_requested_first() {
    let client = TestClient::new(create_test_app().await);

    let first = client.get("/api/statistics/activity?limit=1").await;
    first.assert_status(StatusCode::OK);
    assert_eq!(first.body["total_layers"], 2);
    assert_eq!(first.body["limit"], 1);
    assert_eq!(first.body["offset"], 0);
    let layers = first.body["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 1);
    assert_eq!(
        layers[0]["layer_name"],
        "maize_cwatm_gfdl-esm2m_rcp26_vwc_2020"
    );

    let second = client
        .get("/api/statistics/activity?limit=1&offset=1")
        .await;
    second.assert_status(StatusCode::OK);
    let layers = second.body["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 1);
    assert_eq!(
        layers[0]["layer_name"],
        "wheat_h08_hadgem2-es_rcp85_vwcb_2025"
    );
}

#[tokio::test]
async fn activity_rejects_an_inverted_range() {
    let client = TestClient::new(create_test_app().await);
    let response = client
        .get("/api/statistics/activity?start_date=2024-02-25&end_date=2024-01-20")
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
}
