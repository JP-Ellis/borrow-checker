//! BorrowChecker TUI binary entry point.
//!
//! Resolves the database path (default → config file → `BC_*` env vars → `--db-path` flag),
//! initialises tracing and OpenTelemetry, opens the database, and delegates to
//! [`bc_tui::run`] for the full application lifecycle.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// BorrowChecker terminal user interface.
#[derive(Parser)]
#[command(name = "borrow-checker-tui", version, about)]
struct Args {
    /// Path to the SQLite database file.
    ///
    /// Overrides the config file and `BC_DB_PATH` environment variable.
    #[arg(long)]
    db_path: Option<PathBuf>,
}

#[expect(
    clippy::print_stderr,
    reason = "startup errors are printed to stderr before the TUI takes over the terminal"
)]
fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // Load config — non-fatal; fall back to defaults on error.
    let settings = bc_config::Settings::load().unwrap_or_else(|e| {
        eprintln!("warning: could not load config: {e}; using defaults");
        bc_config::Settings::default()
    });

    // Initialise tracing before the TUI takes over the terminal.
    let _otel_guard = setup_tracing(&settings);

    // Priority order: default < config file < BC_* env vars < --db-path flag.
    let db_path = args.db_path.unwrap_or_else(|| settings.db_path());

    if let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("warning: could not create database directory: {e}");
        }
    }

    let ctx = Arc::new(rt.block_on(bc_tui::context::TuiContext::open(&db_path))?);
    bc_tui::run(ctx)
}

/// Initialises the tracing subscriber for the TUI.
///
/// The TUI runs in raw terminal mode, so we skip the stderr formatter to avoid
/// corrupting the display. Tracing output is forwarded to the OTLP exporter
/// when `OTEL_EXPORTER_OTLP_ENDPOINT` is set; the log level is controlled by
/// the `RUST_LOG` environment variable.
///
/// # Returns
///
/// An [`bc_otel::OtelGuard`] that must be kept alive for the duration of the
/// process, or [`None`] if `OTEL_EXPORTER_OTLP_ENDPOINT` was not set.
#[must_use]
fn setup_tracing(settings: &bc_config::Settings) -> Option<bc_otel::OtelGuard> {
    let filter = std::env::var("RUST_LOG").map_or_else(
        |_| {
            settings
                .cli()
                .log()
                .map_or_else(|| EnvFilter::new("warn"), EnvFilter::new)
        },
        EnvFilter::new,
    );

    #[expect(
        clippy::expect_used,
        reason = "OpenTelemetry initialisation failure is unrecoverable; \
        the user must fix the OTLP endpoint configuration before the process can continue"
    )]
    let otel_guard = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .ok()
        .map(|_| bc_otel::init().expect("failed to initialise OpenTelemetry"));

    tracing_subscriber::registry()
        .with(filter)
        .with(otel_guard.as_ref().map(|_| bc_otel::tracing_layer()))
        .init();

    tracing::debug!("Tracing initialized");
    otel_guard
}
