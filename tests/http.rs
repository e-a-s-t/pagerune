use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use pagerune::{AppState, app};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

static MIGRATIONS: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

fn database_url() -> String {
    env::var("DATABASE_URL")
        .expect("DATABASE_URL is required; start Compose PostgreSQL and run tests through direnv")
}

async fn connected_database() -> PgPool {
    let database = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&database_url())
        .await
        .expect("PostgreSQL must be reachable for integration tests");
    MIGRATIONS
        .get_or_init(|| async {
            sqlx::migrate!()
                .run(&database)
                .await
                .expect("migrations must apply");
        })
        .await;
    database
}

async fn response(uri: &str, database: PgPool) -> axum::response::Response {
    app(AppState {
        database,
        dev_api_enabled: true,
    })
    .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
    .await
    .unwrap()
}

async fn request(
    method: Method,
    uri: &str,
    database: PgPool,
    json: Option<serde_json::Value>,
) -> axum::response::Response {
    let has_json = json.is_some();
    let body = json
        .map(|value| Body::from(value.to_string()))
        .unwrap_or_else(Body::empty);
    let mut builder = Request::builder().method(method).uri(uri);
    if has_json {
        builder = builder.header("content-type", "application/json");
    }
    app(AppState {
        database,
        dev_api_enabled: true,
    })
    .oneshot(builder.body(body).unwrap())
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
    assert!(spec["paths"]["/api/v1/sites"]["get"].is_object());
    assert!(spec["paths"]["/api/v1/sites/{id}"]["patch"].is_object());
    assert!(spec["paths"]["/api/v1/subscriptions/{id}"]["delete"].is_object());
    assert!(spec["paths"]["/api/v1/changes/{id}"]["get"].is_object());
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

#[tokio::test]
async fn development_api_is_disabled_unless_explicitly_enabled() {
    let database = connected_database().await;
    let response = app(AppState {
        database,
        dev_api_enabled: false,
    })
    .oneshot(
        Request::builder()
            .uri("/api/v1/sites")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["code"], "api_disabled");
    assert!(value["request_id"].as_str().is_some());
}

#[tokio::test]
async fn normalized_duplicate_site_registration_returns_same_site() {
    let database = connected_database().await;
    let first = request(
        Method::POST,
        "/api/v1/sites",
        database.clone(),
        Some(serde_json::json!({"url":"HTTP://Example.com:80/a#one"})),
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let first_body = axum::body::to_bytes(first.into_body(), usize::MAX)
        .await
        .unwrap();
    let first: serde_json::Value = serde_json::from_slice(&first_body).unwrap();
    let second = request(
        Method::POST,
        "/api/v1/sites",
        database.clone(),
        Some(serde_json::json!({"url":"http://example.com/a#two","interval_seconds":86400})),
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second_body = axum::body::to_bytes(second.into_body(), usize::MAX)
        .await
        .unwrap();
    let second: serde_json::Value = serde_json::from_slice(&second_body).unwrap();
    assert_eq!(first["id"], second["id"]);
    assert_eq!(first["normalized_url"], "http://example.com/a");
    let id = first["id"].as_str().unwrap();
    let update = request(
        Method::PATCH,
        &format!("/api/v1/sites/{id}"),
        database.clone(),
        Some(serde_json::json!({"interval_seconds":3600})),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);
    let body = axum::body::to_bytes(update.into_body(), usize::MAX)
        .await
        .unwrap();
    let updated: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(updated["interval_seconds"], 3600);
    let invalid = request(
        Method::PATCH,
        &format!("/api/v1/sites/{id}"),
        database.clone(),
        Some(serde_json::json!({"interval_seconds":299})),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let cleanup = request(
        Method::DELETE,
        &format!("/api/v1/sites/{id}"),
        database,
        None,
    )
    .await;
    assert_eq!(cleanup.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn subscription_operations_are_scoped_to_subscriber_identity() {
    let database = connected_database().await;
    let created = request(
        Method::POST,
        "/api/v1/sites",
        database.clone(),
        Some(serde_json::json!({"url":format!("https://example.com/{}", Uuid::new_v4())})),
    )
    .await;
    let body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let site: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let site_id = site["id"].as_str().unwrap();
    let create_path = format!("/api/v1/sites/{site_id}/subscriptions");
    let created = request(
        Method::POST,
        &create_path,
        database.clone(),
        Some(serde_json::json!({"subscriber_identity":"dev:alice"})),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let sub: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let sub_id = sub["id"].as_str().unwrap();
    let wrong_identity = request(
        Method::GET,
        &format!("/api/v1/subscriptions/{sub_id}?subscriber_identity=dev%3Abob"),
        database.clone(),
        None,
    )
    .await;
    assert_eq!(wrong_identity.status(), StatusCode::NOT_FOUND);
    let list = response(
        "/api/v1/subscriptions?subscriber_identity=dev%3Aalice&page=1&limit=10",
        database.clone(),
    )
    .await;
    assert_eq!(list.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list.into_body(), usize::MAX)
        .await
        .unwrap();
    let listed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(listed["items"][0]["id"], sub_id);
    let found = request(
        Method::GET,
        &format!("/api/v1/subscriptions/{sub_id}?subscriber_identity=dev%3Aalice"),
        database.clone(),
        None,
    )
    .await;
    assert_eq!(found.status(), StatusCode::OK);
    let deleted = request(
        Method::DELETE,
        &format!("/api/v1/subscriptions/{sub_id}?subscriber_identity=dev%3Aalice"),
        database.clone(),
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let cleanup = request(
        Method::DELETE,
        &format!("/api/v1/sites/{site_id}"),
        database,
        None,
    )
    .await;
    assert_eq!(cleanup.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn site_change_history_has_stable_individual_resource() {
    let database = connected_database().await;
    let site_response = request(
        Method::POST,
        "/api/v1/sites",
        database.clone(),
        Some(serde_json::json!({"url":format!("https://example.com/history/{}", Uuid::new_v4())})),
    )
    .await;
    let body = axum::body::to_bytes(site_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let site: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let site_id: Uuid = site["id"].as_str().unwrap().parse().unwrap();
    let change_id = Uuid::new_v4();
    sqlx::query("INSERT INTO site_changes (id, site_id, previous_content_hash, new_content_hash) VALUES ($1,$2,$3,$4)")
        .bind(change_id).bind(site_id).bind("a".repeat(64)).bind("b".repeat(64)).execute(&database).await.unwrap();
    let details = response(&format!("/api/v1/changes/{change_id}"), database.clone()).await;
    assert_eq!(details.status(), StatusCode::OK);
    assert!(details.headers().get("x-request-id").is_some());
    let body = axum::body::to_bytes(details.into_body(), usize::MAX)
        .await
        .unwrap();
    let change: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(change["id"], change_id.to_string());
    assert_eq!(change["site_id"], site_id.to_string());
    let history = response(
        &format!("/api/v1/sites/{site_id}/changes?page=1&limit=10"),
        database.clone(),
    )
    .await;
    assert_eq!(history.status(), StatusCode::OK);
    let body = axum::body::to_bytes(history.into_body(), usize::MAX)
        .await
        .unwrap();
    let history: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(history["items"][0]["id"], change_id.to_string());
    let _ = request(
        Method::DELETE,
        &format!("/api/v1/sites/{site_id}"),
        database,
        None,
    )
    .await;
}

#[tokio::test]
async fn malformed_json_uses_consistent_error_shape() {
    let database = connected_database().await;
    let response = app(AppState {
        database,
        dev_api_enabled: true,
    })
    .oneshot(
        Request::builder()
            .method(Method::POST)
            .uri("/api/v1/sites")
            .header("content-type", "application/json")
            .body(Body::from("{"))
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["code"], "invalid_request");
    assert!(value["request_id"].as_str().is_some());
}
