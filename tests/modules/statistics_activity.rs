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
    let wheat = layers[1]["daily_requests"].as_array().unwrap();
    assert_eq!(wheat.len(), 37);
    assert!(wheat[..36].iter().all(|value| value == 0));
    assert_eq!(wheat[36], 998);
}

#[tokio::test]
async fn activity_rejects_an_inverted_range() {
    let client = TestClient::new(create_test_app().await);
    let response = client
        .get("/api/statistics/activity?start_date=2024-02-25&end_date=2024-01-20")
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
}
