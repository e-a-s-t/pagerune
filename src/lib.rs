pub mod api;
pub mod config;
pub mod fetcher;
pub mod notifications;
pub mod web;
pub mod worker;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    extract::{DefaultBodyLimit, Form, Path},
    http::{HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use sha2::Digest;
use sqlx::PgPool;
use std::sync::Arc;
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub database: PgPool,
    pub dev_api_enabled: bool,
    pub public_base_url: String,
    pub token_key: [u8; 32],
    pub human_verifier: Arc<dyn web::HumanVerifier>,
}

#[derive(Serialize, ToSchema)]
pub struct Health {
    pub status: String,
}

#[derive(Serialize, ToSchema)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub request_id: String,
}

fn error_response(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    let body = ErrorBody {
        code: code.into(),
        message: message.into(),
        request_id: Uuid::new_v4().to_string(),
    };
    (status, Json(body)).into_response()
}

#[utoipa::path(get, path = "/healthz", tag = "Operations", responses((status = 200, description = "Process is running", body = Health)))]
async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok".into(),
    })
}

#[utoipa::path(get, path = "/readyz", tag = "Operations", responses((status = 200, description = "PostgreSQL is reachable", body = Health), (status = 503, description = "PostgreSQL is unavailable", body = ErrorBody)))]
async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.database).await {
        Ok(_) => (
            StatusCode::OK,
            Json(Health {
                status: "ready".into(),
            }),
        )
            .into_response(),
        Err(_) => {
            tracing::warn!("readiness database check failed");
            error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "not_ready",
                "Service is not ready",
            )
        }
    }
}

async fn not_found() -> impl IntoResponse {
    error_response(StatusCode::NOT_FOUND, "not_found", "Resource not found")
}

async fn dev_api_gate(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if !state.dev_api_enabled {
        return normalize_api_response(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "api_disabled",
            "Business API is disabled until development access is explicitly enabled",
        ))
        .await;
    }
    normalize_api_response(next.run(request).await).await
}

async fn normalize_api_response(response: Response) -> Response {
    let (mut parts, body) = response.into_parts();
    let status = parts.status;
    let bytes = match to_bytes(body, 8192).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Request could not be completed",
            );
        }
    };
    let parsed = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
    let valid_error = parsed.as_ref().is_some_and(|value| {
        value["code"].as_str().is_some()
            && value["message"].as_str().is_some()
            && value["request_id"].as_str().is_some()
    });
    if (status.is_client_error() || status.is_server_error()) && !valid_error {
        return error_response(status, "invalid_request", "Request could not be processed");
    }
    let request_id = parsed
        .as_ref()
        .and_then(|value| value["request_id"].as_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        parts.headers.insert("x-request-id", value);
    }
    Response::from_parts(parts, Body::from(bytes))
}

#[utoipa::path(get, path = "/unsubscribe/{token}", tag = "Subscriptions", params(("token" = String, Path)), responses((status = 200, description = "Confirmation page; no state change"), (status = 404, description = "Invalid or expired token")))]
async fn unsubscribe_confirm(Path(token): Path<String>, State(state): State<AppState>) -> Response {
    let hash = hex::encode(sha2::Sha256::digest(token.as_bytes()));
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM unsubscribe_tokens WHERE token_hash=$1 AND revoked_at IS NULL AND EXISTS(SELECT 1 FROM subscriptions s WHERE s.id=unsubscribe_tokens.subscription_id))")
        .bind(hash).fetch_one(&state.database).await.unwrap_or(false);
    if !exists {
        return (
            StatusCode::NOT_FOUND,
            "This unsubscribe link is invalid or expired.",
        )
            .into_response();
    }
    let mut response =
        axum::response::Html(include_str!("../templates/email/unsubscribe.html").replace(
            "<form method=\"post\">",
            "<form method=\"post\"><input type=\"hidden\" name=\"unsubscribe\" value=\"confirm\">",
        ))
        .into_response();
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}

#[utoipa::path(post, path = "/unsubscribe/{token}", tag = "Subscriptions", params(("token" = String, Path)), responses((status = 200, description = "Notifications disabled; safe to repeat"), (status = 500, description = "Request failed")))]
async fn unsubscribe_post(
    Path(token): Path<String>,
    State(state): State<AppState>,
    Form(_form): Form<std::collections::HashMap<String, String>>,
) -> Response {
    unsubscribe(&state.database, &token).await
}

#[derive(OpenApi)]
#[openapi(
    info(title = "PageRune", version = "0.3.0", description = "Public opt-in subscriptions use email verification. The development business API is disabled by default; local Compose access is protected by OAuth2 Proxy."),
    paths(healthz, readyz, unsubscribe_confirm, unsubscribe_post,
        web::index, web::subscribe_page, web::subscribe, web::verify_page, web::verify,
        web::manage, web::request_management, web::management_exchange_page,
        web::exchange_management_token, web::management_unsubscribe,
        api::create_site, api::list_sites, api::get_site, api::update_site, api::delete_site,
        api::create_subscription, api::list_subscriptions, api::get_subscription, api::delete_subscription,
        api::list_changes, api::get_change),
    components(schemas(Health, ErrorBody, api::ApiErrorBody, api::Site, api::Subscription, api::Change, api::Page<api::Site>, api::Page<api::Subscription>, api::Page<api::Change>, api::CreateSite, api::UpdateSite, api::CreateSubscription))
)]
pub struct ApiDoc;

pub fn app(state: AppState) -> Router {
    let business =
        api::router().route_layer(middleware::from_fn_with_state(state.clone(), dev_api_gate));
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route(
            "/unsubscribe/{token}",
            get(unsubscribe_confirm).post(unsubscribe_post),
        )
        .merge(web::router())
        .merge(business)
        .fallback(not_found)
        .merge(SwaggerUi::new("/swagger-ui").url("/api/openapi.json", ApiDoc::openapi()))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(state)
}

async fn unsubscribe(pool: &PgPool, token: &str) -> Response {
    use sha2::Digest;
    let hash = hex::encode(sha2::Sha256::digest(token.as_bytes()));
    let result = async {
        let mut tx = pool.begin().await?;
        let subscription_id = sqlx::query_scalar::<_, uuid::Uuid>("SELECT subscription_id FROM unsubscribe_tokens WHERE token_hash=$1 AND revoked_at IS NULL")
            .bind(&hash).fetch_optional(&mut *tx).await?;
        if let Some(subscription_id) = subscription_id {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 7349871))")
                .bind(subscription_id).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM subscriptions WHERE id=$1")
                .bind(subscription_id).execute(&mut *tx).await?;
        }
        tx.commit().await
    }.await;
    let mut response = match result {
        Ok(_) => axum::response::Html("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Unsubscribed</title><h1>Notifications disabled</h1><p>This subscription will no longer receive notifications.</p></html>").into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Request could not be completed").into_response(),
    };
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}
