use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use hmac::{Hmac, Mac};
use pagerune::{AppState, app};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tower::ServiceExt;
use uuid::Uuid;

fn test_state(database: PgPool, dev_api_enabled: bool) -> AppState {
    AppState {
        database,
        dev_api_enabled,
        public_base_url: "http://localhost:8080".into(),
        token_key: [9; 32],
        human_verifier: Arc::new(pagerune::web::DevelopmentTestVerifier),
    }
}

static MIGRATIONS: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
static TEST_IP: AtomicUsize = AtomicUsize::new(1);

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
    app(test_state(database, true))
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn unsubscribe_get_is_read_only_and_post_is_idempotent() {
    let pool = connected_database().await;
    let site = Uuid::new_v4();
    let subscription = Uuid::new_v4();
    let sibling_subscription = Uuid::new_v4();
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let hash = hex::encode(Sha256::digest(token.as_bytes()));
    sqlx::query("INSERT INTO monitored_sites(id,original_url,normalized_url) VALUES($1,$2,$2)")
        .bind(site)
        .bind(format!("https://unsubscribe.example/{site}"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO subscriptions(id,site_id,subscriber_identity) VALUES($1,$2,'private@example.test')").bind(subscription).bind(site).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO subscriptions(id,site_id,subscriber_identity) VALUES($1,$2,'other@example.test')").bind(sibling_subscription).bind(site).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO unsubscribe_tokens(token_hash,subscription_id) VALUES($1,$2)")
        .bind(hash)
        .bind(subscription)
        .execute(&pool)
        .await
        .unwrap();
    let uri = format!("/unsubscribe/{token}");
    assert_eq!(
        response("/unsubscribe/not-a-token", pool.clone())
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let get = response(&uri, pool.clone()).await;
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM subscriptions WHERE id=$1")
            .bind(subscription)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    for _ in 0..2 {
        let result = app(test_state(pool.clone(), false))
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(&uri)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("List-Unsubscribe=One-Click"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(result.status(), StatusCode::OK);
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM subscriptions WHERE id=$1")
            .bind(subscription)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM subscriptions WHERE id=$1")
            .bind(sibling_subscription)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    sqlx::query("DELETE FROM monitored_sites WHERE id=$1")
        .bind(site)
        .execute(&pool)
        .await
        .unwrap();
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
    app(test_state(database, true))
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap()
}

fn form_body(fields: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in fields {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

fn hidden_value(html: &str, name: &str) -> String {
    let marker = format!("name=\"{name}\" value=\"");
    let start = html.find(&marker).unwrap() + marker.len();
    let end = start + html[start..].find('"').unwrap();
    html[start..end].to_owned()
}

fn derived_test_token(purpose: &str, id: Uuid) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(&[9u8; 32]).unwrap();
    mac.update(purpose.as_bytes());
    mac.update(id.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

async fn form_request(
    method: Method,
    uri: &str,
    pool: PgPool,
    body: &str,
    cookie: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    let mut request = builder.body(Body::from(body.to_owned())).unwrap();
    let n = TEST_IP.fetch_add(1, Ordering::Relaxed);
    let ip = Ipv4Addr::new(127, ((n / 250) % 250) as u8, (n % 250) as u8, 1);
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(SocketAddr::new(
            IpAddr::V4(ip),
            45678,
        )));
    app(test_state(pool, true)).oneshot(request).await.unwrap()
}

#[tokio::test]
async fn public_subscription_verification_and_management_are_site_scoped() {
    let pool = connected_database().await;
    let site_id = Uuid::new_v4();
    let email = format!("{}@example.test", Uuid::new_v4().simple());
    let url = format!("https://public-enrollment.example/{site_id}");
    sqlx::query("INSERT INTO monitored_sites(id,original_url,normalized_url,public_subscriptions_enabled) VALUES($1,$2,$2,true)")
        .bind(site_id).bind(&url).execute(&pool).await.unwrap();

    let form = response(&format!("/subscribe/{site_id}"), pool.clone()).await;
    let html = String::from_utf8(
        axum::body::to_bytes(form.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains(&url), "{html}");
    let csrf = hidden_value(&html, "csrf");
    let body = form_body(&[
        ("email", &email),
        ("csrf", &csrf),
        ("cf-turnstile-response", "dev-human-ok"),
    ]);
    let submitted = form_request(
        Method::POST,
        &format!("/subscribe/{site_id}"),
        pool.clone(),
        &body,
        None,
    )
    .await;
    assert_eq!(submitted.status(), StatusCode::OK);
    let subscription_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM subscriptions WHERE site_id=$1 AND subscriber_identity=$2",
    )
    .bind(site_id)
    .bind(&email)
    .fetch_one(&pool)
    .await
    .unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM subscriptions WHERE id=$1")
        .bind(subscription_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "pending");
    let repeat_form = response(&format!("/subscribe/{site_id}"), pool.clone()).await;
    let repeat_html = String::from_utf8(
        axum::body::to_bytes(repeat_form.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let repeat_csrf = hidden_value(&repeat_html, "csrf");
    let repeat_body = form_body(&[
        ("email", &email),
        ("csrf", &repeat_csrf),
        ("cf-turnstile-response", "dev-human-ok"),
    ]);
    form_request(
        Method::POST,
        &format!("/subscribe/{site_id}"),
        pool.clone(),
        &repeat_body,
        None,
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM subscriptions WHERE site_id=$1 AND subscriber_identity=$2"
        )
        .bind(site_id)
        .bind(&email)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM verification_tokens WHERE subscription_id=$1 AND revoked_at IS NULL AND consumed_at IS NULL").bind(subscription_id).fetch_one(&pool).await.unwrap(), 1);
    let verify_id: Uuid = sqlx::query_scalar(
        "SELECT token_id FROM verification_tokens WHERE subscription_id=$1 AND revoked_at IS NULL",
    )
    .bind(subscription_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let verify_token = derived_test_token("verify", verify_id);
    let verify_path = format!("/verify/{verify_token}");
    let get = response(&verify_path, pool.clone()).await;
    let get_html = String::from_utf8(
        axum::body::to_bytes(get.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM subscriptions WHERE id=$1")
            .bind(subscription_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "pending"
    );
    let verify_csrf = hidden_value(&get_html, "csrf");
    let verify_body = form_body(&[("csrf", &verify_csrf)]);
    let wrong_csrf = form_body(&[("csrf", "incorrect")]);
    assert_eq!(
        form_request(Method::POST, &verify_path, pool.clone(), &wrong_csrf, None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM subscriptions WHERE id=$1")
            .bind(subscription_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "pending"
    );
    assert_eq!(
        form_request(Method::POST, &verify_path, pool.clone(), &verify_body, None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM subscriptions WHERE id=$1")
            .bind(subscription_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "active"
    );
    assert_eq!(
        form_request(Method::POST, &verify_path, pool.clone(), &verify_body, None)
            .await
            .status(),
        StatusCode::OK
    );

    let manage = response("/manage", pool.clone()).await;
    let manage_html = String::from_utf8(
        axum::body::to_bytes(manage.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let manage_csrf = hidden_value(&manage_html, "csrf");
    let manage_body = form_body(&[
        ("email", &email),
        ("csrf", &manage_csrf),
        ("cf-turnstile-response", "dev-human-ok"),
    ]);
    form_request(Method::POST, "/manage", pool.clone(), &manage_body, None).await;
    let management_id: Uuid = sqlx::query_scalar("SELECT token_id FROM management_tokens WHERE subscriber_identity=$1 ORDER BY created_at DESC LIMIT 1").bind(&email).fetch_one(&pool).await.unwrap();
    let management_token = derived_test_token("manage", management_id);
    let exchange_path = format!("/manage/{management_token}");
    let exchange = response(&exchange_path, pool.clone()).await;
    let exchange_html = String::from_utf8(
        axum::body::to_bytes(exchange.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let exchange_csrf = hidden_value(&exchange_html, "csrf");
    let exchange_body = form_body(&[("csrf", &exchange_csrf)]);
    let exchanged = form_request(
        Method::POST,
        &exchange_path,
        pool.clone(),
        &exchange_body,
        None,
    )
    .await;
    assert_eq!(exchanged.status(), StatusCode::SEE_OTHER);
    let cookie = exchanged
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let request = Request::builder()
        .uri("/manage")
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();
    let managed = app(test_state(pool.clone(), true))
        .oneshot(request)
        .await
        .unwrap();
    let managed_html = String::from_utf8(
        axum::body::to_bytes(managed.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(managed_html.contains(&url));
    let session_csrf = hidden_value(&managed_html, "csrf");
    let unsubscribe_body = form_body(&[("csrf", &session_csrf)]);
    let removed = form_request(
        Method::POST,
        &format!("/manage/unsubscribe/{subscription_id}"),
        pool.clone(),
        &unsubscribe_body,
        Some(&cookie),
    )
    .await;
    assert_eq!(removed.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM subscriptions WHERE id=$1")
            .bind(subscription_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DELETE FROM monitored_sites WHERE id=$1")
        .bind(site_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn expired_verification_token_never_activates_subscription() {
    let pool = connected_database().await;
    let site_id = Uuid::new_v4();
    let subscription_id = Uuid::new_v4();
    let url = format!("https://expired-verification.example/{site_id}");
    sqlx::query("INSERT INTO monitored_sites(id,original_url,normalized_url) VALUES($1,$2,$2)")
        .bind(site_id)
        .bind(&url)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO subscriptions(id,site_id,subscriber_identity,status) VALUES($1,$2,'expired@example.test','pending')")
        .bind(subscription_id).bind(site_id).execute(&pool).await.unwrap();
    let token_id = Uuid::new_v4();
    let token = derived_test_token("verify", token_id);
    let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
    sqlx::query("INSERT INTO verification_tokens(token_id,token_hash,subscription_id,expires_at) VALUES($1,$2,$3,now()-interval '1 second')")
        .bind(token_id).bind(token_hash).bind(subscription_id).execute(&pool).await.unwrap();
    let response = response(&format!("/verify/{token}"), pool.clone()).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM subscriptions WHERE id=$1")
            .bind(subscription_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "pending"
    );
    sqlx::query("DELETE FROM monitored_sites WHERE id=$1")
        .bind(site_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn concurrent_public_requests_create_one_pending_subscription() {
    let pool = connected_database().await;
    let site_id = Uuid::new_v4();
    let email = format!("{}@example.test", Uuid::new_v4().simple());
    let url = format!("https://concurrent-enrollment.example/{site_id}");
    sqlx::query("INSERT INTO monitored_sites(id,original_url,normalized_url,public_subscriptions_enabled) VALUES($1,$2,$2,true)")
        .bind(site_id).bind(&url).execute(&pool).await.unwrap();
    let a = response(&format!("/subscribe/{site_id}"), pool.clone()).await;
    let a_html = String::from_utf8(
        axum::body::to_bytes(a.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let b = response(&format!("/subscribe/{site_id}"), pool.clone()).await;
    let b_html = String::from_utf8(
        axum::body::to_bytes(b.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let csrf_a = hidden_value(&a_html, "csrf");
    let csrf_b = hidden_value(&b_html, "csrf");
    let body_a = form_body(&[
        ("email", &email),
        ("csrf", &csrf_a),
        ("cf-turnstile-response", "dev-human-ok"),
    ]);
    let subscribe_path = format!("/subscribe/{site_id}");
    let body_b = form_body(&[
        ("email", &email),
        ("csrf", &csrf_b),
        ("cf-turnstile-response", "dev-human-ok"),
    ]);
    let (left, right) = tokio::join!(
        form_request(Method::POST, &subscribe_path, pool.clone(), &body_a, None),
        form_request(Method::POST, &subscribe_path, pool.clone(), &body_b, None),
    );
    assert_eq!(left.status(), StatusCode::OK);
    assert_eq!(right.status(), StatusCode::OK);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM subscriptions WHERE site_id=$1 AND lower(subscriber_identity)=$2"
        )
        .bind(site_id)
        .bind(&email)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM verification_tokens v JOIN subscriptions s ON s.id=v.subscription_id WHERE s.site_id=$1 AND lower(s.subscriber_identity)=$2 AND v.revoked_at IS NULL AND v.consumed_at IS NULL").bind(site_id).bind(&email).fetch_one(&pool).await.unwrap(), 1);
    sqlx::query("DELETE FROM monitored_sites WHERE id=$1")
        .bind(site_id)
        .execute(&pool)
        .await
        .unwrap();
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
    assert!(spec["paths"]["/subscribe/{site_id}"]["post"].is_object());
    assert!(spec["paths"]["/verify/{token}"]["post"].is_object());
    assert!(spec["paths"]["/manage/{token}"]["post"].is_object());
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
    let response = app(test_state(database, false))
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
    assert_eq!(first["public_subscriptions_enabled"], false);
    assert!(
        first["subscription_url"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/subscribe/{}", first["id"].as_str().unwrap()))
    );
    let id = first["id"].as_str().unwrap();
    let disabled_page = response(&format!("/subscribe/{id}"), database.clone()).await;
    assert_eq!(disabled_page.status(), StatusCode::NOT_FOUND);
    let disabled_html = String::from_utf8(
        axum::body::to_bytes(disabled_page.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(disabled_html.contains("subscription page is unavailable"));
    let update = request(
        Method::PATCH,
        &format!("/api/v1/sites/{id}"),
        database.clone(),
        Some(serde_json::json!({"interval_seconds":3600,"public_subscriptions_enabled":true})),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);
    let body = axum::body::to_bytes(update.into_body(), usize::MAX)
        .await
        .unwrap();
    let updated: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(updated["interval_seconds"], 3600);
    assert_eq!(updated["public_subscriptions_enabled"], true);
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
        Some(serde_json::json!({"subscriber_identity":"alice@example.test"})),
    )
    .await;
    assert_eq!(created.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(created.into_body(), usize::MAX)
        .await
        .unwrap();
    let sub: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(sub["status"], "pending");
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
        "/api/v1/subscriptions?subscriber_identity=alice%40example.test&page=1&limit=10",
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
        &format!("/api/v1/subscriptions/{sub_id}?subscriber_identity=alice%40example.test"),
        database.clone(),
        None,
    )
    .await;
    assert_eq!(found.status(), StatusCode::OK);
    let deleted = request(
        Method::DELETE,
        &format!("/api/v1/subscriptions/{sub_id}?subscriber_identity=alice%40example.test"),
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
    let response = app(test_state(database, true))
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
