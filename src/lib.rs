pub mod config;

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub database: PgPool,
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

fn error_response(
    status: StatusCode,
    code: &'static str,
    message: &'static str,
) -> impl IntoResponse {
    let body = ErrorBody {
        code: code.into(),
        message: message.into(),
        request_id: Uuid::new_v4().to_string(),
    };
    (status, Json(body))
}

#[utoipa::path(get, path = "/healthz", tag = "Operations",
    responses((status = 200, description = "Process is running", body = Health)))]
async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok".into(),
    })
}

#[utoipa::path(get, path = "/readyz", tag = "Operations",
    responses(
        (status = 200, description = "PostgreSQL is reachable", body = Health),
        (status = 503, description = "PostgreSQL is unavailable", body = ErrorBody)
    ))]
async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.database).await {
        Ok(_) => (
            StatusCode::OK,
            Json(Health {
                status: "ready".into(),
            }),
        ),
        Err(_) => {
            tracing::warn!("readiness database check failed");
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "not_ready",
                "Service is not ready",
            )
            .into_response();
        }
    }
    .into_response()
}

async fn not_found() -> impl IntoResponse {
    error_response(StatusCode::NOT_FOUND, "not_found", "Resource not found")
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "PageRune",
        version = "0.1.0",
        description = "Implemented scaffold endpoints only. Planned business APIs are documented in docs/Api.md."
    ),
    paths(healthz, readyz),
    components(schemas(Health, ErrorBody))
)]
pub struct ApiDoc;

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .fallback(not_found)
        .merge(SwaggerUi::new("/swagger-ui").url("/api/openapi.json", ApiDoc::openapi()))
        .with_state(state)
}
