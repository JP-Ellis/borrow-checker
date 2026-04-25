//! Shared intermediate representation for OFX v1 and v2 statement data.

use bc_sdk::Date;
use rust_decimal::Decimal;

/// A parsed OFX bank statement.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OfxStatement {
    /// Default currency for the statement (e.g. `"AUD"`).
    pub currency: String,
    /// Account identifier.
    pub account_id: String,
    /// All parsed transactions.
    pub transactions: Vec<OfxTransaction>,
}

/// A single OFX transaction record.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OfxTransaction {
    /// OFX transaction type (informational).
    pub trntype: String,
    /// Value date (parsed from `DTPOSTED`).
    pub date: Date,
    /// Signed amount: negative = debit (money out), positive = credit (money in).
    pub amount: Decimal,
    /// Unique ID for deduplication (`FITID`).
    pub fitid: String,
    /// Payee name (`NAME`).
    pub name: Option<String>,
    /// Memo / description (`MEMO`).
    pub memo: Option<String>,
}
