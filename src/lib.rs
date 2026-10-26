use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

#[derive(Clone)]
pub struct AppState {
    pub database: PgPool,
}

#[derive(Serialize, ToSchema)]
pub struct Health {
    pub status: String,
}

#[utoipa::path(get, path = "/healthz", tag = "Operations",
    responses((status = 200, description = "Process is running", body = Health)))]
async fn healthz() -> Json<Health> {
    Json(Health { status: "ok".into() })
}

#[utoipa::path(get, path = "/readyz", tag = "Operations",
    responses(
        (status = 200, description = "PostgreSQL is reachable", body = Health),
        (status = 503, description = "PostgreSQL is unavailable", body = Health)
    ))]
async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<Health>) {
    match sqlx::query("SELECT 1").execute(&state.database).await {
        Ok(_) => (StatusCode::OK, Json(Health { status: "ready".into() })),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, Json(Health { status: "unavailable".into() })),
    }
}

#[derive(OpenApi)]
#[openapi(
    info(title = "PageRune", version = "0.1.0", description = "Implemented scaffold endpoints only. Planned business APIs are documented in docs/Api.md."),
    paths(healthz, readyz),
    components(schemas(Health))
)]
pub struct ApiDoc;

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .merge(SwaggerUi::new("/swagger-ui").url("/api/openapi.json", ApiDoc::openapi()))
        .with_state(state)
}
