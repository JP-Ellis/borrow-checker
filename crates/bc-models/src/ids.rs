//! Typed ID newtypes for all BorrowChecker domain entities.
//!
//! Each ID type wraps a [`MagicTypeId`] from the `mti` crate,
//! producing human-readable prefixed IDs like `account_01j...`.
//!
//! Note: `PostingId` and `TransactionId` have moved to `transaction.rs`.
//! This module is retained during the migration and will be removed in Task 8.
