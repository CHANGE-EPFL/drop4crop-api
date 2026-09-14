use super::*;

// Scenario: GDAL probes a COG with HEAD before reading it. The EPFL edge rewrites
// Content-Length to 0 on every HEAD it fronts, and GDAL takes that as a zero-byte file.
// Expected behaviour: the route refuses HEAD so GDAL falls back to a ranged GET and reads
// the total from Content-Range.
#[tokio::test]
async fn test_head_cog_data_refuses_the_method() {
    let response = head_cog_data().await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()[header::ALLOW], "GET");
}
