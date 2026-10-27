use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart, SinglePart, header::ContentType},
    transport::smtp::{authentication::Credentials, client::Tls},
};
use rand::Rng;
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use std::{env, time::Duration};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub tls_mode: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from_email: String,
    pub from_name: String,
    pub timeout: Duration,
    pub public_base_url: String,
    pub template_dir: String,
}

impl std::fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("tls_mode", &self.tls_mode)
            .field("authentication_configured", &self.username.is_some())
            .field("from_email", &self.from_email)
            .field("from_name", &self.from_name)
            .field("timeout", &self.timeout)
            .field("template_dir", &self.template_dir)
            .finish()
    }
}

impl Default for SmtpConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 1025,
            tls_mode: "none".into(),
            username: None,
            password: None,
            from_email: "updates@localhost".into(),
            from_name: "PageRune".into(),
            timeout: Duration::from_secs(15),
            public_base_url: env::var("PAGERUNE_PUBLIC_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:8080".into()),
            template_dir: env::var("PAGERUNE_EMAIL_TEMPLATE_DIR")
                .unwrap_or_else(|_| "templates/email".into()),
        }
    }
}

impl SmtpConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let mut c = Self::default();
        c.host = env::var("SMTP_HOST").unwrap_or(c.host);
        c.port = env::var("SMTP_PORT")
            .ok()
            .map(|s| s.parse())
            .transpose()
            .map_err(|_| anyhow::anyhow!("SMTP_PORT must be a valid port"))?
            .unwrap_or(c.port);
        c.tls_mode = env::var("SMTP_TLS_MODE").unwrap_or(c.tls_mode);
        c.username = env::var("SMTP_USERNAME").ok().filter(|s| !s.is_empty());
        c.password = env::var("SMTP_PASSWORD").ok().filter(|s| !s.is_empty());
        c.from_email = env::var("SMTP_FROM_EMAIL").unwrap_or(c.from_email);
        c.from_name = env::var("SMTP_FROM_NAME").unwrap_or(c.from_name);
        c.timeout = Duration::from_secs(
            env::var("SMTP_TIMEOUT_SECONDS")
                .ok()
                .map(|s| s.parse())
                .transpose()
                .map_err(|_| anyhow::anyhow!("SMTP_TIMEOUT_SECONDS must be an integer"))?
                .unwrap_or(15),
        );
        if c.host.is_empty()
            || c.port == 0
            || !matches!(c.tls_mode.as_str(), "none" | "starttls" | "implicit")
            || c.timeout.is_zero()
            || c.timeout > Duration::from_secs(120)
        {
            anyhow::bail!("SMTP configuration is invalid");
        }
        if c.username.is_some() != c.password.is_some() {
            anyhow::bail!("SMTP_USERNAME and SMTP_PASSWORD must be configured together");
        }
        c.from_email
            .parse::<lettre::Address>()
            .map_err(|_| anyhow::anyhow!("SMTP_FROM_EMAIL is invalid"))?;
        Ok(c)
    }
}

#[derive(FromRow, Clone)]
struct Delivery {
    id: Uuid,
    change_id: Uuid,
    subscription_id: Uuid,
    recipient: String,
    site_url: String,
    detected_at: chrono::DateTime<chrono::Utc>,
    lease_owner: Uuid,
    lease_generation: i64,
    attempt_count: i32,
    recovered: bool,
}

pub async fn run(
    pool: sqlx::PgPool,
    smtp: SmtpConfig,
    concurrency: usize,
    max_attempts: i32,
    cancel: CancellationToken,
) {
    let mut tasks = JoinSet::new();
    loop {
        while tasks.len() < concurrency {
            if cancel.is_cancelled() {
                while tasks.join_next().await.is_some() {}
                return;
            }
            match claim(&pool).await {
                Ok(Some(job)) => {
                    tracing::info!(notification_id=%job.id, change_id=%job.change_id, attempt=job.attempt_count, lease_recovery=job.recovered, "notification claimed");
                    let (pool, smtp) = (pool.clone(), smtp.clone());
                    tasks.spawn(async move { deliver(pool, smtp, job, max_attempts).await });
                }
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!(error=%e, "notification claim failed");
                    break;
                }
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => { while tasks.join_next().await.is_some() {} return; },
            Some(result) = tasks.join_next(), if !tasks.is_empty() => { if let Err(e)=result { tracing::warn!(error=%e, "notification task failed"); } },
            _ = tokio::time::sleep(Duration::from_millis(500)), if tasks.is_empty() => {}
        }
    }
}

async fn claim(pool: &sqlx::PgPool) -> Result<Option<Delivery>, sqlx::Error> {
    claim_for_site(pool, None).await
}

async fn claim_for_site(
    pool: &sqlx::PgPool,
    site_id: Option<Uuid>,
) -> Result<Option<Delivery>, sqlx::Error> {
    let owner = Uuid::new_v4();
    sqlx::query_as::<_,Delivery>("WITH due AS (SELECT o.id, (o.status='sending') AS recovered FROM notification_outbox o JOIN subscriptions s ON s.id=o.subscription_id JOIN site_changes c ON c.id=o.change_id WHERE o.status IN ('pending','sending') AND o.next_attempt_at<=now() AND (o.lease_expires_at IS NULL OR o.lease_expires_at<=now()) AND ($2::uuid IS NULL OR c.site_id=$2) ORDER BY o.next_attempt_at,o.created_at FOR UPDATE OF o SKIP LOCKED LIMIT 1) UPDATE notification_outbox o SET status='sending', lease_owner=$1, lease_expires_at=now()+interval '180 seconds', lease_generation=o.lease_generation+1, attempt_count=o.attempt_count+1 FROM due d,site_changes c,monitored_sites m WHERE o.id=d.id AND c.id=o.change_id AND m.id=c.site_id RETURNING o.id,o.change_id,o.subscription_id,o.recipient,c.detected_at,m.original_url AS site_url,o.lease_owner,o.lease_generation,o.attempt_count,d.recovered")
        .bind(owner).bind(site_id).fetch_optional(pool).await
}

async fn deliver(pool: sqlx::PgPool, config: SmtpConfig, job: Delivery, max_attempts: i32) {
    tracing::info!(notification_id=%job.id, change_id=%job.change_id, attempt=job.attempt_count, "SMTP delivery attempt");
    let active =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM subscriptions WHERE id=$1)")
            .bind(job.subscription_id)
            .fetch_one(&pool)
            .await
            .unwrap_or(false);
    if !active {
        return;
    }
    let result = send(&pool, &config, &job).await;
    match result {
        Ok(()) => {
            let done=sqlx::query("UPDATE notification_outbox SET status='delivered',delivered_at=now(),lease_owner=NULL,lease_expires_at=NULL,last_error=NULL WHERE id=$1 AND status='sending' AND lease_owner=$2 AND lease_generation=$3 AND lease_expires_at>now()")
                .bind(job.id).bind(job.lease_owner).bind(job.lease_generation).execute(&pool).await;
            if matches!(done,Ok(r) if r.rows_affected()==1) {
                tracing::info!(notification_id=%job.id,change_id=%job.change_id,"notification delivered");
            }
        }
        Err((error_code, permanent)) => {
            let terminal = permanent || job.attempt_count >= max_attempts;
            let delay = Duration::from_secs(
                (2u64.saturating_pow(job.attempt_count.min(12) as u32)).min(21_600),
            );
            let jitter = rand_jitter(job.id, job.attempt_count, delay.as_secs()).min(21_600);
            let status = if terminal { "failed" } else { "pending" };
            let res=sqlx::query("UPDATE notification_outbox SET status=$4,next_attempt_at=now()+($5 * interval '1 second'),lease_owner=NULL,lease_expires_at=NULL,last_error=$6 WHERE id=$1 AND status='sending' AND lease_owner=$2 AND lease_generation=$3 AND lease_expires_at>now()")
                .bind(job.id).bind(job.lease_owner).bind(job.lease_generation).bind(status).bind(jitter as i64).bind(error_code).execute(&pool).await;
            if matches!(res,Ok(r) if r.rows_affected()==1) {
                if terminal {
                    tracing::error!(notification_id=%job.id,change_id=%job.change_id,error_code,"notification permanently failed");
                } else {
                    tracing::warn!(notification_id=%job.id,change_id=%job.change_id,retry_seconds=jitter,error_code,"notification retry scheduled");
                }
            }
        }
    }
}

fn rand_jitter(id: Uuid, attempt: i32, base: u64) -> u64 {
    let bytes = id.as_bytes();
    base.saturating_mul(75 + ((bytes[0] as u64 + attempt as u64) % 51)) / 100
}

async fn send(
    pool: &sqlx::PgPool,
    c: &SmtpConfig,
    j: &Delivery,
) -> Result<(), (&'static str, bool)> {
    let from: Mailbox = format!("{} <{}>", c.from_name, c.from_email)
        .parse()
        .map_err(|_| ("invalid_sender_address", true))?;
    let to: Mailbox = j
        .recipient
        .parse()
        .map_err(|_| ("invalid_recipient_address", true))?;
    let mut token_bytes = [0u8; 32];
    rand::rng().fill(&mut token_bytes);
    let token = hex::encode(token_bytes);
    let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
    sqlx::query("INSERT INTO unsubscribe_tokens(token_hash,subscription_id) VALUES($1,$2)")
        .bind(&token_hash)
        .bind(j.subscription_id)
        .execute(pool)
        .await
        .map_err(|_| ("unsubscribe_token_store_failed", false))?;
    let unsubscribe_url = format!(
        "{}/unsubscribe/{}",
        c.public_base_url.trim_end_matches('/'),
        token
    );
    let html = templates::render(
        &c.template_dir,
        "change.html",
        j.change_id,
        j.detected_at,
        &j.site_url,
        &unsubscribe_url,
    )
    .map_err(|_| ("email_template_invalid", true))?;
    let text = templates::render(
        &c.template_dir,
        "change.txt",
        j.change_id,
        j.detected_at,
        &j.site_url,
        &unsubscribe_url,
    )
    .map_err(|_| ("email_template_invalid", true))?;
    let email = Message::builder()
        .from(from)
        .to(to)
        .subject("A monitored PageRune website changed")
        .raw_header(lettre::message::header::HeaderValue::new(
            lettre::message::header::HeaderName::new_from_ascii_str("List-Unsubscribe"),
            format!("<{unsubscribe_url}>"),
        ))
        .raw_header(lettre::message::header::HeaderValue::new(
            lettre::message::header::HeaderName::new_from_ascii_str("List-Unsubscribe-Post"),
            "List-Unsubscribe=One-Click".into(),
        ))
        .message_id(Some(format!("<{}@pagerune>", j.id)))
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(text),
                )
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(html),
                ),
        )
        .map_err(|_| ("message_build_failed", true))?;
    let still_active =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM subscriptions WHERE id=$1)")
            .bind(j.subscription_id)
            .fetch_one(pool)
            .await
            .map_err(|_| ("subscription_check_failed", false))?;
    if !still_active {
        return Ok(());
    }
    let mut builder = match c.tls_mode.as_str() {
        "implicit" => AsyncSmtpTransport::<Tokio1Executor>::relay(&c.host)
            .map_err(|_| ("smtp_configuration_failed", false))?,
        "starttls" => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&c.host)
            .map_err(|_| ("smtp_configuration_failed", false))?,
        _ => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&c.host).tls(Tls::None),
    };
    builder = builder.port(c.port).timeout(Some(c.timeout));
    if let (Some(u), Some(p)) = (&c.username, &c.password) {
        builder = builder.credentials(Credentials::new(u.clone(), p.clone()));
    }
    let transport = builder.build();
    tokio::time::timeout(c.timeout, transport.send(email))
        .await
        .map_err(|_| ("smtp_timeout", false))?
        .map(|_| ())
        .map_err(|e| {
            (
                if e.is_permanent() {
                    "smtp_permanent_rejection"
                } else if e.is_transient() {
                    "smtp_transient_rejection"
                } else {
                    "smtp_transport_failure"
                },
                e.is_permanent(),
            )
        })
}

pub mod templates {
    use chrono::{DateTime, Utc};
    use minijinja::{Environment, context};
    use std::fs;
    use uuid::Uuid;
    pub fn render(
        dir: &str,
        name: &str,
        id: Uuid,
        date: DateTime<Utc>,
        url: &str,
        unsubscribe_url: &str,
    ) -> anyhow::Result<String> {
        let fallback = match name {
            "change.html" => include_str!("../templates/email/change.html"),
            "change.txt" => include_str!("../templates/email/change.txt"),
            _ => anyhow::bail!("unknown template"),
        };
        let path = std::path::Path::new(dir).join(name);
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => fallback.to_owned(),
            Err(e) => return Err(e.into()),
        };
        let mut env = Environment::new();
        env.add_template(name, &source)?;
        Ok(env.get_template(name)?.render(context! { site_url => url, site_name => url, detected_at => date.format("%Y-%m-%d %H:%M UTC").to_string(), change_id => id.to_string(), unsubscribe_url => unsubscribe_url, application_name => "PageRune" })?)
    }
}

#[cfg(test)]
mod tests {
    use super::{Delivery, SmtpConfig, claim_for_site, deliver, send, templates};
    use chrono::Utc;
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use tokio::sync::OnceCell;
    use uuid::Uuid;

    static MIGRATIONS: OnceCell<()> = OnceCell::const_new();
    static CLAIM_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn database() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL is required");
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .unwrap();
        MIGRATIONS
            .get_or_init(|| async {
                sqlx::migrate!().run(&pool).await.unwrap();
            })
            .await;
        pool
    }

    #[test]
    fn alternatives_include_brand_source_timestamp_and_stable_change_id() {
        let id = Uuid::new_v4();
        let date = Utc::now();
        let plain = templates::render(
            "templates/email",
            "change.txt",
            id,
            date,
            "https://example.test/page",
            "https://pagerune.test/unsubscribe/abc",
        )
        .unwrap();
        let html = templates::render(
            "templates/email",
            "change.html",
            id,
            date,
            "https://example.test/page",
            "https://pagerune.test/unsubscribe/abc",
        )
        .unwrap();
        for content in [plain, html] {
            assert!(content.contains("PageRune"));
            assert!(content.contains("example.test"), "{content}");
            assert!(content.contains(&date.format("%Y-%m-%d %H:%M UTC").to_string()));
            assert!(content.contains(&id.to_string()));
            assert!(!content.contains("snapshot"));
        }
    }

    #[test]
    fn html_escapes_url_attribute_content() {
        let html = templates::render(
            "templates/email",
            "change.html",
            Uuid::new_v4(),
            Utc::now(),
            "https://example.test/?a=1&b=\"x\"",
            "https://pagerune.test/unsubscribe/a&b",
        )
        .unwrap();
        assert!(html.contains("&amp;"));
        assert!(html.contains("&quot;"));
        assert!(!html.contains("?a=1&b=\"x\""));
    }

    #[test]
    fn missing_template_uses_bundle_and_invalid_template_errors() {
        let id = Uuid::new_v4();
        let date = Utc::now();
        let fallback = templates::render(
            "/definitely/missing/pagerune-templates",
            "change.txt",
            id,
            date,
            "https://example.test",
            "https://pagerune.test/unsubscribe/token",
        )
        .unwrap();
        assert!(fallback.contains("PageRune"));
        let dir = std::env::temp_dir().join(format!("pagerune-template-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("change.txt"), "{% if broken %}").unwrap();
        assert!(
            templates::render(
                dir.to_str().unwrap(),
                "change.txt",
                id,
                date,
                "https://example.test",
                "https://pagerune.test/unsubscribe/token"
            )
            .is_err()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn postgres_claims_are_exclusive_and_fenced_after_expiry() {
        let _guard = CLAIM_TEST_LOCK.lock().await;
        let pool = database().await;
        let site = Uuid::new_v4();
        let sub = Uuid::new_v4();
        sqlx::query("INSERT INTO monitored_sites(id,original_url,normalized_url,next_check_at) VALUES($1,$2,$2,now()+interval '1 day')")
            .bind(site)
            .bind(format!("https://example.test/{site}"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO subscriptions(id,site_id,subscriber_identity) VALUES($1,$2,'test@example.test')").bind(sub).bind(site).execute(&pool).await.unwrap();
        let mut ids = Vec::new();
        for _ in 0..2 {
            let change = Uuid::new_v4();
            sqlx::query("INSERT INTO site_changes(id,site_id,previous_content_hash,new_content_hash) VALUES($1,$2,$3,$4)").bind(change).bind(site).bind("a".repeat(64)).bind("b".repeat(64)).execute(&pool).await.unwrap();
            let notification = Uuid::new_v4();
            sqlx::query("INSERT INTO notification_outbox(id,change_id,subscription_id,recipient) VALUES($1,$2,$3,'test@example.test')").bind(notification).bind(change).bind(sub).execute(&pool).await.unwrap();
            let duplicate=sqlx::query("INSERT INTO notification_outbox(id,change_id,subscription_id,recipient) VALUES($1,$2,$3,'test@example.test') ON CONFLICT(change_id,subscription_id) DO NOTHING").bind(Uuid::new_v4()).bind(change).bind(sub).execute(&pool).await.unwrap();
            assert_eq!(
                duplicate.rows_affected(),
                0,
                "unique change/subscription key prevents duplicate outbox work"
            );
            ids.push(notification);
        }
        let (a, b) = tokio::join!(
            claim_for_site(&pool, Some(site)),
            claim_for_site(&pool, Some(site))
        );
        let a = a.unwrap().unwrap();
        let b = b.unwrap().unwrap();
        assert_ne!(
            a.id, b.id,
            "SKIP LOCKED gives concurrent workers distinct rows"
        );
        sqlx::query(
            "UPDATE notification_outbox SET lease_expires_at=now()-interval '1 second' WHERE id=$1",
        )
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
        let recovered = claim_for_site(&pool, Some(site)).await.unwrap().unwrap();
        assert_eq!(recovered.id, a.id);
        assert!(recovered.lease_generation > a.lease_generation);
        let stale=sqlx::query("UPDATE notification_outbox SET status='delivered' WHERE id=$1 AND lease_owner=$2 AND lease_generation=$3 AND lease_expires_at>now()").bind(a.id).bind(a.lease_owner).bind(a.lease_generation).execute(&pool).await.unwrap();
        assert_eq!(
            stale.rows_affected(),
            0,
            "expired worker is fenced from recording a result"
        );
        sqlx::query("DELETE FROM monitored_sites WHERE id=$1")
            .bind(site)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn smtp_delivers_to_local_mailpit() {
        let _guard = CLAIM_TEST_LOCK.lock().await;
        let pool = database().await;
        let config = SmtpConfig::from_env().unwrap();
        let site = Uuid::new_v4();
        let subscription = Uuid::new_v4();
        let change = Uuid::new_v4();
        sqlx::query("INSERT INTO monitored_sites(id,original_url,normalized_url,next_check_at) VALUES($1,$2,$2,now()+interval '1 day')")
            .bind(site)
            .bind(format!("https://example.test/{site}"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO subscriptions(id,site_id,subscriber_identity) VALUES($1,$2,'pagerune-test@example.test')")
            .bind(subscription).bind(site).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO site_changes(id,site_id,previous_content_hash,new_content_hash) VALUES($1,$2,$3,$4)")
            .bind(change).bind(site).bind("a".repeat(64)).bind("b".repeat(64)).execute(&pool).await.unwrap();
        let notification = Uuid::new_v4();
        let owner = Uuid::new_v4();
        sqlx::query("INSERT INTO notification_outbox(id,change_id,subscription_id,recipient,status,attempt_count,lease_owner,lease_expires_at,lease_generation) VALUES($1,$2,$3,'pagerune-test@example.test','sending',1,$4,now()+interval '180 seconds',1)")
            .bind(notification).bind(change).bind(subscription).bind(owner).execute(&pool).await.unwrap();
        let job = Delivery {
            id: notification,
            change_id: change,
            subscription_id: subscription,
            recipient: "pagerune-test@example.test".into(),
            site_url: format!("https://example.test/{site}"),
            detected_at: Utc::now(),
            lease_owner: owner,
            lease_generation: 1,
            attempt_count: 1,
            recovered: false,
        };
        deliver(pool.clone(), config, job.clone(), 8).await;
        let status: String =
            sqlx::query_scalar("SELECT status FROM notification_outbox WHERE id=$1")
                .bind(job.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            status, "delivered",
            "SMTP acceptance is recorded for the fenced claim"
        );
        sqlx::query("DELETE FROM monitored_sites WHERE id=$1")
            .bind(site)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn smtp_classifies_invalid_recipient_as_permanent_and_connection_failure_as_transient() {
        let mut config = SmtpConfig::default();
        let mut job = Delivery {
            id: Uuid::new_v4(),
            change_id: Uuid::new_v4(),
            subscription_id: Uuid::new_v4(),
            recipient: "not-an-email".into(),
            site_url: "https://example.test/".into(),
            detected_at: Utc::now(),
            lease_owner: Uuid::new_v4(),
            lease_generation: 1,
            attempt_count: 1,
            recovered: false,
        };
        let (code, permanent) = send(&database().await, &config, &job).await.unwrap_err();
        assert_eq!(code, "invalid_recipient_address");
        assert!(permanent);
        job.recipient = "pagerune-test@example.test".into();
        let pool = database().await;
        let site = Uuid::new_v4();
        sqlx::query("INSERT INTO monitored_sites(id,original_url,normalized_url) VALUES($1,$2,$2)")
            .bind(site)
            .bind(format!("https://example.test/{site}"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO subscriptions(id,site_id,subscriber_identity) VALUES($1,$2,'pagerune-test@example.test')").bind(job.subscription_id).bind(site).execute(&pool).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        config.port = listener.local_addr().unwrap().port();
        drop(listener);
        let (code, permanent) = send(&pool, &config, &job).await.unwrap_err();
        assert_eq!(code, "smtp_transport_failure");
        assert!(!permanent);
    }

    #[tokio::test]
    async fn delivery_persists_transient_retry_and_permanent_failure() {
        let _guard = CLAIM_TEST_LOCK.lock().await;
        let pool = database().await;
        let site = Uuid::new_v4();
        let subscription = Uuid::new_v4();
        sqlx::query("INSERT INTO monitored_sites(id,original_url,normalized_url,next_check_at) VALUES($1,$2,$2,now()+interval '1 day')").bind(site).bind(format!("https://example.test/{site}")).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO subscriptions(id,site_id,subscriber_identity) VALUES($1,$2,'retry@example.test')").bind(subscription).bind(site).execute(&pool).await.unwrap();
        let mut config = SmtpConfig::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        config.port = listener.local_addr().unwrap().port();
        drop(listener);
        for (recipient, expected_status, expected_error) in [
            ("retry@example.test", "pending", "smtp_transport_failure"),
            ("invalid-address", "failed", "invalid_recipient_address"),
        ] {
            let change = Uuid::new_v4();
            let notification = Uuid::new_v4();
            let owner = Uuid::new_v4();
            sqlx::query("INSERT INTO site_changes(id,site_id,previous_content_hash,new_content_hash) VALUES($1,$2,$3,$4)").bind(change).bind(site).bind("a".repeat(64)).bind("b".repeat(64)).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO notification_outbox(id,change_id,subscription_id,recipient,status,attempt_count,lease_owner,lease_expires_at,lease_generation) VALUES($1,$2,$3,$4,'sending',1,$5,now()+interval '180 seconds',1)").bind(notification).bind(change).bind(subscription).bind(recipient).bind(owner).execute(&pool).await.unwrap();
            let job = Delivery {
                id: notification,
                change_id: change,
                subscription_id: subscription,
                recipient: recipient.into(),
                site_url: format!("https://example.test/{site}"),
                detected_at: Utc::now(),
                lease_owner: owner,
                lease_generation: 1,
                attempt_count: 1,
                recovered: false,
            };
            deliver(pool.clone(), config.clone(), job, 8).await;
            let (status, error): (String, Option<String>) =
                sqlx::query_as("SELECT status,last_error FROM notification_outbox WHERE id=$1")
                    .bind(notification)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(status, expected_status);
            assert_eq!(error.as_deref(), Some(expected_error));
        }
        sqlx::query("DELETE FROM monitored_sites WHERE id=$1")
            .bind(site)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn notification_worker_stops_when_shutdown_is_requested() {
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();
        let pool = database().await;
        tokio::time::timeout(
            Duration::from_secs(1),
            super::run(pool, SmtpConfig::default(), 1, 8, token),
        )
        .await
        .unwrap();
    }
}
