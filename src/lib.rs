pub mod api;
pub mod config;
pub mod fetcher;
pub mod worker;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub database: PgPool,
    pub dev_api_enabled: bool,
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

#[derive(OpenApi)]
#[openapi(
    info(title = "PageRune", version = "0.2.0", description = "Core monitoring API. Business routes require the loopback-only development API gate until authentication is implemented."),
    paths(healthz, readyz,
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
        .merge(business)
        .fallback(not_found)
        .merge(SwaggerUi::new("/swagger-ui").url("/api/openapi.json", ApiDoc::openapi()))
        .with_state(state)
}
