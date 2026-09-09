// Where a router is mounted decides what path its middleware sees, which is not
// visible below route level.

use crate::common::client::TestClient;
use crate::common::db::create_real_app;
use crate::skip_if_no_redis;
use drop4crop_api::config::Config;
use drop4crop_api::routes::tiles::cache::{
    build_stats_key, get_redis_client, push_cache_raw, rendered_tile_key_for_request,
};

const LAYER: &str = "maize_cwatm_gfdl-esm2m_rcp26_vwc_2020";

async fn take_stats_key(config: &Config, stat_type: &str) -> i64 {
    let client = get_redis_client(config);
    let mut con = client.get_multiplexed_async_connection().await.unwrap();
    let key = build_stats_key(config, LAYER, stat_type);

    // The increment is spawned, so it lands shortly after the response.
    for _ in 0..40 {
        let count: Option<i64> = redis::cmd("GET")
            .arg(&key)
            .query_async(&mut con)
            .await
            .unwrap_or(None);
        if let Some(count) = count {
            let _: () = redis::cmd("DEL").arg(&key).query_async(&mut con).await.unwrap();
            return count;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    0
}

async fn clear_stats_key(config: &Config, stat_type: &str) {
    let client = get_redis_client(config);
    let mut con = client.get_multiplexed_async_connection().await.unwrap();
    let key = build_stats_key(config, LAYER, stat_type);
    let _: () = redis::cmd("DEL").arg(&key).query_async(&mut con).await.unwrap();
}

#[tokio::test]
async fn test_stac_item_request_counts_against_its_layer() {
    skip_if_no_redis!();
    let (router, config) = create_real_app().await;
    clear_stats_key(&config, "stac").await;

    let client = TestClient::new(router).with_header("x-forwarded-for", "203.0.113.5");
    client
        .get(&format!("/api/stac/collections/crop-water-use/items/{LAYER}"))
        .await;

    assert_eq!(take_stats_key(&config, "stac").await, 1);
}

#[tokio::test]
async fn test_card_tile_request_counts_as_a_tile() {
    skip_if_no_redis!();
    let (router, config) = create_real_app().await;
    clear_stats_key(&config, "xyz").await;

    let client = TestClient::new(router).with_header("x-forwarded-for", "203.0.113.5");
    client
        .get(&format!("/api/projects/crop-water-use/card-tile/4/8/5?layer={LAYER}"))
        .await;

    assert_eq!(take_stats_key(&config, "xyz").await, 1);
}

// A cache outcome is only visible inside the handler: the middleware runs before it
// and cannot see whether Redis answered.

async fn take_outcomes(config: &Config) -> (i64, i64) {
    (
        take_stats_key(config, "hit").await,
        take_stats_key(config, "miss").await,
    )
}

#[tokio::test]
async fn test_a_tile_served_from_cache_counts_a_hit() {
    skip_if_no_redis!();
    let (router, config) = create_real_app().await;
    clear_stats_key(&config, "hit").await;
    clear_stats_key(&config, "miss").await;

    let key = rendered_tile_key_for_request(&config, LAYER, None, 3, 4, 2);
    push_cache_raw(&config, &key, b"not really a png").await.unwrap();

    let client = TestClient::new(router).with_header("x-forwarded-for", "203.0.113.5");
    let response = client
        .get(&format!("/api/layers/xyz/3/4/2?layer={LAYER}"))
        .await;
    response.assert_success();

    assert_eq!(take_outcomes(&config).await, (1, 0));
}

// The miss is recorded as soon as the layer resolves, before the render, so the
// assertion does not wait for a tile this environment cannot produce.
#[tokio::test]
async fn test_a_tile_that_has_to_be_rendered_counts_a_miss() {
    skip_if_no_redis!();
    let (router, config) = create_real_app().await;
    clear_stats_key(&config, "hit").await;
    clear_stats_key(&config, "miss").await;

    let client = TestClient::new(router).with_header("x-forwarded-for", "203.0.113.5");
    let request = tokio::spawn(async move {
        client
            .get(&format!("/api/layers/xyz/3/4/3?layer={LAYER}"))
            .await
    });

    assert_eq!(take_stats_key(&config, "miss").await, 1);
    assert_eq!(take_stats_key(&config, "hit").await, 0);
    request.abort();
}

#[tokio::test]
async fn test_a_tile_for_a_layer_that_does_not_exist_counts_neither() {
    skip_if_no_redis!();
    let (router, config) = create_real_app().await;
    clear_stats_key(&config, "hit").await;
    clear_stats_key(&config, "miss").await;

    let client = TestClient::new(router).with_header("x-forwarded-for", "203.0.113.5");
    client.get("/api/layers/xyz/3/4/2?layer=no_such_layer").await;

    assert_eq!(take_outcomes(&config).await, (0, 0));
}
