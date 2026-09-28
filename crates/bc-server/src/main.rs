//! `borrow-checker-server`: serves the ledger to browsers.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser as _;
#[cfg(unix)]
use tokio::signal::unix::SignalKind;
#[cfg(unix)]
use tokio::signal::unix::signal;
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// Exit status after a restore: the supervisor restarts the server, which
/// swaps the candidate in on startup (`EX_TEMPFAIL`).
const RESTART_EXIT: i32 = 75;

/// Serve BorrowChecker to browsers over HTTP.
#[derive(Debug, clap::Parser)]
#[command(name = "borrow-checker-server", version, about)]
struct Args {
    /// Listen address; overrides `server.bind` from the config.
    #[arg(long)]
    bind: Option<SocketAddr>,
}

#[expect(clippy::print_stderr, reason = "fatal startup errors precede logging")]
#[tokio::main]
async fn main() {
    let args = Args::parse();
    let settings = match bc_config::Settings::load() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not load config: {e}");
            std::process::exit(1);
        }
    };
    let _otel = init_tracing();

    let bind = args.bind.unwrap_or_else(|| settings.server().bind());
    if !bind.ip().is_loopback() {
        tracing::warn!(%bind, "listening beyond loopback with no authentication");
    }
    let app = match bc_service::AppState::open(&settings).await {
        Ok(app) => app,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let (shared, restart_rx) = bc_server::Shared::new(app);
    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: cannot listen on {bind}: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!(%bind, "serving");

    let served = axum::serve(listener, bc_server::router(Arc::clone(&shared)))
        .with_graceful_shutdown(shutdown(restart_rx))
        .await;
    shared.app.close().await;
    if let Err(e) = served {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
    if *shared.restart.borrow() {
        std::process::exit(RESTART_EXIT);
    }
}

/// Resolves on Ctrl-C, SIGTERM, or the restart flag turning `true`.
#[expect(
    clippy::integer_division_remainder_used,
    reason = "tokio::select! picks its first branch with a modulo"
)]
async fn shutdown(mut restart: watch::Receiver<bool>) {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        () = terminate() => {}
        _ = restart.wait_for(|&r| r) => {}
    }
}

/// Resolves on SIGTERM (Unix) or never.
async fn terminate() {
    #[cfg(unix)]
    {
        if let Ok(mut s) = signal(SignalKind::terminate()) {
            s.recv().await;
            return;
        }
    }
    std::future::pending::<()>().await;
}

/// Logs to stderr at `RUST_LOG` (default `info`), plus OTLP when
/// `OTEL_EXPORTER_OTLP_ENDPOINT` is set, as the CLI does.
#[expect(
    clippy::expect_used,
    reason = "a bad OTLP endpoint is fatal at startup"
)]
fn init_tracing() -> Option<bc_otel::OtelGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let otel = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .ok()
        .map(|_| bc_otel::init().expect("failed to initialise OpenTelemetry"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(otel.as_ref().map(|_| bc_otel::tracing_layer()))
        .init();
    otel
}
