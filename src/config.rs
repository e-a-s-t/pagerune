use anyhow::{Context, bail};
use sqlx::postgres::PgConnectOptions;
use std::{env, net::SocketAddr, str::FromStr};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub database: PgConnectOptions,
    pub dev_api_enabled: bool,
    pub fetch_timeout_seconds: u64,
    pub max_response_bytes: usize,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_values(
            env::var("PAGERUNE_BIND").ok(),
            env::var("DATABASE_URL").ok(),
            env::var("PAGERUNE_DEV_API_ENABLED").ok(),
            env::var("PAGERUNE_FETCH_TIMEOUT_SECONDS").ok(),
            env::var("PAGERUNE_MAX_RESPONSE_BYTES").ok(),
        )
    }

    pub fn from_values(
        bind: Option<String>,
        database_url: Option<String>,
        dev_api_enabled: Option<String>,
        fetch_timeout_seconds: Option<String>,
        max_response_bytes: Option<String>,
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
        if dev_api_enabled && !bind.ip().is_loopback() {
            bail!("PAGERUNE_DEV_API_ENABLED requires PAGERUNE_BIND to use a loopback address");
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
        Ok(Self {
            bind,
            database,
            dev_api_enabled,
            fetch_timeout_seconds,
            max_response_bytes,
        })
    }
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
}
