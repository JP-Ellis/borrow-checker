//! BorrowChecker command layer shared by the desktop app and the web server.
//!
//! [`AppState::open`] builds the services; [`dispatch`] runs a command by
//! name. Hosts add only their transport.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

pub mod commands;
mod dispatch;
pub(crate) mod ipc;
mod restore;
mod state;

pub use dispatch::dispatch;
pub use restore::confine_to_dir;
pub use state::AppState;
