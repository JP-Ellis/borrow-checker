//! BorrowChecker core engine.
//!
//! Provides the append-only event log, SQLite read projections,
//! account and transaction services, balance engine, and settings store.

#![expect(
    clippy::pub_use,
    reason = "re-exports are intentional for an ergonomic public API surface"
)]

pub(crate) mod account;
pub(crate) mod balance;
pub(crate) mod db;
pub(crate) mod error;
pub(crate) mod events;
pub mod export;
pub mod import;
pub(crate) mod settings;
pub(crate) mod transaction;

pub use account::Service as AccountService;
pub use balance::Engine as BalanceEngine;
pub use db::open_db;
pub use error::BcError;
pub use error::BcResult;
pub use events::Event;
pub use events::EventRecord;
pub use events::SqliteStore as SqliteEventStore;
pub use export::ExportData;
pub use export::ExportError;
pub use export::Exporter;
pub use import::ImportConfig;
pub use import::ImportError;
pub use import::Importer;
pub use import::RawTransaction;
pub use settings::Store as SettingsStore;
pub use transaction::Service as TransactionService;
