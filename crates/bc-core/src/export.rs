//! Export types and the [`Exporter`] trait.
//!
//! A [`Snapshot`] is the ledger rendered as plain values: paths instead of
//! ids, and typed metadata ready for a plugin to print. An [`Exporter`] turns
//! one into the [`File`]s of its format, or says why it cannot.

#![expect(
    clippy::exhaustive_structs,
    clippy::exhaustive_enums,
    reason = "snapshot types mirror the WIT records field for field; callers build them by literal, \
              and a field added here is added to the WIT in the same change"
)]

use bc_models::Amount;
use bc_models::Cost;
use bc_models::Quote;
use bc_models::Reconciliation;
use jiff::civil::Date;

use crate::RawMetaEntry;

/// How much of the ledger's bookkeeping an export carries.
///
/// Re-exported from the crate root as [`crate::ExportLevel`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Level {
    /// What a person edits day to day.
    #[default]
    Working,
    /// Everything, for a complete and durable copy.
    Archive,
}

/// One posting leg of an exported transaction.
///
/// Re-exported from the crate root as [`crate::ExportPostingEntry`].
#[derive(Debug, Clone, PartialEq)]
pub struct PostingEntry {
    /// Opaque identifier used to find this posting again on import.
    pub handle: Option<String>,
    /// Account path, e.g. `"Assets:Bank:Checking"`.
    pub account: String,
    /// Leg amount. `None` means elided.
    pub amount: Option<Amount>,
    /// Price annotation, per unit or total.
    pub price: Option<Quote>,
    /// Cost basis, per unit or total.
    pub cost: Option<Cost>,
    /// Tag paths for this leg, segments joined by `/`.
    pub tags: Vec<String>,
    /// Typed key-value metadata for this leg, in display order.
    pub metadata: Vec<RawMetaEntry>,
    /// First day over which the posting's amount accrues.
    pub spread_from: Option<Date>,
    /// Last day over which the posting's amount accrues, inclusive.
    pub spread_until: Option<Date>,
}

/// One transaction of a snapshot.
///
/// Re-exported from the crate root as [`crate::ExportTransactionEntry`].
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionEntry {
    /// Opaque identifier used to find this transaction again on import.
    pub handle: String,
    /// Transaction date.
    pub date: Date,
    /// Free-text description.
    pub description: String,
    /// Whether, and how far, the transaction is confirmed.
    pub reconciliation: Reconciliation,
    /// Tag paths for the transaction, segments joined by `/`.
    pub tags: Vec<String>,
    /// Typed key-value metadata, in display order.
    pub metadata: Vec<RawMetaEntry>,
    /// The legs, in display order.
    pub postings: Vec<PostingEntry>,
    /// The postings carry no elided leg and their weights sum to non-zero in
    /// some commodity.
    pub unbalanced: bool,
}

/// One account of a snapshot.
///
/// Re-exported from the crate root as [`crate::ExportAccountEntry`].
#[derive(Debug, Clone, PartialEq)]
pub struct AccountEntry {
    /// Account path, e.g. `"Assets:Bank:Checking"`.
    pub path: String,
    /// Day the account opened.
    pub opened_on: Option<Date>,
    /// Day the account closed.
    pub closed_on: Option<Date>,
    /// Commodity codes the account may hold, the default first.
    pub commodities: Vec<String>,
    /// Host-rendered `bc-` fields, printed verbatim by the plugin.
    pub extra: Vec<RawMetaEntry>,
}

/// One commodity of a snapshot.
///
/// Re-exported from the crate root as [`crate::ExportCommodityEntry`].
#[derive(Debug, Clone, PartialEq)]
pub struct CommodityEntry {
    /// Commodity code, e.g. `"AUD"`.
    pub code: String,
    /// Host-rendered `bc-` fields, printed verbatim by the plugin.
    pub extra: Vec<RawMetaEntry>,
}

/// One budget of a snapshot.
///
/// Re-exported from the crate root as [`crate::ExportBudgetEntry`].
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetEntry {
    /// Account path the budget applies to.
    pub account: String,
    /// First day the budget applies.
    pub effective_from: Date,
    /// The period word, such as `monthly`.
    pub period: String,
    /// A fixed target amount, if the budget has one.
    pub target: Option<Amount>,
    /// A target expression, if the budget has one.
    pub expression: Option<String>,
    /// Host-rendered `bc-` fields, printed verbatim by the plugin.
    pub extra: Vec<RawMetaEntry>,
}

/// A stated balance of an account on a date.
///
/// Re-exported from the crate root as [`crate::ExportBalanceEntry`].
#[derive(Debug, Clone, PartialEq)]
pub struct BalanceEntry {
    /// Day the balance is stated for.
    pub date: Date,
    /// Account path.
    pub account: String,
    /// The stated balance.
    pub amount: Amount,
}

/// Everything one export covers.
///
/// Re-exported from the crate root as [`crate::ExportSnapshot`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    /// How much bookkeeping the export carries.
    pub level: Level,
    /// The accounts.
    pub accounts: Vec<AccountEntry>,
    /// The commodities.
    pub commodities: Vec<CommodityEntry>,
    /// The transactions.
    pub transactions: Vec<TransactionEntry>,
    /// The budgets.
    pub budgets: Vec<BudgetEntry>,
    /// The stated balances.
    pub balances: Vec<BalanceEntry>,
}

/// One file an export produces.
///
/// Re-exported from the crate root as [`crate::ExportFile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Path relative to the export root.
    pub path: String,
    /// The file's text.
    pub contents: String,
}

/// Errors produced during an export operation.
///
/// Re-exported from the crate root as [`crate::ExportError`].
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
#[expect(
    clippy::error_impl_error,
    reason = "re-exported as ExportError; the name is unambiguous at the crate root"
)]
pub enum Error {
    /// A write error occurred while producing the output.
    #[error("write error: {0}")]
    Write(String),
    /// The plugin behind the exporter failed: it would not instantiate, it
    /// trapped, or it reported an error of its own.
    #[error("plugin error: {0}")]
    Plugin(String),
    /// The exporter does not export.
    #[error("this exporter does not support export")]
    Unsupported,
    /// The format cannot spell these items; each entry names one.
    #[error("the format cannot express: {}", .0.join(", "))]
    Unspellable(Vec<String>),
}

/// An object-safe trait implemented by every format-specific exporter.
///
/// Implementors are `Send + Sync + 'static` so they can be stored in
/// `Arc<dyn Exporter>` and used across async tasks.
pub trait Exporter: Send + Sync + 'static {
    /// A short, stable identifier for this exporter (e.g. `"beancount"`).
    fn name(&self) -> &str;

    /// Renders `snapshot` as the files of the exporter's format.
    ///
    /// # Arguments
    ///
    /// * `snapshot` - The ledger to render.
    ///
    /// # Returns
    ///
    /// The files to write, with paths relative to the export root.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when the exporter does not export,
    /// [`Error::Unspellable`] when the format cannot express part of the
    /// snapshot, and another [`Error`] for any other failure.
    fn export(&self, snapshot: &Snapshot) -> Result<Vec<File>, Error>;
}
