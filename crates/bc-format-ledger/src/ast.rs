//! Internal AST for the Ledger file format.
//!
//! These types are not part of the public API.

use jiff::civil::Date;
use rust_decimal::Decimal;

/// A single top-level item in a Ledger file.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Entry {
    /// A transaction with one or more postings.
    Transaction(Transaction),
    /// An `account <name>` declaration (parsed but not stored).
    AccountDecl(String),
    /// A `commodity <code>` declaration (parsed but not stored).
    CommodityDecl(String),
    /// A comment line (`;`, `#`, etc.).
    Comment(String),
}

/// A Ledger transaction.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Transaction {
    /// The transaction date.
    pub date: Date,
    /// The cleared status of the transaction.
    pub cleared: ClearedStatus,
    /// The payee name.
    pub payee: String,
    /// An optional inline comment.
    pub comment: Option<String>,
    /// The list of postings.
    pub postings: Vec<Posting>,
}

/// Transaction cleared status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClearedStatus {
    /// No flag — uncleared.
    Uncleared,
    /// `*` — cleared.
    Cleared,
    /// `!` — pending.
    Pending,
}

/// A single posting within a transaction.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Posting {
    /// The account name.
    pub account: String,
    /// `None` means the amount is elided (inferred to balance the transaction).
    pub amount: Option<PostingAmount>,
    /// An optional inline comment.
    pub comment: Option<String>,
}

/// An amount as it appears in a Ledger posting.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PostingAmount {
    /// The numeric value.
    pub value: Decimal,
    /// The commodity code (e.g. `"AUD"`, `"USD"`).
    pub commodity: String,
}
