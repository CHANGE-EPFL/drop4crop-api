// HTTP client utilities for testing

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::Value;
use tower::ServiceExt;

/// Test client for making HTTP requests
pub struct TestClient {
    router: Router,
    auth_token: Option<String>,
}

impl TestClient {
    pub fn new(router: Router) -> Self {
        Self {
            router,
            auth_token: None,
        }
    }

    /// Set the authorization token (JWT)
    pub fn with_auth(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(token.into());
        self
    }

    /// Make a GET request
    pub async fn get(&self, uri: &str) -> TestResponse {
        let mut request = Request::builder()
            .method("GET")
            .uri(uri);

        if let Some(token) = &self.auth_token {
            request = request.header("authorization", format!("Bearer {}", token));
        }

        let request = request.body(Body::empty()).unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();

        TestResponse::new(response).await
    }

    /// Make a POST request with JSON body
    pub async fn post(&self, uri: &str, body: &Value) -> TestResponse {
        let mut request = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");

        if let Some(token) = &self.auth_token {
            request = request.header("authorization", format!("Bearer {}", token));
        }

        let body_bytes = serde_json::to_vec(body).unwrap();
        let request = request.body(Body::from(body_bytes)).unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();

        TestResponse::new(response).await
    }

    /// Make a PUT request with JSON body
    pub async fn put(&self, uri: &str, body: &Value) -> TestResponse {
        let mut request = Request::builder()
            .method("PUT")
            .uri(uri)
            .header("content-type", "application/json");

        if let Some(token) = &self.auth_token {
            request = request.header("authorization", format!("Bearer {}", token));
        }

        let body_bytes = serde_json::to_vec(body).unwrap();
        let request = request.body(Body::from(body_bytes)).unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();

        TestResponse::new(response).await
    }

    /// Make a DELETE request
    pub async fn delete(&self, uri: &str) -> TestResponse {
        let mut request = Request::builder()
            .method("DELETE")
            .uri(uri);

        if let Some(token) = &self.auth_token {
            request = request.header("authorization", format!("Bearer {}", token));
        }

        let request = request.body(Body::empty()).unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();

        TestResponse::new(response).await
    }

    /// Make a DELETE request with JSON body (for batch operations)
    pub async fn delete_with_body(&self, uri: &str, body: &Value) -> TestResponse {
        let mut request = Request::builder()
            .method("DELETE")
            .uri(uri)
            .header("content-type", "application/json");

        if let Some(token) = &self.auth_token {
            request = request.header("authorization", format!("Bearer {}", token));
        }

        let body_bytes = serde_json::to_vec(body).unwrap();
        let request = request.body(Body::from(body_bytes)).unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();

        TestResponse::new(response).await
    }
}

/// Test response wrapper
pub struct TestResponse {
    pub status: StatusCode,
    pub body: Value,
    pub headers: axum::http::HeaderMap,
}

impl TestResponse {
    async fn new(response: axum::response::Response) -> Self {
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        let body: Value = if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap_or(Value::String(
                String::from_utf8_lossy(&body).to_string()
            ))
        };

        Self { status, body, headers }
    }

    /// Assert the status code
    pub fn assert_status(&self, expected: StatusCode) {
        assert_eq!(
            self.status, expected,
            "Expected status {}, got {}. Body: {}",
            expected, self.status, self.body
        );
    }

    /// Assert the response is successful (2xx)
    pub fn assert_success(&self) {
        assert!(
            self.status.is_success(),
            "Expected success status, got {}. Body: {}",
            self.status, self.body
        );
    }

    /// Get JSON value from response
    pub fn json(&self) -> &Value {
        &self.body
    }

    /// Get header value
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)?.to_str().ok()
    }
}
