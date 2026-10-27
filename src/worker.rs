use crate::{
    AppState,
    fetcher::{FetchConfig, fetch},
};
use sqlx::{FromRow, Postgres, Transaction};
use std::time::Duration;
use uuid::Uuid;

#[derive(FromRow)]
struct Job {
    id: Uuid,
    url: String,
    etag: Option<String>,
    last_modified: Option<String>,
    interval_seconds: i32,
    lease_owner: Uuid,
    lease_generation: i64,
}

pub async fn run(state: AppState, fetch_config: FetchConfig) {
    loop {
        match claim(&state.database).await {
            Ok(Some(job)) => process(&state, &fetch_config, job).await,
            Ok(None) => tokio::time::sleep(Duration::from_millis(750)).await,
            Err(error) => {
                tracing::warn!(error = %error, "scheduler could not claim work");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn claim(pool: &sqlx::PgPool) -> Result<Option<Job>, sqlx::Error> {
    let owner = Uuid::new_v4();
    sqlx::query_as::<_, Job>(
        "WITH due AS (SELECT id FROM monitored_sites WHERE next_check_at <= now() AND (lease_expires_at IS NULL OR lease_expires_at <= now()) ORDER BY next_check_at, id FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE monitored_sites s SET lease_owner=$1, lease_expires_at=now() + interval '180 seconds', lease_generation=s.lease_generation+1, updated_at=now() FROM due WHERE s.id=due.id RETURNING s.id, s.normalized_url AS url, s.etag, s.last_modified, s.interval_seconds, s.lease_owner, s.lease_generation"
    ).bind(owner).fetch_optional(pool).await
}

async fn process(state: &AppState, config: &FetchConfig, job: Job) {
    let result = fetch(
        &job.url,
        job.etag.as_deref(),
        job.last_modified.as_deref(),
        config,
    )
    .await;
    if let Err(error) = apply_result(state, &job, result).await {
        tracing::warn!(site_id = %job.id, error = %error, "site result could not be committed");
    }
}

async fn apply_result(
    state: &AppState,
    job: &Job,
    result: Result<crate::fetcher::FetchResult, crate::fetcher::FetchError>,
) -> Result<(), sqlx::Error> {
    let mut tx = state.database.begin().await?;
    let previous: Option<String> = sqlx::query_scalar(
        "SELECT current_content_hash FROM monitored_sites WHERE id=$1 AND lease_owner=$2 AND lease_generation=$3 AND lease_expires_at > now() FOR UPDATE"
    ).bind(job.id).bind(job.lease_owner).bind(job.lease_generation).fetch_optional(&mut *tx).await?.flatten();
    let owns_lease: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM monitored_sites WHERE id=$1 AND lease_owner=$2 AND lease_generation=$3 AND lease_expires_at > now())"
    ).bind(job.id).bind(job.lease_owner).bind(job.lease_generation).fetch_one(&mut *tx).await?;
    if !owns_lease {
        tx.rollback().await?;
        return Ok(());
    }

    match result {
        Ok(result) => {
            let new_hash = if result.not_modified {
                previous.clone()
            } else {
                result.hash.clone()
            };
            if new_hash.is_none() {
                fail_site(&mut tx, job, "not_modified_without_baseline").await?;
            } else {
                if let (Some(previous), Some(new_hash)) = (previous.as_deref(), new_hash.as_deref())
                    && previous != new_hash
                {
                    let change_id = Uuid::new_v4();
                    sqlx::query("INSERT INTO site_changes (id, site_id, previous_content_hash, new_content_hash) VALUES ($1,$2,$3,$4)")
                        .bind(change_id).bind(job.id).bind(previous).bind(new_hash).execute(&mut *tx).await?;
                    let created = sqlx::query("INSERT INTO notification_outbox (id, change_id, subscription_id, recipient) SELECT gen_random_uuid(), $1, s.id, s.subscriber_identity FROM subscriptions s WHERE s.site_id=$2 ON CONFLICT (change_id, subscription_id) DO NOTHING")
                        .bind(change_id).bind(job.id).execute(&mut *tx).await?.rows_affected();
                    tracing::info!(change_id = %change_id, site_id = %job.id, notification_count = created, "change and notifications created");
                }
                sqlx::query("UPDATE monitored_sites SET current_content_hash=$4, etag=COALESCE($5, etag), last_modified=COALESCE($6, last_modified), last_checked_at=now(), last_successful_check_at=now(), current_status='ok', last_error=NULL, next_check_at=now() + ($7 * interval '1 second'), lease_owner=NULL, lease_expires_at=NULL, updated_at=now() WHERE id=$1 AND lease_owner=$2 AND lease_generation=$3")
                    .bind(job.id).bind(job.lease_owner).bind(job.lease_generation).bind(new_hash).bind(result.etag).bind(result.last_modified).bind(job.interval_seconds).execute(&mut *tx).await?;
            }
        }
        Err(error) => fail_site(&mut tx, job, fetch_error_code(&error)).await?,
    }
    tx.commit().await
}

async fn fail_site(
    tx: &mut Transaction<'_, Postgres>,
    job: &Job,
    code: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE monitored_sites SET last_checked_at=now(), current_status='error', last_error=$4, next_check_at=now() + LEAST($5, 300) * interval '1 second', lease_owner=NULL, lease_expires_at=NULL, updated_at=now() WHERE id=$1 AND lease_owner=$2 AND lease_generation=$3")
        .bind(job.id).bind(job.lease_owner).bind(job.lease_generation).bind(code).bind(job.interval_seconds).execute(&mut **tx).await?;
    Ok(())
}

fn fetch_error_code(error: &crate::fetcher::FetchError) -> &'static str {
    use crate::fetcher::FetchError::*;
    match error {
        UnsafeUrl | Resolution => "destination_not_allowed",
        Request => "request_failed",
        HttpStatus => "http_error",
        TooLarge => "response_too_large",
        RedirectLimit => "redirect_limit",
        InvalidRedirect => "invalid_redirect",
        InvalidHeader => "invalid_response_header",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use tokio::sync::OnceCell;

    static MIGRATIONS: OnceCell<()> = OnceCell::const_new();

    async fn database() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL is required for worker integration tests");
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("PostgreSQL must be reachable");
        MIGRATIONS
            .get_or_init(|| async {
                sqlx::migrate!()
                    .run(&pool)
                    .await
                    .expect("migrations must apply");
            })
            .await;
        pool
    }

    #[test]
    fn error_reporting_uses_stable_codes() {
        assert_eq!(
            fetch_error_code(&crate::fetcher::FetchError::TooLarge),
            "response_too_large"
        );
    }

    #[tokio::test]
    async fn postgres_baseline_unchanged_change_and_fencing_flow() {
        let pool = database().await;
        let site_id = Uuid::new_v4();
        let url = format!("https://example.com/worker-test/{site_id}");
        sqlx::query("UPDATE monitored_sites SET next_check_at=now()+interval '1 day', lease_expires_at=NULL")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO monitored_sites (id, original_url, normalized_url, next_check_at) VALUES ($1,$2,$2,now())")
            .bind(site_id).bind(&url).execute(&pool).await.unwrap();
        let subscription_ids = [Uuid::new_v4(), Uuid::new_v4()];
        for (id, recipient) in subscription_ids
            .into_iter()
            .zip(["alice@example.test", "bob@example.test"])
        {
            sqlx::query(
                "INSERT INTO subscriptions (id,site_id,subscriber_identity) VALUES ($1,$2,$3)",
            )
            .bind(id)
            .bind(site_id)
            .bind(recipient)
            .execute(&pool)
            .await
            .unwrap();
        }
        let state = AppState {
            database: pool.clone(),
            dev_api_enabled: false,
            public_base_url: "http://localhost:8080".into(),
        };

        let (left, right) = tokio::join!(claim(&pool), claim(&pool));
        let first = left
            .unwrap()
            .or(right.unwrap())
            .expect("one worker claims the site");
        assert_eq!(first.id, site_id);
        apply_result(
            &state,
            &first,
            Ok(crate::fetcher::FetchResult {
                hash: Some("a".repeat(64)),
                etag: None,
                last_modified: None,
                not_modified: false,
            }),
        )
        .await
        .unwrap();
        let changes: i64 = sqlx::query_scalar("SELECT count(*) FROM site_changes WHERE site_id=$1")
            .bind(site_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(changes, 0, "initial check establishes only a baseline");
        let notifications: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM notification_outbox WHERE subscription_id=ANY($1)",
        )
        .bind(&subscription_ids[..])
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(notifications, 0, "baseline does not enqueue notifications");

        sqlx::query("UPDATE monitored_sites SET next_check_at=now() WHERE id=$1")
            .bind(site_id)
            .execute(&pool)
            .await
            .unwrap();
        let unchanged = claim(&pool).await.unwrap().expect("site is due");
        assert_eq!(unchanged.id, site_id);
        apply_result(
            &state,
            &unchanged,
            Ok(crate::fetcher::FetchResult {
                hash: Some("a".repeat(64)),
                etag: None,
                last_modified: None,
                not_modified: false,
            }),
        )
        .await
        .unwrap();
        let changes: i64 = sqlx::query_scalar("SELECT count(*) FROM site_changes WHERE site_id=$1")
            .bind(site_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(changes, 0, "unchanged content creates no event");
        let notifications: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM notification_outbox WHERE subscription_id=ANY($1)",
        )
        .bind(&subscription_ids[..])
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            notifications, 0,
            "unchanged content does not enqueue notifications"
        );

        sqlx::query("UPDATE monitored_sites SET next_check_at=now() WHERE id=$1")
            .bind(site_id)
            .execute(&pool)
            .await
            .unwrap();
        let stale = claim(&pool).await.unwrap().expect("site is due");
        sqlx::query(
            "UPDATE monitored_sites SET lease_expires_at=now() - interval '1 second' WHERE id=$1",
        )
        .bind(site_id)
        .execute(&pool)
        .await
        .unwrap();
        let current = claim(&pool)
            .await
            .unwrap()
            .expect("expired lease is recoverable");
        assert!(current.lease_generation > stale.lease_generation);
        apply_result(
            &state,
            &stale,
            Ok(crate::fetcher::FetchResult {
                hash: Some("b".repeat(64)),
                etag: None,
                last_modified: None,
                not_modified: false,
            }),
        )
        .await
        .unwrap();
        let still_baseline: Option<String> =
            sqlx::query_scalar("SELECT current_content_hash FROM monitored_sites WHERE id=$1")
                .bind(site_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            still_baseline.as_deref(),
            Some("a".repeat(64).as_str()),
            "stale worker cannot overwrite newer lease"
        );
        apply_result(
            &state,
            &current,
            Ok(crate::fetcher::FetchResult {
                hash: Some("c".repeat(64)),
                etag: None,
                last_modified: None,
                not_modified: false,
            }),
        )
        .await
        .unwrap();
        apply_result(
            &state,
            &current,
            Ok(crate::fetcher::FetchResult {
                hash: Some("c".repeat(64)),
                etag: None,
                last_modified: None,
                not_modified: false,
            }),
        )
        .await
        .unwrap();
        let changes: Vec<(String, String)> = sqlx::query_as(
            "SELECT previous_content_hash, new_content_hash FROM site_changes WHERE site_id=$1",
        )
        .bind(site_id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(changes, vec![("a".repeat(64), "c".repeat(64))]);
        let notifications: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM notification_outbox WHERE subscription_id=ANY($1)",
        )
        .bind(&subscription_ids[..])
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(notifications, 2, "one notification per active subscriber");
        let late_subscription = Uuid::new_v4();
        sqlx::query("INSERT INTO subscriptions (id,site_id,subscriber_identity) VALUES ($1,$2,'late@example.test')")
            .bind(late_subscription).bind(site_id).execute(&pool).await.unwrap();
        let historical: i64 =
            sqlx::query_scalar("SELECT count(*) FROM notification_outbox WHERE subscription_id=$1")
                .bind(late_subscription)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            historical, 0,
            "new subscriptions do not receive historical notifications"
        );
        sqlx::query("DELETE FROM subscriptions WHERE id=$1")
            .bind(subscription_ids[0])
            .execute(&pool)
            .await
            .unwrap();
        let notifications: i64 =
            sqlx::query_scalar("SELECT count(*) FROM notification_outbox WHERE subscription_id=$1")
                .bind(subscription_ids[0])
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            notifications, 0,
            "deleting a subscription suppresses its queued rows"
        );

        sqlx::query("UPDATE monitored_sites SET next_check_at=now() WHERE id=$1")
            .bind(site_id)
            .execute(&pool)
            .await
            .unwrap();
        let failed = claim(&pool).await.unwrap().expect("site is due");
        apply_result(&state, &failed, Err(crate::fetcher::FetchError::HttpStatus))
            .await
            .unwrap();
        let (hash, status, error): (Option<String>, String, Option<String>) = sqlx::query_as("SELECT current_content_hash, current_status, last_error FROM monitored_sites WHERE id=$1").bind(site_id).fetch_one(&pool).await.unwrap();
        assert_eq!(hash.as_deref(), Some("c".repeat(64).as_str()));
        assert_eq!(status, "error");
        assert_eq!(error.as_deref(), Some("http_error"));
        sqlx::query("DELETE FROM monitored_sites WHERE id=$1")
            .bind(site_id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
