//! BorrowChecker TUI binary entry point.
//!
//! Resolves the database path, opens the database, and delegates to
//! [`bc_tui::run`] for the full application lifecycle.

use std::path::PathBuf;
use std::sync::Arc;

#[expect(
    clippy::print_stderr,
    reason = "directory creation warning is printed to stderr before the TUI starts"
)]
fn main() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let db_path = db_path_from_args();
    if let Some(parent) = db_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("warning: could not create database directory: {e}");
        }
    }
    let ctx = Arc::new(rt.block_on(bc_tui::context::TuiContext::open(&db_path))?);
    bc_tui::run(ctx)
}

/// Returns the database path from `--db-path <path>` CLI argument, or the
/// XDG data default.
fn db_path_from_args() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--db-path" {
            if let Some(path) = args.next() {
                return PathBuf::from(path);
            }
        }
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("borrow-checker")
        .join("borrow-checker.db")
}
