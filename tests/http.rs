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
    assert!(html.contains(&url.replace('/', "&#x2f;")), "{html}");
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

    sqlx::query("UPDATE monitored_sites SET public_subscriptions_enabled=false WHERE id=$1")
        .bind(site_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        response(&format!("/subscribe/{site_id}"), pool.clone())
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM subscriptions WHERE id=$1 AND status='active'"
        )
        .bind(subscription_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1,
        "disabling new enrollment preserves the existing subscriber"
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
    assert!(managed_html.contains(&url.replace('/', "&#x2f;")));
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
async fn management_landing_is_separate_from_public_subscriber_management() {
    let pool = connected_database().await;
    let management_response = response("/management", pool.clone()).await;
    assert_eq!(management_response.status(), StatusCode::OK);
    let csp = management_response
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let body = axum::body::to_bytes(management_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("Registered websites"));
    assert!(html.contains("Register a website"));
    assert!(html.contains("/oauth2/sign_out"));
    assert!(csp.contains("script-src 'self' https://challenges.cloudflare.com"));

    let script = response("/static/management.js", pool.clone()).await;
    assert_eq!(script.status(), StatusCode::OK);
    assert_eq!(
        script.headers().get("content-type").unwrap(),
        "text/javascript; charset=utf-8"
    );
    let script_body = axum::body::to_bytes(script.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&script_body).contains("aria-expanded"));

    let subscriber_management = response("/manage", pool).await;
    assert_eq!(subscriber_management.status(), StatusCode::OK);
}

#[tokio::test]
async fn disabled_subscription_link_does_not_accept_a_new_subscription() {
    let pool = connected_database().await;
    let site_id = Uuid::new_v4();
    let source_url = format!("https://disabled-enrollment.example/{site_id}");
    sqlx::query("INSERT INTO monitored_sites(id,original_url,normalized_url,public_subscriptions_enabled) VALUES($1,$2,$2,false)")
        .bind(site_id)
        .bind(source_url)
        .execute(&pool)
        .await
        .unwrap();

    let csrf = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let email = format!("{}@example.test", Uuid::new_v4().simple());
    let csrf_hash = hex::encode(Sha256::digest(csrf.as_bytes()));
    sqlx::query("INSERT INTO web_form_nonces(token_hash,purpose,site_id,expires_at) VALUES($1,'subscribe',$2,now()+interval '20 minutes')")
        .bind(csrf_hash)
        .bind(site_id)
        .execute(&pool)
        .await
        .unwrap();
    let body = form_body(&[
        ("email", &email),
        ("csrf", &csrf),
        ("cf-turnstile-response", "dev-human-ok"),
    ]);
    let result = form_request(
        Method::POST,
        &format!("/subscribe/{site_id}"),
        pool.clone(),
        &body,
        None,
    )
    .await;
    assert_eq!(result.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM subscriptions WHERE site_id=$1")
            .bind(site_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn management_ui_creates_edits_and_removes_site_with_csrf_and_cascades() {
    let pool = connected_database().await;
    let listing = response("/management", pool.clone()).await;
    let html = String::from_utf8(
        axum::body::to_bytes(listing.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let csrf = hidden_value(&html, "csrf");
    let bad = form_body(&[
        ("name", "Bad site"),
        ("url", "https://bad.example"),
        ("csrf", "wrong"),
    ]);
    let rejected = form_request(Method::POST, "/management", pool.clone(), &bad, None).await;
    assert_eq!(rejected.status(), StatusCode::OK);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM monitored_sites WHERE normalized_url='https://bad.example/'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    let invalid_url = form_body(&[
        ("name", "Invalid scheme"),
        ("url", "javascript:alert(1)"),
        ("csrf", &csrf),
    ]);
    let invalid = form_request(
        Method::POST,
        "/management",
        pool.clone(),
        &invalid_url,
        None,
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::OK);
    let invalid_html = String::from_utf8(
        axum::body::to_bytes(invalid.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(invalid_html.contains("valid HTTP or HTTPS URL"));

    let url = format!("https://management.example/{}", Uuid::new_v4());
    let create = form_body(&[("name", "Managed example"), ("url", &url), ("csrf", &csrf)]);
    let created = form_request(Method::POST, "/management", pool.clone(), &create, None).await;
    assert_eq!(created.status(), StatusCode::SEE_OTHER);
    let site_id: Uuid =
        sqlx::query_scalar("SELECT id FROM monitored_sites WHERE normalized_url=$1")
            .bind(&url)
            .fetch_one(&pool)
            .await
            .unwrap();
    let subscriptions_enabled: bool =
        sqlx::query_scalar("SELECT public_subscriptions_enabled FROM monitored_sites WHERE id=$1")
            .bind(site_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !subscriptions_enabled,
        "omitted registration checkbox defaults disabled"
    );
    let listing = response("/management", pool.clone()).await;
    let listing_html = String::from_utf8(
        axum::body::to_bytes(listing.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let create_csrf = hidden_value(&listing_html, "csrf");
    let enabled_url = format!("https://management.example/enabled/{}", Uuid::new_v4());
    let enabled_create = form_body(&[
        ("name", "Enabled at registration"),
        ("url", &enabled_url),
        ("public_subscriptions_enabled", "true"),
        ("csrf", &create_csrf),
    ]);
    let created_enabled = form_request(
        Method::POST,
        "/management",
        pool.clone(),
        &enabled_create,
        None,
    )
    .await;
    assert_eq!(created_enabled.status(), StatusCode::SEE_OTHER);
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT public_subscriptions_enabled FROM monitored_sites WHERE normalized_url=$1"
        )
        .bind(&enabled_url)
        .fetch_one(&pool)
        .await
        .unwrap()
    );
    let duplicate_page = response("/management", pool.clone()).await;
    let duplicate_html = String::from_utf8(
        axum::body::to_bytes(duplicate_page.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let duplicate_csrf = hidden_value(&duplicate_html, "csrf");
    let duplicate_url = format!("{url}#same-site");
    let duplicate_form = form_body(&[
        ("name", "Should not overwrite"),
        ("url", &duplicate_url),
        ("csrf", &duplicate_csrf),
    ]);
    let duplicate = form_request(
        Method::POST,
        "/management",
        pool.clone(),
        &duplicate_form,
        None,
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        duplicate.headers().get("location").unwrap(),
        "/management?notice=duplicate"
    );
    let stored_name: String = sqlx::query_scalar("SELECT name FROM monitored_sites WHERE id=$1")
        .bind(site_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored_name, "Managed example");
    let edit_page = response(&format!("/management/{site_id}/edit"), pool.clone()).await;
    let edit_html = String::from_utf8(
        axum::body::to_bytes(edit_page.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let edit_csrf = hidden_value(&edit_html, "csrf");
    let changed_url = format!("https://management.example/changed/{}", Uuid::new_v4());
    let edited_name = "Edited <script>alert(1)</script>";
    let forged_edit = form_body(&[
        ("name", "Forged update"),
        ("url", &url),
        ("interval_seconds", "7200"),
        ("public_subscriptions_enabled", "true"),
        ("csrf", "incorrect"),
    ]);
    assert_eq!(
        form_request(
            Method::POST,
            &format!("/management/{site_id}/edit"),
            pool.clone(),
            &forged_edit,
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert!(
        !sqlx::query_scalar::<_, bool>(
            "SELECT public_subscriptions_enabled FROM monitored_sites WHERE id=$1"
        )
        .bind(site_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        "invalid CSRF must not change subscription availability"
    );
    let edit = form_body(&[
        ("name", edited_name),
        ("url", &changed_url),
        ("interval_seconds", "7200"),
        ("monitoring_enabled", "true"),
        ("public_subscriptions_enabled", "true"),
        ("csrf", &edit_csrf),
    ]);
    let edited = form_request(
        Method::POST,
        &format!("/management/{site_id}/edit"),
        pool.clone(),
        &edit,
        None,
    )
    .await;
    assert_eq!(edited.status(), StatusCode::SEE_OTHER);
    let (name, hash, enabled, subscriptions_enabled, interval): (String, Option<String>, bool, bool, i32) = sqlx::query_as(
        "SELECT name,current_content_hash,monitoring_enabled,public_subscriptions_enabled,interval_seconds FROM monitored_sites WHERE id=$1",
    )
    .bind(site_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(name, edited_name);
    assert_eq!(hash, None);
    assert!(enabled);
    assert!(subscriptions_enabled);
    assert_eq!(interval, 7200);
    let pause_page = response(&format!("/management/{site_id}/edit"), pool.clone()).await;
    let pause_html = String::from_utf8(
        axum::body::to_bytes(pause_page.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let pause_csrf = hidden_value(&pause_html, "csrf");
    let pause = form_body(&[
        ("name", edited_name),
        ("url", &changed_url),
        ("interval_seconds", "7200"),
        ("csrf", &pause_csrf),
    ]);
    let paused = form_request(
        Method::POST,
        &format!("/management/{site_id}/edit"),
        pool.clone(),
        &pause,
        None,
    )
    .await;
    assert_eq!(paused.status(), StatusCode::SEE_OTHER);
    let monitoring_enabled: bool =
        sqlx::query_scalar("SELECT monitoring_enabled FROM monitored_sites WHERE id=$1")
            .bind(site_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!monitoring_enabled);
    let subscriptions_enabled: bool =
        sqlx::query_scalar("SELECT public_subscriptions_enabled FROM monitored_sites WHERE id=$1")
            .bind(site_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !subscriptions_enabled,
        "unchecked subscription checkbox disables enrollment independently"
    );

    let subscription_id = Uuid::new_v4();
    let token_id = Uuid::new_v4();
    sqlx::query("INSERT INTO subscriptions(id,site_id,subscriber_identity,status) VALUES($1,$2,$3,'active')").bind(subscription_id).bind(site_id).bind(format!("{}@example.test", Uuid::new_v4().simple())).execute(&pool).await.unwrap();
    let change_id = Uuid::new_v4();
    sqlx::query("INSERT INTO site_changes(id,site_id,previous_content_hash,new_content_hash) VALUES($1,$2,$3,$4)").bind(change_id).bind(site_id).bind("a".repeat(64)).bind("b".repeat(64)).execute(&pool).await.unwrap();
    sqlx::query("UPDATE site_changes SET detected_at='2026-01-02 03:04:00+00' WHERE id=$1")
        .bind(change_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE monitored_sites SET last_checked_at='2026-02-03 04:05:00+00' WHERE id=$1")
        .bind(site_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO notification_outbox(id,change_id,subscription_id,recipient) VALUES($1,$2,$3,'person@example.test')").bind(Uuid::new_v4()).bind(change_id).bind(subscription_id).execute(&pool).await.unwrap();
    let verification_hash = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query("INSERT INTO verification_tokens(token_id,token_hash,subscription_id,expires_at) VALUES($1,$2,$3,now()+interval '1 hour')").bind(token_id).bind(verification_hash).bind(subscription_id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO email_action_outbox(id,action,recipient,token_id) VALUES($1,'verify','person@example.test',$2)").bind(Uuid::new_v4()).bind(token_id).execute(&pool).await.unwrap();
    let listing = response("/management", pool.clone()).await;
    let listing_html = String::from_utf8(
        axum::body::to_bytes(listing.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(listing_html.contains("Edited &lt;script&gt;alert(1)&lt;&#x2f;script&gt;"));
    assert!(!listing_html.contains("Edited <script>alert(1)</script>"));
    assert!(listing_html.contains("Active subscribers"));
    assert!(listing_html.contains(">Paused</span>"));
    assert!(listing_html.contains(
        "data-label=\"Subscriptions\"><span class=\"status-pill status-off\">Disabled</span>"
    ));
    assert!(listing_html.contains(&format!(
        "http:&#x2f;&#x2f;localhost:8080&#x2f;subscribe&#x2f;{site_id}"
    )));
    assert!(listing_html.contains("Copy subscription link"));
    assert!(listing_html.contains("View subscription page"));
    assert!(listing_html.contains("Edit website"));
    assert!(listing_html.contains("Remove website"));
    assert!(listing_html.contains("3 Feb 2026, 04:05 UTC"));
    assert!(listing_html.contains("title=\"2026-02-03T04:05:00+00:00\""));
    assert!(listing_html.contains("2 Jan 2026, 03:04 UTC"));
    assert!(listing_html.contains("title=\"2026-01-02T03:04:00+00:00\""));
    let enable_page = response(&format!("/management/{site_id}/edit"), pool.clone()).await;
    let enable_html = String::from_utf8(
        axum::body::to_bytes(enable_page.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let enable_csrf = hidden_value(&enable_html, "csrf");
    let enable_subscriptions = form_body(&[
        ("name", edited_name),
        ("url", &changed_url),
        ("interval_seconds", "7200"),
        ("public_subscriptions_enabled", "true"),
        ("csrf", &enable_csrf),
    ]);
    let enabled = form_request(
        Method::POST,
        &format!("/management/{site_id}/edit"),
        pool.clone(),
        &enable_subscriptions,
        None,
    )
    .await;
    assert_eq!(enabled.status(), StatusCode::SEE_OTHER);
    let (monitoring_enabled, subscriptions_enabled): (bool, bool) = sqlx::query_as(
        "SELECT monitoring_enabled,public_subscriptions_enabled FROM monitored_sites WHERE id=$1",
    )
    .bind(site_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !monitoring_enabled && subscriptions_enabled,
        "enabling subscriptions must not resume monitoring"
    );
    let enabled_listing = response("/management", pool.clone()).await;
    let enabled_html = String::from_utf8(
        axum::body::to_bytes(enabled_listing.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(
        enabled_html
            .contains("data-label=\"Subscriptions\"><span class=\"status-pill \">Enabled</span>")
    );
    assert!(enabled_html.contains(&format!(
        "http:&#x2f;&#x2f;localhost:8080&#x2f;subscribe&#x2f;{site_id}"
    )));
    let disable_page = response(&format!("/management/{site_id}/edit"), pool.clone()).await;
    let disable_html = String::from_utf8(
        axum::body::to_bytes(disable_page.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let disable_csrf = hidden_value(&disable_html, "csrf");
    let disable_subscriptions = form_body(&[
        ("name", edited_name),
        ("url", &changed_url),
        ("interval_seconds", "7200"),
        ("csrf", &disable_csrf),
    ]);
    let disabled = form_request(
        Method::POST,
        &format!("/management/{site_id}/edit"),
        pool.clone(),
        &disable_subscriptions,
        None,
    )
    .await;
    assert_eq!(disabled.status(), StatusCode::SEE_OTHER);
    let delete_page = response(&format!("/management/{site_id}/delete"), pool.clone()).await;
    let delete_html = String::from_utf8(
        axum::body::to_bytes(delete_page.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let delete_csrf = hidden_value(&delete_html, "csrf");
    let delete = form_body(&[("csrf", &delete_csrf)]);
    let removed = form_request(
        Method::POST,
        &format!("/management/{site_id}/delete"),
        pool.clone(),
        &delete,
        None,
    )
    .await;
    assert_eq!(removed.status(), StatusCode::SEE_OTHER);
    for (table, col) in [
        ("subscriptions", "site_id"),
        ("site_changes", "site_id"),
        ("email_action_outbox", "token_id"),
    ] {
        let query = format!("SELECT count(*) FROM {table} WHERE {col}=$1");
        if col == "token_id" {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(&query)
                    .bind(token_id)
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                0
            );
        } else {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(&query)
                    .bind(site_id)
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                0
            );
        }
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM notification_outbox WHERE change_id=$1 OR subscription_id=$2"
        )
        .bind(change_id)
        .bind(subscription_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        response(&format!("/management/{site_id}/edit"), pool)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
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
    let unique_path = Uuid::new_v4();
    let original_url = format!("HTTP://Example.com:80/a/{unique_path}#one");
    let duplicate_url = format!("http://example.com/a/{unique_path}#two");
    let first = request(
        Method::POST,
        "/api/v1/sites",
        database.clone(),
        Some(serde_json::json!({"name":"Example", "url":original_url})),
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
        Some(serde_json::json!({"name":"Ignored duplicate", "url":duplicate_url,"interval_seconds":86400})),
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second_body = axum::body::to_bytes(second.into_body(), usize::MAX)
        .await
        .unwrap();
    let second: serde_json::Value = serde_json::from_slice(&second_body).unwrap();
    assert_eq!(first["id"], second["id"]);
    assert_eq!(
        first["normalized_url"],
        format!("http://example.com/a/{unique_path}")
    );
    assert_eq!(first["public_subscriptions_enabled"], false);
    assert!(
        first["subscription_url"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/subscribe/{}", first["id"].as_str().unwrap()))
    );
    let id = Uuid::parse_str(first["id"].as_str().unwrap()).unwrap();
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
        Some(serde_json::json!({"name":"Example updated", "interval_seconds":3600,"public_subscriptions_enabled":true,"monitoring_enabled":false})),
    )
    .await;
    assert_eq!(update.status(), StatusCode::OK);
    let body = axum::body::to_bytes(update.into_body(), usize::MAX)
        .await
        .unwrap();
    let updated: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(updated["interval_seconds"], 3600);
    assert_eq!(updated["public_subscriptions_enabled"], true);
    assert_eq!(updated["name"], "Example updated");
    assert_eq!(updated["monitoring_enabled"], false);
    sqlx::query("UPDATE monitored_sites SET current_content_hash=$2,last_checked_at=now(),last_successful_check_at=now() WHERE id=$1")
        .bind(id)
        .bind("a".repeat(64))
        .execute(&database)
        .await
        .unwrap();
    let replacement_url = format!("https://example.com/revised/{}", Uuid::new_v4());
    let changed_url = request(
        Method::PATCH,
        &format!("/api/v1/sites/{id}"),
        database.clone(),
        Some(serde_json::json!({"url":replacement_url,"monitoring_enabled":true})),
    )
    .await;
    assert_eq!(changed_url.status(), StatusCode::OK);
    let changed: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(changed_url.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(changed["id"], id.to_string());
    assert_eq!(changed["current_content_hash"], serde_json::Value::Null);
    assert_eq!(changed["last_checked_at"], serde_json::Value::Null);
    assert_eq!(changed["last_successful_check_at"], serde_json::Value::Null);
    let other_url = format!("https://example.com/other/{}", Uuid::new_v4());
    let other = request(
        Method::POST,
        "/api/v1/sites",
        database.clone(),
        Some(serde_json::json!({"name":"Other", "url":other_url})),
    )
    .await;
    let other: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(other.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let conflict = request(
        Method::PATCH,
        &format!("/api/v1/sites/{id}"),
        database.clone(),
        Some(serde_json::json!({"url":other["original_url"]})),
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let other_id = other["id"].as_str().unwrap();
    assert_eq!(
        request(
            Method::DELETE,
            &format!("/api/v1/sites/{other_id}"),
            database.clone(),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
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
        Some(serde_json::json!({"name":"Create test", "url":format!("https://example.com/{}", Uuid::new_v4())})),
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
        Some(serde_json::json!({"name":"History test", "url":format!("https://example.com/history/{}", Uuid::new_v4())})),
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
