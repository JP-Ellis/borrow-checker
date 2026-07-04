//! BorrowChecker core engine.
//!
//! Provides the append-only event log, SQLite read projections,
//! account and transaction services, balance engine, and settings store.

#![expect(
    clippy::pub_use,
    reason = "re-exports are intentional for an ergonomic public API surface"
)]

pub(crate) mod account;
pub(crate) mod asset;
pub mod backup;
pub(crate) mod balance;
pub(crate) mod budget;
pub(crate) mod budget_tree;
pub(crate) mod commodity;
pub(crate) mod db;
pub(crate) mod error;
pub(crate) mod events;
pub(crate) mod export;
pub(crate) mod fx;
pub(crate) mod import;
#[cfg(feature = "ipc")]
pub mod ipc;
pub(crate) mod loan;
pub(crate) mod period_overlap;
pub(crate) mod settings;
pub(crate) mod tag;
pub(crate) mod transaction;

pub use account::Service as AccountService;
pub use asset::Service as AssetService;
pub use backup::BackupKind;
pub use backup::BackupPolicy;
pub use backup::BackupRecord;
pub use backup::Service as BackupService;
pub use balance::Engine as BalanceEngine;
pub use balance::PostingBucket;
pub use bc_models::BudgetWindow;
pub use bc_models::governing_revision;
pub use budget::BudgetService;
pub use budget::BudgetStatus;
pub use budget::BudgetStatusEngine;
pub use budget_tree::BudgetOverview;
pub use budget_tree::BudgetTreeItem;
pub use budget_tree::BudgetTreeService;
pub use budget_tree::NativePeriodStatus;
pub use commodity::Service as CommodityService;
pub use db::open_db;
pub use db::open_db_at;
pub use db::open_db_with_backup;
pub use error::BcError;
pub use error::BcResult;
pub use events::Event;
pub use events::EventRecord;
pub use events::SqliteStore as SqliteEventStore;
pub use export::Data as ExportData;
pub use export::Error as ExportError;
pub use export::Exporter;
pub use fx::FxError;
pub use fx::FxRateService;
pub use fx::NoopFxRateService;
pub use fx::noop_fx;
pub use import::Config as ImportConfig;
pub use import::Error as ImportError;
pub use import::Importer;
pub use import::RawTransaction;
pub use import::profile::ImportProfile;
pub use import::profile::Service as ImportProfileService;
pub use import::registry::Factory as ImporterFactory;
pub use import::registry::Registry as ImporterRegistry;
pub use loan::Service as LoanService;
pub use period_overlap::PeriodOverlap;
pub use settings::Store as SettingsStore;
pub use tag::Service as TagService;
pub use transaction::Service as TransactionService;

#[cfg(test)]
mod migration_smoke {
    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_sources_table_exists(pool: sqlx::SqlitePool) {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='transaction_sources'",
        )
        .fetch_one(&pool)
        .await
        .expect("query sqlite_master");
        pretty_assertions::assert_eq!(count, 1, "transaction_sources table must exist");
    }
}
