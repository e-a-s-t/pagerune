use anyhow::{Context, bail};
use sqlx::postgres::PgConnectOptions;
use std::{env, net::SocketAddr, str::FromStr};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub database: PgConnectOptions,
    pub dev_api_enabled: bool,
    pub trusted_proxy_mode: bool,
    pub fetch_timeout_seconds: u64,
    pub max_response_bytes: usize,
    pub smtp: crate::notifications::SmtpConfig,
    pub notification_concurrency: usize,
    pub notification_max_attempts: i32,
    pub public_base_url: String,
    pub token_key: [u8; 32],
    pub turnstile_site_key: String,
    pub turnstile_secret: Option<String>,
    pub test_human_verification: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_values_full(
            env::var("PAGERUNE_BIND").ok(),
            env::var("DATABASE_URL").ok(),
            env::var("PAGERUNE_DEV_API_ENABLED").ok(),
            env::var("PAGERUNE_TRUSTED_PROXY_MODE").ok(),
            env::var("PAGERUNE_FETCH_TIMEOUT_SECONDS").ok(),
            env::var("PAGERUNE_MAX_RESPONSE_BYTES").ok(),
            crate::notifications::SmtpConfig::from_env()?,
            env::var("PAGERUNE_NOTIFICATION_CONCURRENCY").ok(),
            env::var("PAGERUNE_NOTIFICATION_MAX_ATTEMPTS").ok(),
            env::var("PAGERUNE_PUBLIC_BASE_URL").ok(),
            env::var("PAGERUNE_TOKEN_KEY").ok(),
            env::var("PAGERUNE_TURNSTILE_SITE_KEY").ok(),
            env::var("PAGERUNE_TURNSTILE_SECRET").ok(),
            env::var("PAGERUNE_TURNSTILE_TEST_MODE").ok(),
        )
    }

    pub fn from_values(
        bind: Option<String>,
        database_url: Option<String>,
        dev_api_enabled: Option<String>,
        fetch_timeout_seconds: Option<String>,
        max_response_bytes: Option<String>,
    ) -> anyhow::Result<Self> {
        Self::from_values_full(
            bind,
            database_url,
            dev_api_enabled,
            None,
            fetch_timeout_seconds,
            max_response_bytes,
            crate::notifications::SmtpConfig::default(),
            None,
            None,
            None,
            Some("0000000000000000000000000000000000000000000000000000000000000000".into()),
            Some("test-site-key".into()),
            Some("test-secret".into()),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_values_full(
        bind: Option<String>,
        database_url: Option<String>,
        dev_api_enabled: Option<String>,
        trusted_proxy_mode: Option<String>,
        fetch_timeout_seconds: Option<String>,
        max_response_bytes: Option<String>,
        smtp: crate::notifications::SmtpConfig,
        notification_concurrency: Option<String>,
        notification_max_attempts: Option<String>,
        public_base_url: Option<String>,
        token_key: Option<String>,
        turnstile_site_key: Option<String>,
        turnstile_secret: Option<String>,
        test_human_verification: Option<String>,
    ) -> anyhow::Result<Self> {
        let bind = bind.unwrap_or_else(|| "127.0.0.1:8080".to_owned());
        let bind: SocketAddr = bind
            .parse()
            .context("PAGERUNE_BIND must be a valid socket address")?;
        let database_url =
            database_url.ok_or_else(|| anyhow::anyhow!("DATABASE_URL is required"))?;
        if !database_url.starts_with("postgres://") && !database_url.starts_with("postgresql://") {
            bail!("DATABASE_URL must use PostgreSQL");
        }
        let database = PgConnectOptions::from_str(&database_url)
            .map_err(|_| anyhow::anyhow!("DATABASE_URL is invalid"))?;
        let dev_api_enabled = match dev_api_enabled.as_deref().unwrap_or("false") {
            "true" => true,
            "false" => false,
            _ => bail!("PAGERUNE_DEV_API_ENABLED must be true or false"),
        };
        let trusted_proxy_mode = match trusted_proxy_mode.as_deref().unwrap_or("false") {
            "true" => true,
            "false" => false,
            _ => bail!("PAGERUNE_TRUSTED_PROXY_MODE must be true or false"),
        };
        if dev_api_enabled && !bind.ip().is_loopback() && !trusted_proxy_mode {
            bail!(
                "PAGERUNE_DEV_API_ENABLED requires a loopback bind unless PAGERUNE_TRUSTED_PROXY_MODE is true"
            );
        }
        let fetch_timeout_seconds = fetch_timeout_seconds
            .unwrap_or_else(|| "15".into())
            .parse::<u64>()
            .map_err(|_| anyhow::anyhow!("PAGERUNE_FETCH_TIMEOUT_SECONDS must be an integer"))?;
        if !(1..=120).contains(&fetch_timeout_seconds) {
            bail!("PAGERUNE_FETCH_TIMEOUT_SECONDS must be between 1 and 120");
        }
        let max_response_bytes = max_response_bytes
            .unwrap_or_else(|| "2097152".into())
            .parse::<usize>()
            .map_err(|_| anyhow::anyhow!("PAGERUNE_MAX_RESPONSE_BYTES must be an integer"))?;
        if !(1024..=20 * 1024 * 1024).contains(&max_response_bytes) {
            bail!("PAGERUNE_MAX_RESPONSE_BYTES must be between 1024 and 20971520");
        }
        let token_key_text =
            token_key.ok_or_else(|| anyhow::anyhow!("PAGERUNE_TOKEN_KEY is required"))?;
        let token_key_bytes = hex::decode(&token_key_text)
            .map_err(|_| anyhow::anyhow!("PAGERUNE_TOKEN_KEY must be 64 hexadecimal characters"))?;
        let token_key: [u8; 32] = token_key_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("PAGERUNE_TOKEN_KEY must be 64 hexadecimal characters"))?;
        let test_human_verification = match test_human_verification.as_deref().unwrap_or("false") {
            "true" if dev_api_enabled && (bind.ip().is_loopback() || trusted_proxy_mode) => true,
            "true" => {
                bail!(
                    "PAGERUNE_TURNSTILE_TEST_MODE requires the loopback development API or trusted proxy mode"
                )
            }
            "false" => false,
            _ => bail!("PAGERUNE_TURNSTILE_TEST_MODE must be true or false"),
        };
        let turnstile_site_key = turnstile_site_key.unwrap_or_default();
        if !test_human_verification
            && (turnstile_site_key.is_empty()
                || turnstile_secret.as_deref().unwrap_or_default().is_empty())
        {
            bail!(
                "PAGERUNE_TURNSTILE_SITE_KEY and PAGERUNE_TURNSTILE_SECRET are required unless development test mode is enabled"
            );
        }
        Ok(Self {
            bind,
            database,
            dev_api_enabled,
            trusted_proxy_mode,
            fetch_timeout_seconds,
            max_response_bytes,
            smtp,
            notification_concurrency: parse_range(
                notification_concurrency,
                "PAGERUNE_NOTIFICATION_CONCURRENCY",
                2,
                1,
                32,
            )?,
            notification_max_attempts: parse_range(
                notification_max_attempts,
                "PAGERUNE_NOTIFICATION_MAX_ATTEMPTS",
                8,
                1,
                20,
            )? as i32,
            public_base_url: public_base_url.unwrap_or_else(|| "http://localhost:8080".into()),
            token_key,
            turnstile_site_key,
            turnstile_secret,
            test_human_verification,
        })
    }
}

fn parse_range(
    value: Option<String>,
    name: &str,
    default: usize,
    min: usize,
    max: usize,
) -> anyhow::Result<usize> {
    let parsed = value
        .unwrap_or_else(|| default.to_string())
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("{name} must be an integer"))?;
    if !(min..=max).contains(&parsed) {
        bail!("{name} must be between {min} and {max}");
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_DATABASE: &str = "postgres://user:password@localhost/pagerune";

    #[test]
    fn defaults_bind_and_accepts_postgres_url() {
        let config =
            Config::from_values(None, Some(VALID_DATABASE.into()), None, None, None).unwrap();
        assert_eq!(config.bind, "127.0.0.1:8080".parse().unwrap());
    }

    #[test]
    fn rejects_missing_database_url() {
        assert!(
            Config::from_values(None, None, None, None, None)
                .unwrap_err()
                .to_string()
                .contains("DATABASE_URL")
        );
    }

    #[test]
    fn rejects_invalid_bind_address() {
        assert!(
            Config::from_values(
                Some("not-an-address".into()),
                Some(VALID_DATABASE.into()),
                None,
                None,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_non_postgres_database_url() {
        let error = Config::from_values(None, Some("sqlite::memory:".into()), None, None, None)
            .unwrap_err();
        assert!(error.to_string().contains("PostgreSQL"));
    }

    #[test]
    fn does_not_echo_database_credentials_in_errors() {
        let error = Config::from_values(
            None,
            Some("postgres://secret:secret@localhost:invalid/pagerune".into()),
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(!error.to_string().contains("secret"));
    }

    #[test]
    fn development_api_requires_loopback_and_explicit_boolean() {
        assert!(
            Config::from_values(
                Some("0.0.0.0:8080".into()),
                Some(VALID_DATABASE.into()),
                Some("true".into()),
                None,
                None
            )
            .is_err()
        );
        assert!(
            Config::from_values(
                None,
                Some(VALID_DATABASE.into()),
                Some("yes".into()),
                None,
                None
            )
            .is_err()
        );
        assert!(
            Config::from_values(
                None,
                Some(VALID_DATABASE.into()),
                Some("true".into()),
                None,
                None
            )
            .unwrap()
            .dev_api_enabled
        );
    }

    #[test]
    fn trusted_proxy_mode_allows_private_non_loopback_dev_api_bind() {
        let config = Config::from_values_full(
            Some("0.0.0.0:8080".into()),
            Some(VALID_DATABASE.into()),
            Some("true".into()),
            Some("true".into()),
            None,
            None,
            crate::notifications::SmtpConfig::default(),
            None,
            None,
            None,
            Some("0000000000000000000000000000000000000000000000000000000000000000".into()),
            Some("test-site-key".into()),
            None,
            Some("true".into()),
        )
        .unwrap();
        assert!(config.dev_api_enabled);
        assert!(config.trusted_proxy_mode);
        assert!(config.test_human_verification);
    }
}
