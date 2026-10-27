use crate::{AppState, fetcher::normalize_url};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Serialize, ToSchema, FromRow)]
pub struct Site {
    pub id: Uuid,
    pub original_url: String,
    pub normalized_url: String,
    pub interval_seconds: i32,
    pub current_content_hash: Option<String>,
    pub last_checked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_successful_check_at: Option<chrono::DateTime<chrono::Utc>>,
    pub current_status: String,
    pub last_error: Option<String>,
    pub next_check_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub public_subscriptions_enabled: bool,
    #[sqlx(skip)]
    pub subscription_url: String,
}

#[derive(Serialize, ToSchema, FromRow)]
pub struct Subscription {
    pub id: Uuid,
    pub site_id: Uuid,
    pub subscriber_identity: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub status: String,
}

#[derive(Serialize, ToSchema, FromRow)]
pub struct Change {
    pub id: Uuid,
    pub site_id: Uuid,
    pub previous_content_hash: String,
    pub new_content_hash: String,
    pub detected_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, ToSchema)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: i64,
    pub limit: i64,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateSite {
    pub url: String,
    #[serde(default = "default_interval")]
    pub interval_seconds: i32,
    #[serde(default)]
    pub public_subscriptions_enabled: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateSite {
    pub interval_seconds: Option<i32>,
    pub public_subscriptions_enabled: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateSubscription {
    pub subscriber_identity: String,
}

#[derive(Deserialize)]
pub struct Pagination {
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct SubscriberFilter {
    pub subscriber_identity: String,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
    pub request_id: String,
}

pub struct ApiError(StatusCode, &'static str, &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let id = Uuid::new_v4().to_string();
        (
            self.0,
            Json(ApiErrorBody {
                code: self.1.into(),
                message: self.2.into(),
                request_id: id,
            }),
        )
            .into_response()
    }
}

fn default_interval() -> i32 {
    86_400
}
fn set_subscription_url(site: &mut Site, base: &str) {
    site.subscription_url = format!("{}/subscribe/{}", base.trim_end_matches('/'), site.id);
}
fn pagination(p: &Pagination) -> Result<(i64, i64, i64), ApiError> {
    let page = p.page.unwrap_or(1);
    let limit = p.limit.unwrap_or(50);
    if page < 1 || !(1..=100).contains(&limit) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_pagination",
            "Pagination values are invalid",
        ));
    }
    let offset = (page - 1).checked_mul(limit).ok_or(ApiError(
        StatusCode::BAD_REQUEST,
        "invalid_pagination",
        "Pagination values are invalid",
    ))?;
    Ok((page, limit, offset))
}
fn validate_interval(interval: i32) -> Result<(), ApiError> {
    if !(300..=2_592_000).contains(&interval) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_interval",
            "Interval must be between 300 and 2592000 seconds",
        ));
    }
    Ok(())
}
fn validate_identity(identity: &str) -> Result<(), ApiError> {
    if identity.trim().is_empty() || identity.len() > 256 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_subscriber_identity",
            "Subscriber identity must contain 1 to 256 characters",
        ));
    }
    Ok(())
}
fn db_error() -> ApiError {
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "Request could not be completed",
    )
}

#[utoipa::path(post, path = "/api/v1/sites", tag = "Sites", request_body = CreateSite,
    responses((status = 200, description = "Equivalent site already exists", body = Site), (status = 201, body = Site), (status = 400, body = ApiErrorBody), (status = 503, body = ApiErrorBody)))]
pub async fn create_site(
    State(state): State<AppState>,
    Json(input): Json<CreateSite>,
) -> Result<(StatusCode, Json<Site>), ApiError> {
    validate_interval(input.interval_seconds)?;
    let normalized = normalize_url(&input.url).map_err(|_| {
        ApiError(
            StatusCode::BAD_REQUEST,
            "invalid_url",
            "URL must be an HTTP or HTTPS URL without credentials",
        )
    })?;
    let id = Uuid::new_v4();
    let mut tx = state.database.begin().await.map_err(|_| db_error())?;
    let inserted = sqlx::query_as::<_, Site>(
        "INSERT INTO monitored_sites (id, original_url, normalized_url, interval_seconds, public_subscriptions_enabled) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (normalized_url) DO NOTHING RETURNING id, original_url, normalized_url, interval_seconds, current_content_hash, last_checked_at, last_successful_check_at, current_status, last_error, next_check_at, created_at, public_subscriptions_enabled",
    )
    .bind(id)
    .bind(input.url)
    .bind(&normalized)
    .bind(input.interval_seconds)
    .bind(input.public_subscriptions_enabled)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| db_error())?;
    let (status, mut site) = if let Some(site) = inserted {
        (StatusCode::CREATED, site)
    } else {
        let site = sqlx::query_as::<_, Site>(
            "SELECT id, original_url, normalized_url, interval_seconds, current_content_hash, last_checked_at, last_successful_check_at, current_status, last_error, next_check_at, created_at, public_subscriptions_enabled FROM monitored_sites WHERE normalized_url=$1",
        )
        .bind(normalized)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| db_error())?
        .ok_or_else(db_error)?;
        (StatusCode::OK, site)
    };
    tx.commit().await.map_err(|_| db_error())?;
    set_subscription_url(&mut site, &state.public_base_url);
    Ok((status, Json(site)))
}

#[utoipa::path(get, path = "/api/v1/sites", tag = "Sites", params(("page" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = Page<Site>), (status = 400, body = ApiErrorBody), (status = 503, body = ApiErrorBody)))]
pub async fn list_sites(
    State(state): State<AppState>,
    Query(query): Query<Pagination>,
) -> Result<Json<Page<Site>>, ApiError> {
    let (page, limit, offset) = pagination(&query)?;
    let mut items = sqlx::query_as::<_, Site>("SELECT id, original_url, normalized_url, interval_seconds, current_content_hash, last_checked_at, last_successful_check_at, current_status, last_error, next_check_at, created_at, public_subscriptions_enabled FROM monitored_sites ORDER BY created_at DESC, id LIMIT $1 OFFSET $2")
        .bind(limit).bind(offset).fetch_all(&state.database).await.map_err(|_| db_error())?;
    for site in &mut items {
        set_subscription_url(site, &state.public_base_url);
    }
    Ok(Json(Page { items, page, limit }))
}

#[utoipa::path(get, path = "/api/v1/sites/{id}", tag = "Sites", params(("id" = Uuid, Path)), responses((status = 200, body = Site), (status = 404, body = ApiErrorBody), (status = 503, body = ApiErrorBody)))]
pub async fn get_site(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Site>, ApiError> {
    let mut site = sqlx::query_as::<_, Site>("SELECT id, original_url, normalized_url, interval_seconds, current_content_hash, last_checked_at, last_successful_check_at, current_status, last_error, next_check_at, created_at, public_subscriptions_enabled FROM monitored_sites WHERE id=$1")
        .bind(id).fetch_optional(&state.database).await.map_err(|_| db_error())?.ok_or(ApiError(StatusCode::NOT_FOUND, "not_found", "Site not found"))?;
    set_subscription_url(&mut site, &state.public_base_url);
    Ok(Json(site))
}

#[utoipa::path(patch, path = "/api/v1/sites/{id}", tag = "Sites", request_body = UpdateSite, params(("id" = Uuid, Path)), responses((status = 200, body = Site), (status = 400, body = ApiErrorBody), (status = 404, body = ApiErrorBody), (status = 503, body = ApiErrorBody)))]
pub async fn update_site(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateSite>,
) -> Result<Json<Site>, ApiError> {
    if input.interval_seconds.is_none() && input.public_subscriptions_enabled.is_none() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "empty_update",
            "At least one site setting must be provided",
        ));
    }
    if let Some(interval) = input.interval_seconds {
        validate_interval(interval)?;
    }
    let mut site = sqlx::query_as::<_, Site>("UPDATE monitored_sites SET interval_seconds=COALESCE($2,interval_seconds), public_subscriptions_enabled=COALESCE($3,public_subscriptions_enabled), next_check_at=CASE WHEN $2 IS NULL THEN next_check_at ELSE LEAST(next_check_at,now()) END, updated_at=now() WHERE id=$1 RETURNING id, original_url, normalized_url, interval_seconds, current_content_hash, last_checked_at, last_successful_check_at, current_status, last_error, next_check_at, created_at, public_subscriptions_enabled")
        .bind(id).bind(input.interval_seconds).bind(input.public_subscriptions_enabled).fetch_optional(&state.database).await.map_err(|_| db_error())?.ok_or(ApiError(StatusCode::NOT_FOUND, "not_found", "Site not found"))?;
    set_subscription_url(&mut site, &state.public_base_url);
    Ok(Json(site))
}

#[utoipa::path(delete, path = "/api/v1/sites/{id}", tag = "Sites", params(("id" = Uuid, Path)), responses((status = 204, description = "Site deleted"), (status = 404, body = ApiErrorBody), (status = 503, body = ApiErrorBody)))]
pub async fn delete_site(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query("DELETE FROM monitored_sites WHERE id=$1")
        .bind(id)
        .execute(&state.database)
        .await
        .map_err(|_| db_error())?;
    if result.rows_affected() == 0 {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "not_found",
            "Site not found",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/sites/{site_id}/subscriptions", tag = "Subscriptions", request_body = CreateSubscription, params(("site_id" = Uuid, Path)), responses((status = 202, body = Subscription), (status = 400, body = ApiErrorBody), (status = 404, body = ApiErrorBody), (status = 503, body = ApiErrorBody)))]
pub async fn create_subscription(
    State(state): State<AppState>,
    Path(site_id): Path<Uuid>,
    Json(input): Json<CreateSubscription>,
) -> Result<(StatusCode, Json<Subscription>), ApiError> {
    let id = crate::web::create_pending_api(&state, site_id, &input.subscriber_identity)
        .await
        .map_err(|code| {
            if code == "invalid_email" {
                ApiError(
                    StatusCode::BAD_REQUEST,
                    "invalid_email",
                    "A valid email address is required",
                )
            } else if code == "site_not_found" {
                ApiError(StatusCode::NOT_FOUND, "not_found", "Site not found")
            } else {
                db_error()
            }
        })?;
    let subscription = sqlx::query_as::<_, Subscription>(
        "SELECT id,site_id,subscriber_identity,created_at,status FROM subscriptions WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&state.database)
    .await
    .map_err(|_| db_error())?;
    Ok((StatusCode::ACCEPTED, Json(subscription)))
}

#[utoipa::path(get, path = "/api/v1/subscriptions", tag = "Subscriptions", params(("subscriber_identity" = String, Query), ("page" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = Page<Subscription>), (status = 400, body = ApiErrorBody), (status = 503, body = ApiErrorBody)))]
pub async fn list_subscriptions(
    State(state): State<AppState>,
    Query(query): Query<SubscriberFilter>,
) -> Result<Json<Page<Subscription>>, ApiError> {
    validate_identity(&query.subscriber_identity)?;
    let (page, limit, offset) = pagination(&Pagination {
        page: query.page,
        limit: query.limit,
    })?;
    let items = sqlx::query_as::<_, Subscription>("SELECT id, site_id, subscriber_identity, created_at,status FROM subscriptions WHERE subscriber_identity=$1 ORDER BY created_at DESC, id LIMIT $2 OFFSET $3")
        .bind(query.subscriber_identity).bind(limit).bind(offset).fetch_all(&state.database).await.map_err(|_| db_error())?;
    Ok(Json(Page { items, page, limit }))
}

#[utoipa::path(get, path = "/api/v1/subscriptions/{id}", tag = "Subscriptions", params(("id" = Uuid, Path), ("subscriber_identity" = String, Query)), responses((status = 200, body = Subscription), (status = 400, body = ApiErrorBody), (status = 404, body = ApiErrorBody), (status = 503, body = ApiErrorBody)))]
pub async fn get_subscription(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<IdentityQuery>,
) -> Result<Json<Subscription>, ApiError> {
    validate_identity(&query.subscriber_identity)?;
    sqlx::query_as::<_, Subscription>("SELECT id, site_id, subscriber_identity, created_at,status FROM subscriptions WHERE id=$1 AND subscriber_identity=$2")
        .bind(id).bind(query.subscriber_identity).fetch_optional(&state.database).await.map_err(|_| db_error())?.map(Json).ok_or(ApiError(StatusCode::NOT_FOUND, "not_found", "Subscription not found"))
}

#[derive(Deserialize)]
pub struct IdentityQuery {
    pub subscriber_identity: String,
}

#[utoipa::path(delete, path = "/api/v1/subscriptions/{id}", tag = "Subscriptions", params(("id" = Uuid, Path), ("subscriber_identity" = String, Query)), responses((status = 204, description = "Subscription deleted"), (status = 400, body = ApiErrorBody), (status = 404, body = ApiErrorBody), (status = 503, body = ApiErrorBody)))]
pub async fn delete_subscription(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<IdentityQuery>,
) -> Result<StatusCode, ApiError> {
    validate_identity(&query.subscriber_identity)?;
    let result = sqlx::query("DELETE FROM subscriptions WHERE id=$1 AND subscriber_identity=$2")
        .bind(id)
        .bind(query.subscriber_identity)
        .execute(&state.database)
        .await
        .map_err(|_| db_error())?;
    if result.rows_affected() == 0 {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "not_found",
            "Subscription not found",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v1/sites/{site_id}/changes", tag = "Changes", params(("site_id" = Uuid, Path), ("page" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = Page<Change>), (status = 400, body = ApiErrorBody), (status = 404, body = ApiErrorBody), (status = 503, body = ApiErrorBody)))]
pub async fn list_changes(
    State(state): State<AppState>,
    Path(site_id): Path<Uuid>,
    Query(query): Query<Pagination>,
) -> Result<Json<Page<Change>>, ApiError> {
    let (page, limit, offset) = pagination(&query)?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM monitored_sites WHERE id=$1)")
            .bind(site_id)
            .fetch_one(&state.database)
            .await
            .map_err(|_| db_error())?;
    if !exists {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "not_found",
            "Site not found",
        ));
    }
    let items = sqlx::query_as::<_, Change>("SELECT id, site_id, previous_content_hash, new_content_hash, detected_at FROM site_changes WHERE site_id=$1 ORDER BY detected_at DESC, id DESC LIMIT $2 OFFSET $3")
        .bind(site_id).bind(limit).bind(offset).fetch_all(&state.database).await.map_err(|_| db_error())?;
    Ok(Json(Page { items, page, limit }))
}

#[utoipa::path(get, path = "/api/v1/changes/{id}", tag = "Changes", params(("id" = Uuid, Path)), responses((status = 200, body = Change), (status = 404, body = ApiErrorBody), (status = 503, body = ApiErrorBody)))]
pub async fn get_change(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Change>, ApiError> {
    sqlx::query_as::<_, Change>("SELECT id, site_id, previous_content_hash, new_content_hash, detected_at FROM site_changes WHERE id=$1")
        .bind(id).fetch_optional(&state.database).await.map_err(|_| db_error())?.map(Json).ok_or(ApiError(StatusCode::NOT_FOUND, "not_found", "Change not found"))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/sites", get(list_sites).post(create_site))
        .route(
            "/api/v1/sites/{id}",
            get(get_site).patch(update_site).delete(delete_site),
        )
        .route(
            "/api/v1/sites/{site_id}/subscriptions",
            post(create_subscription),
        )
        .route("/api/v1/subscriptions", get(list_subscriptions))
        .route(
            "/api/v1/subscriptions/{id}",
            get(get_subscription).delete(delete_subscription),
        )
        .route("/api/v1/sites/{site_id}/changes", get(list_changes))
        .route("/api/v1/changes/{id}", get(get_change))
}
