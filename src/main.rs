use anyhow::Context;
use pagerune::{AppState, app, config::Config};
use sqlx::postgres::PgPoolOptions;
use std::{env, time::Duration};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "pagerune=info".into()),
        )
        .init();
    let config = Config::from_env()?;
    let database = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(3))
        .connect_lazy_with(config.database);

    if env::args().nth(1).as_deref() == Some("migrate") {
        sqlx::migrate!()
            .run(&database)
            .await
            .context("applying database migrations")?;
        tracing::info!("database migrations applied");
        return Ok(());
    }

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let bind = listener.local_addr()?;
    tracing::info!(%bind, "PageRune listening");
    let state = AppState {
        database,
        dev_api_enabled: config.dev_api_enabled,
        public_base_url: config.public_base_url.clone(),
    };
    let worker = tokio::spawn(pagerune::worker::run(
        state.clone(),
        pagerune::fetcher::FetchConfig {
            timeout: Duration::from_secs(config.fetch_timeout_seconds),
            max_body_bytes: config.max_response_bytes,
        },
    ));
    let cancel = tokio_util::sync::CancellationToken::new();
    let notification_worker = tokio::spawn(pagerune::notifications::run(
        state.database.clone(),
        config.smtp.clone(),
        config.notification_concurrency,
        config.notification_max_attempts,
        cancel.clone(),
    ));
    let shutdown_cancel = cancel.clone();
    let server_result = axum::serve(listener, app(state))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown_cancel.cancel();
        })
        .await;
    cancel.cancel();
    notification_worker.await?;
    worker.abort();
    server_result?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl+C handler")
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { () = ctrl_c => {}, () = terminate => {} }
}
