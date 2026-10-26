use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use pagerune::{AppState, app};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, time::Duration};
use tower::ServiceExt;

fn database_url() -> String {
    env::var("DATABASE_URL")
        .expect("DATABASE_URL is required; start Compose PostgreSQL and run tests through direnv")
}

async fn connected_database() -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&database_url())
        .await
        .expect("PostgreSQL must be reachable for integration tests")
}

async fn response(uri: &str, database: PgPool) -> axum::response::Response {
    app(AppState { database })
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn liveness_does_not_require_database() {
    let unavailable = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(100))
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    assert_eq!(
        response("/healthz", unavailable).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn readiness_succeeds_with_postgres() {
    assert_eq!(
        response("/readyz", connected_database().await)
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn readiness_fails_when_database_is_unavailable_with_consistent_error() {
    let unavailable = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(100))
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let response = response("/readyz", unavailable).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["code"], "not_ready");
    assert!(value["request_id"].as_str().is_some());
}

#[tokio::test]
async fn postgres_connection_is_usable() {
    let value: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&connected_database().await)
        .await
        .unwrap();
    assert_eq!(value, 1);
}

#[tokio::test]
async fn baseline_migration_applies_and_is_recorded() {
    let database = connected_database().await;
    sqlx::migrate!().run(&database).await.unwrap();
    let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(&database)
        .await
        .unwrap();
    assert!(applied >= 1);
}

#[tokio::test]
async fn openapi_documents_every_implemented_operation() {
    use utoipa::OpenApi;
    let spec = serde_json::to_value(pagerune::ApiDoc::openapi()).unwrap();
    assert!(spec["paths"]["/healthz"]["get"].is_object());
    assert!(spec["paths"]["/readyz"]["get"]["responses"]["503"].is_object());
    let response = response("/api/openapi.json", connected_database().await).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn unknown_routes_use_consistent_error_shape() {
    let response = response("/missing", connected_database().await).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["code"], "not_found");
    assert!(value["request_id"].as_str().is_some());
}
