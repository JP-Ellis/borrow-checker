//! BorrowChecker core engine.
//!
//! Provides the append-only event log, SQLite read projections,
//! account and transaction services, balance engine, and settings store.

#![expect(
    clippy::pub_use,
    reason = "re-exports are intentional for an ergonomic public API surface"
)]

pub mod account;
pub mod balance;
pub mod db;
pub mod error;
pub mod events;
pub mod settings;
pub mod transaction;
pub use account::AccountService;
pub use balance::BalanceEngine;
pub use db::open_db;
pub use error::{BcError, BcResult};
pub use events::{Event, EventRecord, SqliteEventStore};
pub use settings::SettingsStore;
pub use transaction::TransactionService;
