use axum::{body::Body, http::{Request, StatusCode}};
use pagerune::{AppState, app};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tower::ServiceExt;

fn test_app() -> axum::Router {
    let database = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(100))
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .expect("valid test URL");
    app(AppState { database })
}

#[tokio::test]
async fn liveness_does_not_require_database() {
    let response = test_app().oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn readiness_fails_when_database_is_unavailable() {
    let response = test_app().oneshot(Request::builder().uri("/readyz").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn openapi_documents_every_implemented_operation() {
    use utoipa::OpenApi;
    let spec = serde_json::to_value(pagerune::ApiDoc::openapi()).unwrap();
    assert!(spec["paths"]["/healthz"]["get"].is_object());
    assert!(spec["paths"]["/readyz"]["get"].is_object());
    assert!(spec["paths"]["/readyz"]["get"]["responses"]["503"].is_object());
    let response = test_app().oneshot(Request::builder().uri("/api/openapi.json").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
