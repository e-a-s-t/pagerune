use crate::{AppState, fetcher::normalize_url, notifications::SmtpConfig};
use axum::{
    Form, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use hmac::{Hmac, Mac};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::Mailbox,
    transport::smtp::{authentication::Credentials, client::Tls},
};
use minijinja::{Environment, context};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Row};
use std::{future::Future, net::SocketAddr, pin::Pin, time::Duration};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const VERIFY_TTL_HOURS: i64 = 24;
const MANAGE_TOKEN_TTL_MINUTES: i64 = 15;
const SESSION_TTL_MINUTES: i64 = 30;

pub trait HumanVerifier: Send + Sync {
    fn site_key(&self) -> &str;
    fn is_development(&self) -> bool {
        false
    }
    fn verify<'a>(
        &'a self,
        response: &'a str,
        action: &'a str,
        hostname: &'a str,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>>;
}

pub struct TurnstileVerifier {
    pub site_key: String,
    pub secret: String,
}

impl HumanVerifier for TurnstileVerifier {
    fn site_key(&self) -> &str {
        &self.site_key
    }
    fn verify<'a>(
        &'a self,
        response: &'a str,
        action: &'a str,
        hostname: &'a str,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move { validate_turnstile(&self.secret, response, action, hostname).await })
    }
}

pub struct DevelopmentTestVerifier;

impl HumanVerifier for DevelopmentTestVerifier {
    fn site_key(&self) -> &str {
        ""
    }
    fn is_development(&self) -> bool {
        true
    }
    fn verify<'a>(
        &'a self,
        response: &'a str,
        _action: &'a str,
        _hostname: &'a str,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move { response == "dev-human-ok" })
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SubscribeForm {
    email: String,
    csrf: String,
    #[serde(rename = "cf-turnstile-response")]
    challenge: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ManageForm {
    email: String,
    csrf: String,
    #[serde(rename = "cf-turnstile-response")]
    challenge: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct NonceForm {
    csrf: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UnsubscribeForm {
    csrf: String,
}

#[derive(Deserialize)]
struct ManagementCreateForm {
    name: String,
    url: String,
    public_subscriptions_enabled: Option<String>,
    csrf: String,
}
#[derive(Deserialize)]
struct ManagementEditForm {
    name: String,
    url: String,
    interval_seconds: String,
    monitoring_enabled: Option<String>,
    public_subscriptions_enabled: Option<String>,
    csrf: String,
}
#[derive(Deserialize)]
struct ManagementDeleteForm {
    csrf: String,
}
#[derive(Deserialize)]
struct ManagementQuery {
    page: Option<i64>,
    notice: Option<String>,
}
#[derive(Serialize, FromRow)]
struct ManagementSite {
    id: Uuid,
    name: String,
    original_url: String,
    current_status: String,
    last_checked_at: Option<chrono::DateTime<chrono::Utc>>,
    last_detected_change_at: Option<chrono::DateTime<chrono::Utc>>,
    active_subscriber_count: i64,
    monitoring_enabled: bool,
    interval_seconds: i32,
    last_error: Option<String>,
    public_subscriptions_enabled: bool,
}

#[derive(Serialize, FromRow)]
struct ManagedSubscription {
    id: Uuid,
    site_url: String,
    interval_text: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(FromRow)]
struct ActionEmail {
    id: Uuid,
    action: String,
    recipient: String,
    token_id: Uuid,
    lease_owner: Uuid,
    lease_generation: i64,
    attempt_count: i32,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/management", get(management).post(management_create))
        .route("/management/", get(management))
        .route(
            "/management/{id}/edit",
            get(management_edit_page).post(management_edit),
        )
        .route(
            "/management/{id}/delete",
            get(management_delete_page).post(management_delete),
        )
        .route("/", get(index))
        .route("/subscribe/{site_id}", get(subscribe_page).post(subscribe))
        .route("/verify/{token}", get(verify_page).post(verify))
        .route("/manage", get(manage).post(request_management))
        .route(
            "/manage/{token}",
            get(management_exchange_page).post(exchange_management_token),
        )
        .route(
            "/manage/unsubscribe/{id}",
            axum::routing::post(management_unsubscribe),
        )
        .route("/static/pagerune.css", get(stylesheet))
        .route("/static/management.js", get(management_script))
        .layer(DefaultBodyLimit::max(16 * 1024))
}

fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn fresh_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn format_interval(seconds: i32) -> String {
    let seconds = seconds.max(1);
    let (value, unit) = if seconds % 86_400 == 0 {
        (seconds / 86_400, "day")
    } else if seconds % 3_600 == 0 {
        (seconds / 3_600, "hour")
    } else if seconds % 60 == 0 {
        (seconds / 60, "minute")
    } else {
        return format!("Every {seconds} seconds");
    };
    if value == 1 {
        format!("Every {unit}")
    } else {
        format!("Every {value} {unit}s")
    }
}

fn derived_token(key: &[u8; 32], purpose: &str, id: Uuid) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(purpose.as_bytes());
    mac.update(id.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn render(template: &str, values: minijinja::Value) -> Response {
    let source = match template {
        "base.html" => include_str!("../templates/web/base.html"),
        "index.html" => include_str!("../templates/web/index.html"),
        "subscribe.html" => include_str!("../templates/web/subscribe.html"),
        "confirmation.html" => include_str!("../templates/web/confirmation.html"),
        "verify.html" => include_str!("../templates/web/verify.html"),
        "manage.html" => include_str!("../templates/web/manage.html"),
        "manage_link.html" => include_str!("../templates/web/manage_link.html"),
        "management.html" => include_str!("../templates/web/management.html"),
        "management_edit.html" => include_str!("../templates/web/management_edit.html"),
        "management_delete.html" => include_str!("../templates/web/management_delete.html"),
        "error.html" => include_str!("../templates/web/error.html"),
        _ => include_str!("../templates/web/error.html"),
    };
    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::Html);
    env.add_template("base.html", include_str!("../templates/web/base.html"))
        .expect("bundled base template parses");
    env.add_template("page", source)
        .expect("bundled template parses");
    match env.get_template("page").and_then(|t| t.render(values)) {
        Ok(body) => Html(body).into_response(),
        Err(error) => {
            tracing::error!(template, error = %error, "web template rendering failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Request could not be completed",
            )
                .into_response()
        }
    }
}

fn page(template: &str, values: minijinja::Value) -> Response {
    let mut response = render(template, values);
    response.headers_mut().insert("content-security-policy", HeaderValue::from_static("default-src 'self'; style-src 'self'; script-src 'self' https://challenges.cloudflare.com; frame-src https://challenges.cloudflare.com; form-action 'self'; base-uri 'none'; object-src 'none'"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn token_response(template: &str, values: minijinja::Value) -> Response {
    let mut response = page(template, values);
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}

async fn management(
    State(state): State<AppState>,
    Query(query): Query<ManagementQuery>,
) -> Response {
    let page_number = query.page.unwrap_or(1).max(1);
    let notice = match query.notice.as_deref() {
        Some("created") => "Website registered successfully.",
        Some("updated") => "Website updated successfully.",
        Some("deleted") => "Website removed.",
        Some("duplicate") => "That URL is already registered.",
        _ => "",
    };
    management_page(&state, page_number, notice, "", "", "", false).await
}

async fn management_page(
    state: &AppState,
    page_number: i64,
    notice: &str,
    error: &str,
    form_name: &str,
    form_url: &str,
    form_public_subscriptions_enabled: bool,
) -> Response {
    let csrf = match nonce(&state.database, "management_create", None).await {
        Ok(value) => value,
        Err(_) => return generic_error(),
    };
    let limit = 25_i64;
    let total: i64 = match sqlx::query_scalar("SELECT count(*) FROM monitored_sites")
        .fetch_one(&state.database)
        .await
    {
        Ok(total) => total,
        Err(_) => return generic_error(),
    };
    let sites = match sqlx::query_as::<_, ManagementSite>("SELECT ms.id, ms.name, ms.original_url, ms.current_status, ms.last_checked_at, (SELECT max(sc.detected_at) FROM site_changes sc WHERE sc.site_id=ms.id) AS last_detected_change_at, (SELECT count(*) FROM subscriptions s WHERE s.site_id=ms.id AND s.status='active') AS active_subscriber_count, ms.monitoring_enabled, ms.interval_seconds, ms.last_error, ms.public_subscriptions_enabled FROM monitored_sites ms ORDER BY ms.created_at DESC, ms.id LIMIT $1 OFFSET $2")
        .bind(limit).bind((page_number - 1) * limit).fetch_all(&state.database).await { Ok(sites) => sites, Err(_) => return generic_error() };
    let sites: Vec<_> = sites.into_iter().map(|site| serde_json::json!({
        "id": site.id.to_string(), "name": site.name, "original_url": site.original_url,
        "subscription_url": subscription_url(&state.public_base_url, site.id),
        "public_subscriptions_enabled": site.public_subscriptions_enabled,
        "current_status": site.current_status, "monitoring_enabled": site.monitoring_enabled,
        "active_subscriber_count": site.active_subscriber_count,
        "last_error": site.last_error, "interval_seconds": site.interval_seconds,
        "interval_text": format_interval(site.interval_seconds),
        "last_checked_at": site.last_checked_at.map(|date| date.to_rfc3339()),
        "last_checked_at_display": site.last_checked_at.map(format_management_timestamp),
        "last_detected_change_at": site.last_detected_change_at.map(|date| date.to_rfc3339()),
        "last_detected_change_at_display": site.last_detected_change_at.map(format_management_timestamp),
    })).collect();
    let pages = ((total + limit - 1) / limit).max(1);
    admin_page(
        "management.html",
        context! { sites => sites, csrf => csrf, notice => notice, error => error, form_name => form_name, form_url => form_url, form_public_subscriptions_enabled => form_public_subscriptions_enabled, page => page_number, pages => pages, total => total },
    )
}

fn format_management_timestamp(date: chrono::DateTime<chrono::Utc>) -> String {
    date.format("%-d %b %Y, %H:%M UTC").to_string()
}

fn subscription_url(public_base_url: &str, site_id: Uuid) -> String {
    format!(
        "{}/subscribe/{site_id}",
        public_base_url.trim_end_matches('/')
    )
}

fn admin_page(template: &str, values: minijinja::Value) -> Response {
    let mut response = page(template, values);
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
}

async fn management_create(
    State(state): State<AppState>,
    Form(input): Form<ManagementCreateForm>,
) -> Response {
    let name = input.name.trim();
    let public_subscriptions_enabled =
        input.public_subscriptions_enabled.as_deref() == Some("true");
    if name.is_empty() || name.chars().count() > 200 || normalize_url(&input.url).is_err() {
        return management_create_error(
            &state,
            "Enter a name of 1 to 200 characters and a valid HTTP or HTTPS URL.",
            name,
            &input.url,
            public_subscriptions_enabled,
        )
        .await;
    }
    let mut tx = match state.database.begin().await {
        Ok(tx) => tx,
        Err(_) => return generic_error(),
    };
    if !consume_nonce(&mut tx, &input.csrf, "management_create", None).await {
        let _ = tx.rollback().await;
        return management_create_error(
            &state,
            "This form expired. Please try again.",
            name,
            &input.url,
            public_subscriptions_enabled,
        )
        .await;
    }
    if tx.commit().await.is_err() {
        return generic_error();
    }
    match crate::api::create_site(
        State(state.clone()),
        axum::Json(crate::api::CreateSite {
            name: name.to_owned(),
            url: input.url.trim().to_owned(),
            interval_seconds: 86_400,
            public_subscriptions_enabled,
        }),
    )
    .await
    {
        Ok((StatusCode::CREATED, _)) => Redirect::to("/management?notice=created").into_response(),
        Ok(_) => Redirect::to("/management?notice=duplicate").into_response(),
        Err(error) if error.1 == "duplicate_url" => {
            management_create_error(
                &state,
                "That URL is already registered.",
                name,
                &input.url,
                public_subscriptions_enabled,
            )
            .await
        }
        Err(_) => generic_error(),
    }
}

async fn management_create_error(
    state: &AppState,
    error: &str,
    name: &str,
    url: &str,
    public_subscriptions_enabled: bool,
) -> Response {
    management_page(state, 1, "", error, name, url, public_subscriptions_enabled).await
}

async fn management_edit_page(State(state): State<AppState>, Path(id): Path<Uuid>) -> Response {
    management_edit_render(&state, id, "", None, None, None, None).await
}

async fn management_edit_render(
    state: &AppState,
    id: Uuid,
    error: &str,
    form_name: Option<&str>,
    form_url: Option<&str>,
    form_interval: Option<&str>,
    form_public_subscriptions_enabled: Option<bool>,
) -> Response {
    let site = match sqlx::query_as::<_, ManagementSite>("SELECT ms.id, ms.name, ms.original_url, ms.current_status, ms.last_checked_at, (SELECT max(sc.detected_at) FROM site_changes sc WHERE sc.site_id=ms.id) AS last_detected_change_at, (SELECT count(*) FROM subscriptions s WHERE s.site_id=ms.id AND s.status='active') AS active_subscriber_count, ms.monitoring_enabled, ms.interval_seconds, ms.last_error, ms.public_subscriptions_enabled FROM monitored_sites ms WHERE ms.id=$1").bind(id).fetch_optional(&state.database).await {
        Ok(Some(site)) => site, Ok(None) => return not_found("Website not found."), Err(_) => return generic_error(),
    };
    let csrf = match nonce(&state.database, "management_edit", Some(id)).await {
        Ok(value) => value,
        Err(_) => return generic_error(),
    };
    let form_name = form_name.unwrap_or(&site.name).to_owned();
    let form_url = form_url.unwrap_or(&site.original_url).to_owned();
    let form_interval = form_interval
        .map(str::to_owned)
        .unwrap_or_else(|| site.interval_seconds.to_string());
    let form_public_subscriptions_enabled =
        form_public_subscriptions_enabled.unwrap_or(site.public_subscriptions_enabled);
    admin_page(
        "management_edit.html",
        context! { site => site, csrf => csrf, error => error, form_name => form_name, form_url => form_url, form_interval => form_interval, form_public_subscriptions_enabled => form_public_subscriptions_enabled },
    )
}

async fn management_edit(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(input): Form<ManagementEditForm>,
) -> Response {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 200 || normalize_url(&input.url).is_err() {
        return management_edit_render(
            &state,
            id,
            "Enter a name of 1 to 200 characters and a valid HTTP or HTTPS URL.",
            Some(&input.name),
            Some(&input.url),
            Some(&input.interval_seconds),
            Some(input.public_subscriptions_enabled.as_deref() == Some("true")),
        )
        .await;
    }
    let interval_seconds = match input.interval_seconds.parse::<i32>() {
        Ok(seconds) if (300..=2_592_000).contains(&seconds) => seconds,
        _ => {
            return management_edit_render(
                &state,
                id,
                "Check interval must be between 300 and 2,592,000 seconds.",
                Some(&input.name),
                Some(&input.url),
                Some(&input.interval_seconds),
                Some(input.public_subscriptions_enabled.as_deref() == Some("true")),
            )
            .await;
        }
    };
    let monitoring_enabled = input.monitoring_enabled.as_deref() == Some("true");
    let mut tx = match state.database.begin().await {
        Ok(tx) => tx,
        Err(_) => return generic_error(),
    };
    if !consume_nonce(&mut tx, &input.csrf, "management_edit", Some(id)).await {
        let _ = tx.rollback().await;
        return management_edit_render(
            &state,
            id,
            "This form expired. Please try again.",
            Some(&input.name),
            Some(&input.url),
            Some(&input.interval_seconds),
            Some(input.public_subscriptions_enabled.as_deref() == Some("true")),
        )
        .await;
    }
    if tx.commit().await.is_err() {
        return generic_error();
    }
    match crate::api::update_site(
        State(state.clone()),
        Path(id),
        axum::Json(crate::api::UpdateSite {
            name: Some(name.to_owned()),
            url: Some(input.url.trim().to_owned()),
            interval_seconds: Some(interval_seconds),
            public_subscriptions_enabled: Some(
                input.public_subscriptions_enabled.as_deref() == Some("true"),
            ),
            monitoring_enabled: Some(monitoring_enabled),
        }),
    )
    .await
    {
        Ok(_) => Redirect::to("/management?notice=updated").into_response(),
        Err(error) if error.1 == "duplicate_url" => {
            management_edit_render(
                &state,
                id,
                "That URL is already registered.",
                Some(&input.name),
                Some(&input.url),
                Some(&input.interval_seconds),
                Some(input.public_subscriptions_enabled.as_deref() == Some("true")),
            )
            .await
        }
        Err(error) if error.1 == "not_found" => not_found("Website not found."),
        Err(_) => generic_error(),
    }
}

async fn management_delete_page(State(state): State<AppState>, Path(id): Path<Uuid>) -> Response {
    let site = match sqlx::query_as::<_, (String, String)>(
        "SELECT name, original_url FROM monitored_sites WHERE id=$1",
    )
    .bind(id)
    .fetch_optional(&state.database)
    .await
    {
        Ok(Some(site)) => site,
        Ok(None) => return not_found("Website not found."),
        Err(_) => return generic_error(),
    };
    let csrf = match nonce(&state.database, "management_delete", Some(id)).await {
        Ok(value) => value,
        Err(_) => return generic_error(),
    };
    admin_page(
        "management_delete.html",
        context! { id => id.to_string(), name => site.0, url => site.1, csrf => csrf },
    )
}

async fn management_delete(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(input): Form<ManagementDeleteForm>,
) -> Response {
    let mut tx = match state.database.begin().await {
        Ok(tx) => tx,
        Err(_) => return generic_error(),
    };
    if !consume_nonce(&mut tx, &input.csrf, "management_delete", Some(id)).await {
        let _ = tx.rollback().await;
        return (
            StatusCode::BAD_REQUEST,
            "This confirmation expired. Please review the website again.",
        )
            .into_response();
    }
    if tx.commit().await.is_err() {
        return generic_error();
    }
    match crate::api::delete_site(State(state), Path(id)).await {
        Ok(_) => Redirect::to("/management?notice=deleted").into_response(),
        Err(error) => error.into_response(),
    }
}

#[utoipa::path(get, path = "/", tag = "Public web", responses((status = 200, description = "PageRune public landing page")))]
pub async fn index() -> Response {
    page("index.html", context! {})
}

async fn nonce(pool: &PgPool, purpose: &str, site_id: Option<Uuid>) -> Result<String, sqlx::Error> {
    let value = fresh_token();
    sqlx::query("INSERT INTO web_form_nonces(token_hash,purpose,site_id,expires_at) VALUES($1,$2,$3,now()+interval '20 minutes')")
        .bind(hash(&value)).bind(purpose).bind(site_id).execute(pool).await?;
    Ok(value)
}

async fn consume_nonce(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    value: &str,
    purpose: &str,
    site_id: Option<Uuid>,
) -> bool {
    sqlx::query("UPDATE web_form_nonces SET consumed_at=now() WHERE token_hash=$1 AND purpose=$2 AND site_id IS NOT DISTINCT FROM $3 AND consumed_at IS NULL AND expires_at>now()")
        .bind(hash(value)).bind(purpose).bind(site_id).execute(&mut **tx).await.is_ok_and(|r| r.rows_affected() == 1)
}

#[utoipa::path(get, path = "/subscribe/{site_id}", tag = "Public subscriptions", params(("site_id" = Uuid, Path)), responses((status = 200, description = "Subscription form"), (status = 404, description = "Site unavailable")))]
pub async fn subscribe_page(State(state): State<AppState>, Path(site_id): Path<Uuid>) -> Response {
    subscribe_form(&state, site_id, None).await
}

async fn subscribe_form(state: &AppState, site_id: Uuid, error: Option<&str>) -> Response {
    let site = sqlx::query_as::<_, (String, i32)>("SELECT original_url,interval_seconds FROM monitored_sites WHERE id=$1 AND public_subscriptions_enabled=true")
        .bind(site_id).fetch_optional(&state.database).await;
    let Some((url, interval)) = site.ok().flatten() else {
        return not_found("This subscription page is unavailable.");
    };
    let Ok(csrf) = nonce(&state.database, "subscribe", Some(site_id)).await else {
        return generic_error();
    };
    page(
        "subscribe.html",
        context! { site_url => url, interval_text => format_interval(interval), site_id => site_id.to_string(), csrf => csrf, site_key => state.human_verifier.site_key(), test_mode => state.human_verifier.is_development(), error => error.unwrap_or("") },
    )
}

#[utoipa::path(post, path = "/subscribe/{site_id}", tag = "Public subscriptions", params(("site_id" = Uuid, Path)), responses((status = 200, description = "Generic confirmation response"), (status = 400, description = "Invalid form"), (status = 404, description = "Site unavailable")))]
pub async fn subscribe(
    State(state): State<AppState>,
    Path(site_id): Path<Uuid>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Form(input): Form<SubscribeForm>,
) -> Response {
    let remote_key = remote.ip().to_string();
    if !allow_rate(
        &state.database,
        &state.token_key,
        &format!("ip:{remote_key}"),
        10,
    )
    .await
    {
        return confirmation(
            "If this address can subscribe, a confirmation email will be sent shortly.",
        );
    }
    let Some(email) = normalize_email(&input.email) else {
        return subscribe_form(&state, site_id, Some("Enter a valid email address.")).await;
    };
    if !allow_rate(
        &state.database,
        &state.token_key,
        &format!("email:{email}"),
        3,
    )
    .await
    {
        return confirmation(
            "If this address can subscribe, a confirmation email will be sent shortly.",
        );
    }
    if !verify_human(&state, &input.challenge, "subscribe").await {
        return subscribe_form(
            &state,
            site_id,
            Some("Human verification failed. Please try again."),
        )
        .await;
    }
    let mut tx = match state.database.begin().await {
        Ok(tx) => tx,
        Err(_) => return generic_error(),
    };
    if !consume_nonce(&mut tx, &input.csrf, "subscribe", Some(site_id)).await {
        return page(
            "error.html",
            context! {message => "This form expired. Reload the page and try again."},
        );
    }
    let site_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM monitored_sites WHERE id=$1 AND public_subscriptions_enabled=true)").bind(site_id).fetch_one(&mut *tx).await.unwrap_or(false);
    if !site_exists {
        return not_found("This subscription page is unavailable.");
    }
    let existing = sqlx::query_as::<_, (Uuid, String)>("SELECT id,status FROM subscriptions WHERE site_id=$1 AND lower(btrim(subscriber_identity))=$2 FOR UPDATE")
        .bind(site_id).bind(&email).fetch_optional(&mut *tx).await;
    let existing = match existing {
        Ok(v) => v,
        Err(_) => return generic_error(),
    };
    let subscription_id = if let Some((id, status)) = existing {
        if status == "active" {
            let _ = tx.commit().await;
            return confirmation(
                "If this address can subscribe, a confirmation email will be sent shortly.",
            );
        }
        id
    } else {
        let id = Uuid::new_v4();
        let inserted = sqlx::query_scalar::<_, Uuid>("INSERT INTO subscriptions(id,site_id,subscriber_identity,status) VALUES($1,$2,$3,'pending') ON CONFLICT DO NOTHING RETURNING id")
            .bind(id).bind(site_id).bind(&email).fetch_optional(&mut *tx).await;
        match inserted {
            Ok(Some(id)) => id,
            Ok(None) => {
                let raced = sqlx::query_as::<_, (Uuid,String)>("SELECT id,status FROM subscriptions WHERE site_id=$1 AND lower(btrim(subscriber_identity))=$2 FOR UPDATE")
                    .bind(site_id).bind(&email).fetch_optional(&mut *tx).await;
                match raced {
                    Ok(Some((id, status))) if status == "pending" => id,
                    Ok(Some(_)) => {
                        let _ = tx.commit().await;
                        return confirmation(
                            "If this address can subscribe, a confirmation email will be sent shortly.",
                        );
                    }
                    _ => return generic_error(),
                }
            }
            Err(_) => return generic_error(),
        }
    };
    let token_id = Uuid::new_v4();
    let token = derived_token(&state.token_key, "verify", token_id);
    if sqlx::query("UPDATE verification_tokens SET revoked_at=now() WHERE subscription_id=$1 AND consumed_at IS NULL AND revoked_at IS NULL")
        .bind(subscription_id).execute(&mut *tx).await.is_err() { return generic_error(); }
    if sqlx::query("INSERT INTO verification_tokens(token_id,token_hash,subscription_id,expires_at) VALUES($1,$2,$3,now()+($4 * interval '1 hour'))")
        .bind(token_id).bind(hash(&token)).bind(subscription_id).bind(VERIFY_TTL_HOURS).execute(&mut *tx).await.is_err() { return generic_error(); }
    if sqlx::query(
        "INSERT INTO email_action_outbox(id,action,recipient,token_id) VALUES($1,'verify',$2,$3)",
    )
    .bind(Uuid::new_v4())
    .bind(email)
    .bind(token_id)
    .execute(&mut *tx)
    .await
    .is_err()
    {
        return generic_error();
    }
    if tx.commit().await.is_err() {
        return generic_error();
    }
    confirmation("If this address can subscribe, a confirmation email will be sent shortly.")
}

#[utoipa::path(get, path = "/verify/{token}", tag = "Public subscriptions", params(("token" = String, Path)), responses((status = 200, description = "Read-only verification confirmation"), (status = 404, description = "Invalid or expired token")))]
pub async fn verify_page(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    let token_hash = hash(&token);
    let token_id = sqlx::query_scalar::<_, Uuid>("SELECT token_id FROM verification_tokens WHERE token_hash=$1 AND expires_at>now() AND consumed_at IS NULL AND revoked_at IS NULL")
        .bind(token_hash).fetch_optional(&state.database).await;
    let Some(id) = token_id.ok().flatten() else {
        return not_found("This verification link is invalid or expired.");
    };
    let Ok(csrf) = nonce(&state.database, "verify", None).await else {
        return generic_error();
    };
    token_response(
        "verify.html",
        context! {token => token, csrf => csrf, token_id => id.to_string()},
    )
}

#[utoipa::path(post, path = "/verify/{token}", tag = "Public subscriptions", params(("token" = String, Path)), responses((status = 200, description = "Subscription verified"), (status = 404, description = "Invalid or expired token")))]
pub async fn verify(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Form(input): Form<NonceForm>,
) -> Response {
    let mut tx = match state.database.begin().await {
        Ok(tx) => tx,
        Err(_) => return generic_error(),
    };
    if !consume_nonce(&mut tx, &input.csrf, "verify", None).await {
        return token_response(
            "error.html",
            context! {message => "This confirmation form expired. Open the link again."},
        );
    }
    let result = sqlx::query("UPDATE verification_tokens SET consumed_at=now() WHERE token_hash=$1 AND expires_at>now() AND consumed_at IS NULL AND revoked_at IS NULL RETURNING subscription_id")
        .bind(hash(&token)).fetch_optional(&mut *tx).await;
    let Some(row) = result.ok().flatten() else {
        return not_found("This verification link is invalid or expired.");
    };
    let subscription_id: Uuid = row.try_get("subscription_id").unwrap_or(Uuid::nil());
    if sqlx::query("UPDATE subscriptions SET status='active' WHERE id=$1 AND status='pending'")
        .bind(subscription_id)
        .execute(&mut *tx)
        .await
        .is_err()
        || tx.commit().await.is_err()
    {
        return generic_error();
    }
    token_response(
        "confirmation.html",
        context! {message => "Your email is verified. This website is now in your subscriptions."},
    )
}

#[utoipa::path(get, path = "/manage", tag = "Public management", responses((status = 200, description = "Management request form or scoped subscription list")))]
pub async fn manage(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(cookie) = cookie_value(&headers, "pagerune_session")
        && let Some((email, csrf)) = session(&state.database, &cookie).await
    {
        let rows = sqlx::query_as::<_, (Uuid, String, i32, chrono::DateTime<chrono::Utc>)>("SELECT s.id,m.original_url AS site_url,m.interval_seconds,s.created_at FROM subscriptions s JOIN monitored_sites m ON m.id=s.site_id WHERE s.subscriber_identity=$1 AND s.status='active' ORDER BY s.created_at DESC")
            .bind(&email).fetch_all(&state.database).await.unwrap_or_default();
        let rows: Vec<ManagedSubscription> = rows
            .into_iter()
            .map(|(id, site_url, seconds, created_at)| ManagedSubscription {
                id,
                site_url,
                interval_text: format_interval(seconds),
                created_at,
            })
            .collect();
        return page(
            "manage.html",
            context! {authenticated => true, email => email, csrf => csrf, subscriptions => rows},
        );
    }
    let csrf = nonce(&state.database, "manage_request", None)
        .await
        .unwrap_or_default();
    page(
        "manage.html",
        context! {authenticated => false, csrf => csrf, site_key => state.human_verifier.site_key(), test_mode => state.human_verifier.is_development()},
    )
}

#[utoipa::path(post, path = "/manage", tag = "Public management", responses((status = 200, description = "Generic confirmation response")))]
pub async fn request_management(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Form(input): Form<ManageForm>,
) -> Response {
    let remote_key = remote.ip().to_string();
    if !allow_rate(
        &state.database,
        &state.token_key,
        &format!("ip:{remote_key}"),
        10,
    )
    .await
    {
        return confirmation(
            "If this address has subscriptions, a management link will be sent shortly.",
        );
    }
    let Some(email) = normalize_email(&input.email) else {
        return confirmation(
            "If this address has subscriptions, a management link will be sent shortly.",
        );
    };
    if !allow_rate(
        &state.database,
        &state.token_key,
        &format!("manage:{email}"),
        3,
    )
    .await
    {
        return confirmation(
            "If this address has subscriptions, a management link will be sent shortly.",
        );
    }
    if !verify_human(&state, &input.challenge, "manage").await {
        return page(
            "manage.html",
            context! {error => "Human verification failed. Please try again.", csrf => input.csrf, site_key => state.human_verifier.site_key(), test_mode => state.human_verifier.is_development()},
        );
    }
    let mut tx = match state.database.begin().await {
        Ok(tx) => tx,
        Err(_) => return generic_error(),
    };
    if !consume_nonce(&mut tx, &input.csrf, "manage_request", None).await {
        return page(
            "error.html",
            context! {message => "This form expired. Reload the page and try again."},
        );
    }
    let token_id = Uuid::new_v4();
    let token = derived_token(&state.token_key, "manage", token_id);
    let inserted = sqlx::query("INSERT INTO management_tokens(token_id,token_hash,subscriber_identity,expires_at) VALUES($1,$2,$3,now()+($4 * interval '1 minute'))")
        .bind(token_id).bind(hash(&token)).bind(&email).bind(MANAGE_TOKEN_TTL_MINUTES).execute(&mut *tx).await;
    if inserted.is_err() || sqlx::query("INSERT INTO email_action_outbox(id,action,recipient,token_id) VALUES($1,'manage',$2,$3)").bind(Uuid::new_v4()).bind(&email).bind(token_id).execute(&mut *tx).await.is_err() { return generic_error(); }
    if tx.commit().await.is_err() {
        return generic_error();
    }
    confirmation("If this address has subscriptions, a management link will be sent shortly.")
}

#[utoipa::path(get, path = "/manage/{token}", tag = "Public management", params(("token" = String, Path)), responses((status = 200, description = "Read-only management session exchange form"), (status = 404, description = "Invalid or expired token")))]
pub async fn management_exchange_page(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Response {
    let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM management_tokens WHERE token_hash=$1 AND expires_at>now() AND consumed_at IS NULL AND revoked_at IS NULL)")
        .bind(hash(&token)).fetch_one(&state.database).await.unwrap_or(false);
    if !valid {
        return not_found("This management link is invalid or expired.");
    }
    let csrf = nonce(&state.database, "manage_exchange", None)
        .await
        .unwrap_or_default();
    token_response("manage_link.html", context! {token => token, csrf => csrf})
}

#[utoipa::path(post, path = "/manage/{token}", tag = "Public management", params(("token" = String, Path)), responses((status = 303, description = "Management session created"), (status = 404, description = "Invalid or expired token")))]
pub async fn exchange_management_token(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Form(input): Form<NonceForm>,
) -> Response {
    let mut tx = match state.database.begin().await {
        Ok(tx) => tx,
        Err(_) => return generic_error(),
    };
    if !consume_nonce(&mut tx, &input.csrf, "manage_exchange", None).await {
        return token_response(
            "error.html",
            context! {message => "This form expired. Open the link again."},
        );
    }
    let identity = sqlx::query_scalar::<_, String>("UPDATE management_tokens SET consumed_at=now() WHERE token_hash=$1 AND expires_at>now() AND consumed_at IS NULL AND revoked_at IS NULL RETURNING subscriber_identity")
        .bind(hash(&token)).fetch_optional(&mut *tx).await;
    let Some(identity) = identity.ok().flatten() else {
        return not_found("This management link is invalid or expired.");
    };
    let session_token = fresh_token();
    let csrf = fresh_token();
    let inserted = sqlx::query("INSERT INTO management_sessions(session_hash,subscriber_identity,csrf_hash,expires_at) VALUES($1,$2,$3,now()+($4 * interval '1 minute'))")
        .bind(hash(&session_token)).bind(identity).bind(hash(&csrf)).bind(SESSION_TTL_MINUTES).execute(&mut *tx).await;
    if inserted.is_err() || tx.commit().await.is_err() {
        return generic_error();
    }
    let mut response = Redirect::to("/manage").into_response();
    let secure = state.public_base_url.starts_with("https://");
    let cookie = format!(
        "pagerune_session={session_token}; Path=/manage; HttpOnly; SameSite=Strict; Max-Age=1800{}",
        if secure { "; Secure" } else { "" }
    );
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().append("set-cookie", value);
    }
    response
}

#[utoipa::path(post, path = "/manage/unsubscribe/{id}", tag = "Public management", params(("id" = Uuid, Path)), responses((status = 303, description = "Subscription removed from this email identity"), (status = 403, description = "Invalid CSRF token")))]
pub async fn management_unsubscribe(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Form(input): Form<UnsubscribeForm>,
) -> Response {
    let Some(session_token) = cookie_value(&headers, "pagerune_session") else {
        return Redirect::to("/manage").into_response();
    };
    let Some((identity, csrf)) = session(&state.database, &session_token).await else {
        return Redirect::to("/manage").into_response();
    };
    if input.csrf != csrf {
        return (StatusCode::FORBIDDEN, "Form validation failed").into_response();
    }
    let result = async {
        let mut tx = state.database.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 7349871))")
            .bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM subscriptions WHERE id=$1 AND lower(btrim(subscriber_identity))=$2 AND status='active'")
            .bind(id).bind(identity).execute(&mut *tx).await?;
        tx.commit().await
    }.await;
    if result.is_err() {
        return generic_error();
    }
    Redirect::to("/manage").into_response()
}

async fn session(pool: &PgPool, token: &str) -> Option<(String, String)> {
    let row = sqlx::query_as::<_, (String,String)>("SELECT subscriber_identity,csrf_hash FROM management_sessions WHERE session_hash=$1 AND revoked_at IS NULL AND expires_at>now()")
        .bind(hash(token)).fetch_optional(pool).await.ok().flatten()?;
    Some((row.0, row.1))
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            (key == name).then(|| value.to_owned())
        })
}

fn normalize_email(value: &str) -> Option<String> {
    let email = value.trim().to_lowercase();
    if email.len() > 320 || email.parse::<lettre::Address>().is_err() {
        return None;
    }
    Some(email)
}

async fn verify_human(state: &AppState, token: &str, action: &str) -> bool {
    if token.is_empty() || token.len() > 2048 {
        return false;
    }
    let Some(hostname) = url::Url::parse(&state.public_base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
    else {
        return false;
    };
    state.human_verifier.verify(token, action, &hostname).await
}

async fn validate_turnstile(secret: &str, token: &str, action: &str, hostname: &str) -> bool {
    if token.is_empty() || token.len() > 2048 {
        return false;
    }
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("secret", secret)
        .append_pair("response", token)
        .finish();
    let result = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .no_proxy()
        .build();
    let Ok(client) = result else {
        return false;
    };
    let Ok(response) = client
        .post("https://challenges.cloudflare.com/turnstile/v0/siteverify")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
    else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    let Ok(bytes) = response.bytes().await else {
        return false;
    };
    valid_turnstile_response(&bytes, action, hostname)
}

fn valid_turnstile_response(bytes: &[u8], action: &str, hostname: &str) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    value["success"] == true
        && value["action"].as_str() == Some(action)
        && value["hostname"].as_str() == Some(hostname)
}

async fn allow_rate(pool: &PgPool, secret: &[u8; 32], key: &str, limit: i32) -> bool {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(b"pagerune:rate-limit:");
    mac.update(key.as_bytes());
    let digest = hex::encode(mac.finalize().into_bytes());
    sqlx::query_scalar::<_, i32>("INSERT INTO web_rate_limits(rate_key,bucket_start,request_count,expires_at) VALUES($1,date_trunc('hour',now()),1,date_trunc('hour',now())+interval '2 hours') ON CONFLICT(rate_key,bucket_start) DO UPDATE SET request_count=web_rate_limits.request_count+1 RETURNING request_count")
        .bind(digest).fetch_one(pool).await.is_ok_and(|count| count <= limit)
}

fn confirmation(message: &str) -> Response {
    page("confirmation.html", context! {message => message})
}
fn not_found(message: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        page("error.html", context! {message => message}),
    )
        .into_response()
}
fn generic_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Request could not be completed",
    )
        .into_response()
}

async fn stylesheet() -> Response {
    (
        [("content-type", "text/css; charset=utf-8")],
        include_str!("../static/css/pagerune.css"),
    )
        .into_response()
}

async fn management_script() -> Response {
    (
        [("content-type", "text/javascript; charset=utf-8")],
        include_str!("../static/js/management.js"),
    )
        .into_response()
}

pub async fn run_action_mailer(
    pool: PgPool,
    smtp: SmtpConfig,
    key: [u8; 32],
    concurrency: usize,
    max_attempts: i32,
    cancel: CancellationToken,
) {
    let mut tasks = JoinSet::new();
    let mut cleanup = tokio::time::interval(Duration::from_secs(3600));
    loop {
        while tasks.len() < concurrency {
            match claim_action_email(&pool).await {
                Ok(Some(job)) => {
                    let (pool, smtp, key) = (pool.clone(), smtp.clone(), key);
                    tasks.spawn(async move {
                        deliver_action_email(pool, smtp, key, job, max_attempts).await;
                    });
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => { while tasks.join_next().await.is_some() {} return; },
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            _ = cleanup.tick() => { cleanup_expired(&pool).await; },
            _ = tokio::time::sleep(Duration::from_millis(500)), if tasks.is_empty() => {}
        }
    }
}

async fn cleanup_expired(pool: &PgPool) {
    for query in [
        "DELETE FROM verification_tokens WHERE expires_at < now() - interval '7 days' OR consumed_at < now() - interval '7 days' OR revoked_at < now() - interval '7 days'",
        "DELETE FROM management_tokens WHERE expires_at < now() - interval '7 days' OR consumed_at < now() - interval '7 days' OR revoked_at < now() - interval '7 days'",
        "DELETE FROM management_sessions WHERE expires_at < now() OR revoked_at < now() - interval '7 days'",
        "DELETE FROM web_form_nonces WHERE expires_at < now() OR consumed_at < now() - interval '1 day'",
        "DELETE FROM web_rate_limits WHERE expires_at < now()",
    ] {
        if let Err(error) = sqlx::query(query).execute(pool).await {
            tracing::warn!(error = %error, "expired public subscription state cleanup failed");
            break;
        }
    }
}

async fn claim_action_email(pool: &PgPool) -> Result<Option<ActionEmail>, sqlx::Error> {
    sqlx::query_as::<_,ActionEmail>("WITH due AS (SELECT id FROM email_action_outbox WHERE status IN ('pending','sending') AND next_attempt_at<=now() AND (lease_expires_at IS NULL OR lease_expires_at<=now()) ORDER BY next_attempt_at,created_at FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE email_action_outbox o SET status='sending',lease_owner=$1,lease_expires_at=now()+interval '180 seconds',lease_generation=o.lease_generation+1,attempt_count=o.attempt_count+1 FROM due WHERE o.id=due.id RETURNING o.id,o.action,o.recipient,o.token_id,o.lease_owner,o.lease_generation,o.attempt_count")
        .bind(Uuid::new_v4()).fetch_optional(pool).await
}

async fn deliver_action_email(
    pool: PgPool,
    config: SmtpConfig,
    key: [u8; 32],
    job: ActionEmail,
    max_attempts: i32,
) {
    let (token, valid) = if job.action == "verify" {
        let token = derived_token(&key, "verify", job.token_id);
        let valid = sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM verification_tokens WHERE token_id=$1 AND token_hash=$2 AND expires_at>now() AND consumed_at IS NULL AND revoked_at IS NULL)").bind(job.token_id).bind(hash(&token)).fetch_one(&pool).await.unwrap_or(false);
        (token, valid)
    } else {
        let token = derived_token(&key, "manage", job.token_id);
        let valid = sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM management_tokens WHERE token_id=$1 AND token_hash=$2 AND expires_at>now() AND consumed_at IS NULL AND revoked_at IS NULL)").bind(job.token_id).bind(hash(&token)).fetch_one(&pool).await.unwrap_or(false);
        (token, valid)
    };
    if !valid {
        let _ = sqlx::query("UPDATE email_action_outbox SET status='failed',last_error='token_not_active',lease_owner=NULL,lease_expires_at=NULL WHERE id=$1 AND lease_owner=$2 AND lease_generation=$3").bind(job.id).bind(job.lease_owner).bind(job.lease_generation).execute(&pool).await;
        return;
    }
    let route = if job.action == "verify" {
        "verify"
    } else {
        "manage"
    };
    let url = format!(
        "{}/{}/{}",
        config.public_base_url.trim_end_matches('/'),
        route,
        token
    );
    let subject = if job.action == "verify" {
        "Verify your PageRune subscription"
    } else {
        "Your PageRune management link"
    };
    let text = if job.action == "verify" {
        format!(
            "Confirm your email to activate this PageRune subscription: {url}\nThis link expires in 24 hours."
        )
    } else {
        format!(
            "Open this temporary PageRune management link: {url}\nThis link expires in 15 minutes."
        )
    };
    let result = send_action_smtp(&config, &job.recipient, subject, &text).await;
    match result {
        Ok(()) => {
            let _ = sqlx::query("UPDATE email_action_outbox SET status='delivered',delivered_at=now(),lease_owner=NULL,lease_expires_at=NULL,last_error=NULL WHERE id=$1 AND status='sending' AND lease_owner=$2 AND lease_generation=$3 AND lease_expires_at>now()").bind(job.id).bind(job.lease_owner).bind(job.lease_generation).execute(&pool).await;
        }
        Err(code) => {
            let terminal = job.attempt_count >= max_attempts;
            let delay = (2i64.pow(job.attempt_count.min(12) as u32)).min(21600);
            let _ = sqlx::query("UPDATE email_action_outbox SET status=$4,next_attempt_at=now()+($5 * interval '1 second'),lease_owner=NULL,lease_expires_at=NULL,last_error=$6 WHERE id=$1 AND lease_owner=$2 AND lease_generation=$3").bind(job.id).bind(job.lease_owner).bind(job.lease_generation).bind(if terminal{"failed"}else{"pending"}).bind(delay).bind(code).execute(&pool).await;
        }
    }
}

async fn send_action_smtp(
    config: &SmtpConfig,
    recipient: &str,
    subject: &str,
    body: &str,
) -> Result<(), &'static str> {
    let from: Mailbox = format!("{} <{}>", config.from_name, config.from_email)
        .parse()
        .map_err(|_| "invalid_sender_address")?;
    let to: Mailbox = recipient.parse().map_err(|_| "invalid_recipient_address")?;
    let email = Message::builder()
        .from(from)
        .to(to)
        .subject(subject)
        .body(body.to_owned())
        .map_err(|_| "email_build_failed")?;
    let mut builder = match config.tls_mode.as_str() {
        "implicit" => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
            .map_err(|_| "smtp_configuration_failed")?,
        "starttls" => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
            .map_err(|_| "smtp_configuration_failed")?,
        _ => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host).tls(Tls::None),
    };
    builder = builder.port(config.port).timeout(Some(config.timeout));
    if let (Some(username), Some(password)) = (&config.username, &config.password) {
        builder = builder.credentials(Credentials::new(username.clone(), password.clone()));
    }
    tokio::time::timeout(config.timeout, builder.build().send(email))
        .await
        .map_err(|_| "smtp_timeout")?
        .map(|_| ())
        .map_err(|_| "smtp_delivery_failed")
}

// Used by the development-only JSON API; it enqueues the same double-opt-in flow.
pub async fn create_pending_api(
    state: &AppState,
    site_id: Uuid,
    email: &str,
) -> Result<Uuid, &'static str> {
    let email = normalize_email(email).ok_or("invalid_email")?;
    let mut tx = state.database.begin().await.map_err(|_| "database_error")?;
    let site =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM monitored_sites WHERE id=$1)")
            .bind(site_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| "database_error")?;
    if !site {
        return Err("site_not_found");
    }
    let existing = sqlx::query_as::<_,(Uuid,String)>("SELECT id,status FROM subscriptions WHERE site_id=$1 AND lower(btrim(subscriber_identity))=$2 FOR UPDATE").bind(site_id).bind(&email).fetch_optional(&mut *tx).await.map_err(|_|"database_error")?;
    if let Some((id, status)) = existing.as_ref()
        && status == "active"
    {
        let id = *id;
        tx.commit().await.map_err(|_| "database_error")?;
        return Ok(id);
    }
    let subscription_id = if let Some((id, _)) = existing {
        id
    } else {
        let id = Uuid::new_v4();
        match sqlx::query_scalar::<_,Uuid>("INSERT INTO subscriptions(id,site_id,subscriber_identity,status) VALUES($1,$2,$3,'pending') ON CONFLICT DO NOTHING RETURNING id")
            .bind(id).bind(site_id).bind(&email).fetch_optional(&mut *tx).await.map_err(|_|"database_error")? {
                Some(id) => id,
                None => {
                    let (id,status) = sqlx::query_as::<_,(Uuid,String)>("SELECT id,status FROM subscriptions WHERE site_id=$1 AND lower(btrim(subscriber_identity))=$2 FOR UPDATE")
                        .bind(site_id).bind(&email).fetch_one(&mut *tx).await.map_err(|_|"database_error")?;
                    if status == "active" { tx.commit().await.map_err(|_|"database_error")?; return Ok(id); }
                    id
                }
            }
    };
    let token_id = Uuid::new_v4();
    let token = derived_token(&state.token_key, "verify", token_id);
    sqlx::query("UPDATE verification_tokens SET revoked_at=now() WHERE subscription_id=$1 AND consumed_at IS NULL AND revoked_at IS NULL").bind(subscription_id).execute(&mut *tx).await.map_err(|_|"database_error")?;
    sqlx::query("INSERT INTO verification_tokens(token_id,token_hash,subscription_id,expires_at) VALUES($1,$2,$3,now()+interval '24 hours')").bind(token_id).bind(hash(&token)).bind(subscription_id).execute(&mut *tx).await.map_err(|_|"database_error")?;
    sqlx::query(
        "INSERT INTO email_action_outbox(id,action,recipient,token_id) VALUES($1,'verify',$2,$3)",
    )
    .bind(Uuid::new_v4())
    .bind(&email)
    .bind(token_id)
    .execute(&mut *tx)
    .await
    .map_err(|_| "database_error")?;
    tx.commit().await.map_err(|_| "database_error")?;
    Ok(subscription_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use tokio::sync::OnceCell;

    static DATABASE: OnceCell<PgPool> = OnceCell::const_new();

    async fn database() -> PgPool {
        DATABASE
            .get_or_init(|| async {
                let url = std::env::var("DATABASE_URL")
                    .expect("DATABASE_URL is required for PostgreSQL tests");
                let pool = PgPoolOptions::new()
                    .max_connections(8)
                    .connect(&url)
                    .await
                    .unwrap();
                sqlx::migrate!().run(&pool).await.unwrap();
                pool
            })
            .await
            .clone()
    }
    #[test]
    fn formats_monitoring_intervals_without_rounding_or_bad_grammar() {
        for (seconds, expected) in [
            (60, "Every minute"),
            (300, "Every 5 minutes"),
            (900, "Every 15 minutes"),
            (3600, "Every hour"),
            (7200, "Every 2 hours"),
            (86400, "Every day"),
        ] {
            assert_eq!(format_interval(seconds), expected);
        }
        assert_eq!(format_interval(61), "Every 61 seconds");
    }

    #[test]
    fn subscription_url_uses_configured_public_origin_and_stable_site_id() {
        let site_id = Uuid::parse_str("f56f3e71-4f64-4f10-a246-084d38c3816f").unwrap();
        assert_eq!(
            subscription_url("https://public.example/pagerune///", site_id),
            format!("https://public.example/pagerune/subscribe/{site_id}")
        );
    }

    #[tokio::test]
    async fn management_template_renders_and_escapes_site_names() {
        let response = render(
            "management.html",
            context! {
                sites => vec![serde_json::json!({
                    "id": Uuid::new_v4().to_string(),
                    "name": "<script>alert(1)</script>",
                    "original_url": "https://example.test/",
                    "subscription_url": "https://public.example/subscribe/f56f3e71-4f64-4f10-a246-084d38c3816f?x=1&y=2",
                    "public_subscriptions_enabled": false,
                    "current_status": "ok",
                    "monitoring_enabled": true,
                    "active_subscriber_count": 2,
                    "interval_text": "Every day",
                    "last_error": Option::<String>::None,
                    "last_checked_at": Option::<String>::None,
                    "last_detected_change_at": Option::<String>::None,
                })],
                csrf => "nonce",
                notice => "",
                error => "",
                form_name => "",
                form_url => "",
                page => 1,
                pages => 1,
                total => 1,
            },
        );
        assert_eq!(response.status(), StatusCode::OK);
        let html = String::from_utf8(
            axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;&#x2f;script&gt;"),
            "{html}"
        );
        assert!(!html.contains("<script>alert(1)</script>"), "{html}");
        assert!(
            html.contains("data-label=\"Subscriptions\"><span class=\"status-pill status-off\">Disabled</span>"),
            "{html}"
        );
        assert!(
            html.contains(
                "aria-label=\"Website details for &lt;script&gt;alert(1)&lt;&#x2f;script&gt;\""
            ),
            "{html}"
        );
        assert!(html.contains("Permanent subscription URL"), "{html}");
        assert!(html.contains("View subscription page"), "{html}");
        assert!(html.contains("Copy subscription link"), "{html}");
        assert!(html.contains("Edit website"), "{html}");
        assert!(html.contains("Remove website"), "{html}");
        assert!(html.contains("data-copy-url=\"https:&#x2f;&#x2f;public.example&#x2f;subscribe&#x2f;f56f3e71-4f64-4f10-a246-084d38c3816f?x=1&amp;y=2\""), "{html}");
        assert!(html.contains("Register a website"));
    }

    #[test]
    fn turnstile_response_requires_success_matching_action_and_hostname() {
        assert!(valid_turnstile_response(
            br#"{"success":true,"action":"subscribe","hostname":"pagerune.example"}"#,
            "subscribe",
            "pagerune.example"
        ));
        for response in [
            br#"{"success":false,"error-codes":["invalid-input-response"]}"#.as_slice(),
            br#"{"success":false,"error-codes":["timeout-or-duplicate"]}"#,
            br#"{"success":false,"error-codes":["internal-error"]}"#,
            br#"{"success":true,"action":"manage","hostname":"pagerune.example"}"#,
            br#"{"success":true,"action":"subscribe","hostname":"attacker.example"}"#,
            b"unavailable",
        ] {
            assert!(!valid_turnstile_response(
                response,
                "subscribe",
                "pagerune.example"
            ));
        }
    }

    #[test]
    fn generated_tokens_are_scoped_and_not_their_database_ids() {
        let key = [7u8; 32];
        let id = Uuid::new_v4();
        let verify = derived_token(&key, "verify", id);
        let manage = derived_token(&key, "manage", id);
        assert_eq!(verify.len(), 64);
        assert_ne!(verify, manage);
        assert_ne!(verify, id.to_string());
    }
    #[test]
    fn email_normalization_validates_and_keeps_aliases() {
        assert_eq!(
            normalize_email(" Alice+news@Example.COM ").as_deref(),
            Some("alice+news@example.com")
        );
        assert!(normalize_email("not-email").is_none());
    }

    #[tokio::test]
    async fn rate_limit_updates_are_atomic_across_tasks() {
        let pool = database().await;
        let secret = [4u8; 32];
        let request_key = format!("rate-test:{}", Uuid::new_v4());
        let mut tasks = JoinSet::new();
        for _ in 0..12 {
            let pool = pool.clone();
            let request_key = request_key.clone();
            tasks.spawn(async move { allow_rate(&pool, &secret, &request_key, 4).await });
        }
        let mut allowed = 0;
        while let Some(result) = tasks.join_next().await {
            if result.unwrap() {
                allowed += 1;
            }
        }
        assert_eq!(allowed, 4);
    }
}
