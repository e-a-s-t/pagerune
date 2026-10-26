use anyhow::{Context, bail};
use sqlx::postgres::PgConnectOptions;
use std::{env, net::SocketAddr, str::FromStr};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub database: PgConnectOptions,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_values(
            env::var("PAGERUNE_BIND").ok(),
            env::var("DATABASE_URL").ok(),
        )
    }

    pub fn from_values(bind: Option<String>, database_url: Option<String>) -> anyhow::Result<Self> {
        let bind = bind.unwrap_or_else(|| "127.0.0.1:8080".to_owned());
        let bind = bind
            .parse()
            .context("PAGERUNE_BIND must be a valid socket address")?;
        let database_url =
            database_url.ok_or_else(|| anyhow::anyhow!("DATABASE_URL is required"))?;
        if !database_url.starts_with("postgres://") && !database_url.starts_with("postgresql://") {
            bail!("DATABASE_URL must use PostgreSQL");
        }
        let database = PgConnectOptions::from_str(&database_url)
            .map_err(|_| anyhow::anyhow!("DATABASE_URL is invalid"))?;
        Ok(Self { bind, database })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_DATABASE: &str = "postgres://user:password@localhost/pagerune";

    #[test]
    fn defaults_bind_and_accepts_postgres_url() {
        let config = Config::from_values(None, Some(VALID_DATABASE.into())).unwrap();
        assert_eq!(config.bind, "127.0.0.1:8080".parse().unwrap());
    }

    #[test]
    fn rejects_missing_database_url() {
        assert!(
            Config::from_values(None, None)
                .unwrap_err()
                .to_string()
                .contains("DATABASE_URL")
        );
    }

    #[test]
    fn rejects_invalid_bind_address() {
        assert!(
            Config::from_values(Some("not-an-address".into()), Some(VALID_DATABASE.into()))
                .is_err()
        );
    }

    #[test]
    fn rejects_non_postgres_database_url() {
        let error = Config::from_values(None, Some("sqlite::memory:".into())).unwrap_err();
        assert!(error.to_string().contains("PostgreSQL"));
    }

    #[test]
    fn does_not_echo_database_credentials_in_errors() {
        let error = Config::from_values(
            None,
            Some("postgres://secret:secret@localhost:invalid/pagerune".into()),
        )
        .unwrap_err();
        assert!(!error.to_string().contains("secret"));
    }
}
