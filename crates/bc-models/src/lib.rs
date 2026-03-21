//! Shared domain types for BorrowChecker.
//!
//! This crate is the shared vocabulary for the whole workspace.
//! It has no internal dependencies and no I/O.

#![expect(
    clippy::pub_use,
    reason = "re-exports are intentional for an ergonomic public API surface"
)]

pub mod account;
pub mod ids;
pub mod money;
pub mod period;
pub mod settings;
pub mod transaction;

pub use account::{Account, AccountType};
pub use ids::{AccountId, EventId, ImportBatchId, PostingId, ProfileId, TransactionId};
pub use money::{Amount, CommodityCode, Decimal};
pub use period::Period;
pub use settings::GlobalSettings;
pub use transaction::{Posting, Transaction, TransactionStatus};
