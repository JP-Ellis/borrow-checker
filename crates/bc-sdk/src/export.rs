//! Types for the exporter interface: the snapshot an exporter renders and the
//! files it returns.
//!
//! These mirror the `exporter` WIT interface. The host builds an
//! [`ExportSnapshot`] from its own store; a plugin implementing [`Exporter`]
//! turns it into [`ExportFile`]s. Dates, amounts, prices, costs and metadata
//! reuse the importer-side types.

#![expect(
    clippy::module_name_repetitions,
    reason = "the types are named after the WIT records they mirror"
)]

use std::str::FromStr as _;

use rust_decimal::Decimal;

use crate::__bindings::borrow_checker::sdk::types::Amount as WitAmount;
use crate::__bindings::borrow_checker::sdk::types::Cost as WitCost;
use crate::__bindings::borrow_checker::sdk::types::Date as WitDate;
use crate::__bindings::borrow_checker::sdk::types::MetaEntry as WitMetaEntry;
use crate::__bindings::borrow_checker::sdk::types::MetaValue as WitMetaValue;
use crate::__bindings::borrow_checker::sdk::types::Quote as WitQuote;
use crate::__bindings::borrow_checker::sdk::types::Reconciliation as WitReconciliation;
use crate::__bindings::exports::borrow_checker::sdk::exporter::ExportAccount as WitExportAccount;
use crate::__bindings::exports::borrow_checker::sdk::exporter::ExportBalance as WitExportBalance;
use crate::__bindings::exports::borrow_checker::sdk::exporter::ExportBudget as WitExportBudget;
use crate::__bindings::exports::borrow_checker::sdk::exporter::ExportCommodity as WitExportCommodity;
use crate::__bindings::exports::borrow_checker::sdk::exporter::ExportError as WitExportError;
use crate::__bindings::exports::borrow_checker::sdk::exporter::ExportFile as WitExportFile;
use crate::__bindings::exports::borrow_checker::sdk::exporter::ExportLevel as WitExportLevel;
use crate::__bindings::exports::borrow_checker::sdk::exporter::ExportPosting as WitExportPosting;
use crate::__bindings::exports::borrow_checker::sdk::exporter::ExportSnapshot as WitExportSnapshot;
use crate::__bindings::exports::borrow_checker::sdk::exporter::ExportTransaction as WitExportTransaction;
use crate::Amount;
use crate::Cost;
use crate::Date;
use crate::MetaEntry;
use crate::MetaValue;
use crate::Quote;
use crate::Reconciliation;

/// How much of the ledger's bookkeeping an export carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExportLevel {
    /// What a person edits day to day.
    Working,
    /// Everything, for a complete and durable copy.
    Archive,
}

/// One posting leg of an exported transaction.
#[derive(bon::Builder, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportPosting {
    /// Opaque identifier the host uses to find this posting again on import.
    #[builder(into)]
    pub handle: Option<String>,
    /// Account path, e.g. `"Assets:Bank:Checking"`.
    #[builder(into)]
    pub account: String,
    /// Leg amount. `None` means elided.
    pub amount: Option<Amount>,
    /// Price annotation, per unit or total.
    pub price: Option<Quote>,
    /// Cost basis, per unit or total.
    pub cost: Option<Cost>,
    /// Tag paths for this leg, segments joined by `/`.
    #[builder(default)]
    pub tags: Vec<String>,
    /// Typed key-value metadata for this leg, in display order.
    #[builder(default)]
    pub metadata: Vec<MetaEntry>,
    /// First day over which the posting's amount accrues.
    pub spread_from: Option<Date>,
    /// Last day over which the posting's amount accrues, inclusive.
    pub spread_until: Option<Date>,
}

/// One transaction of the snapshot.
#[derive(bon::Builder, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportTransaction {
    /// Opaque identifier the host uses to find this transaction again on
    /// import.
    #[builder(into)]
    pub handle: String,
    /// The transaction's date.
    pub date: Date,
    /// The transaction's description.
    #[builder(into)]
    pub description: String,
    /// How far the transaction is confirmed.
    pub reconciliation: Reconciliation,
    /// Tag paths for the transaction, segments joined by `/`.
    #[builder(default)]
    pub tags: Vec<String>,
    /// Typed key-value metadata, in display order.
    #[builder(default)]
    pub metadata: Vec<MetaEntry>,
    /// The posting legs, in order.
    #[builder(default)]
    pub postings: Vec<ExportPosting>,
    /// The postings carry no elided leg and their weights sum to non-zero in
    /// some commodity.
    #[builder(default)]
    pub unbalanced: bool,
}

/// One account of the snapshot.
#[derive(bon::Builder, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportAccount {
    /// Account path, e.g. `"Assets:Bank:Checking"`.
    #[builder(into)]
    pub path: String,
    /// The day the account opened.
    pub opened_on: Option<Date>,
    /// The day the account closed.
    pub closed_on: Option<Date>,
    /// Commodity codes the account may hold, the first being its default.
    #[builder(default)]
    pub commodities: Vec<String>,
    /// Host-rendered `bc-` fields, such as the account kind. Print them
    /// verbatim as metadata lines.
    #[builder(default)]
    pub extra: Vec<MetaEntry>,
}

/// One commodity of the snapshot.
#[derive(bon::Builder, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportCommodity {
    /// The commodity code.
    #[builder(into)]
    pub code: String,
    /// Host-rendered `bc-` fields describing the commodity. Print them
    /// verbatim as metadata lines.
    #[builder(default)]
    pub extra: Vec<MetaEntry>,
}

/// One budget of the snapshot.
#[derive(bon::Builder, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportBudget {
    /// Account path the budget applies to.
    #[builder(into)]
    pub account: String,
    /// First day the budget applies.
    pub effective_from: Date,
    /// The period word, such as `monthly`, written verbatim.
    #[builder(into)]
    pub period: String,
    /// A fixed target amount, if the budget has one.
    pub target: Option<Amount>,
    /// A target expression, if the budget has one.
    #[builder(into)]
    pub expression: Option<String>,
    /// Host-rendered `bc-` fields, such as rollover and intent. Print them
    /// verbatim as metadata lines.
    #[builder(default)]
    pub extra: Vec<MetaEntry>,
}

/// A stated balance of an account on a date.
#[derive(bon::Builder, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportBalance {
    /// The day of the balance.
    pub date: Date,
    /// Account path.
    #[builder(into)]
    pub account: String,
    /// The stated balance.
    pub amount: Amount,
}

/// Everything one export covers.
#[derive(bon::Builder, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportSnapshot {
    /// How much bookkeeping the export carries.
    pub level: ExportLevel,
    /// Accounts, in the host's order.
    #[builder(default)]
    pub accounts: Vec<ExportAccount>,
    /// Commodities, in the host's order.
    #[builder(default)]
    pub commodities: Vec<ExportCommodity>,
    /// Transactions, in the host's order.
    #[builder(default)]
    pub transactions: Vec<ExportTransaction>,
    /// Budgets, in the host's order.
    #[builder(default)]
    pub budgets: Vec<ExportBudget>,
    /// Stated balances, in the host's order.
    #[builder(default)]
    pub balances: Vec<ExportBalance>,
}

/// One file an export produces.
#[derive(bon::Builder, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportFile {
    /// Path relative to the export root.
    #[builder(into)]
    pub path: String,
    /// The file's contents.
    #[builder(into)]
    pub contents: String,
}

/// Why an export produced nothing.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    /// The plugin does not export.
    #[error("this plugin does not export")]
    Unsupported,
    /// The format cannot spell these items; each entry names one.
    #[error("the format cannot spell {} item(s): {}", .0.len(), .0.join("; "))]
    Unspellable(Vec<String>),
    /// Any other failure, described for a person.
    #[error("export failed: {0}")]
    Other(String),
}

/// The trait an exporting plugin implements.
///
/// Pair it with `#[bc_sdk::importer(exporter)]` on the plugin's
/// [`Importer`](crate::Importer) impl to export it. Without the argument the
/// plugin exports an exporter that answers [`ExportError::Unsupported`].
pub trait Exporter {
    /// Render `snapshot` as files in this plugin's format.
    ///
    /// # Arguments
    ///
    /// * `snapshot` - Everything the export covers.
    ///
    /// # Returns
    ///
    /// The files to write, with paths relative to the export root.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Unspellable`] when the format cannot express
    /// part of the snapshot, and [`ExportError::Other`] for any other
    /// failure.
    fn export(&self, snapshot: ExportSnapshot) -> Result<Vec<ExportFile>, ExportError>;
}

// MARK: Wire conversions

/// Parses a wire decimal, keeping its stated scale.
fn decimal(value: &str) -> Result<Decimal, ExportError> {
    Decimal::from_str(value)
        .map_err(|e| ExportError::Other(format!("`{value}` is not a decimal number: {e}")))
}

/// Converts every element of a wire list.
fn all<W, S>(items: Vec<W>) -> Result<Vec<S>, ExportError>
where
    S: TryFrom<W, Error = ExportError>,
{
    items.into_iter().map(S::try_from).collect()
}

#[doc(hidden)]
impl From<WitDate> for Date {
    #[inline]
    fn from(d: WitDate) -> Self {
        Self::new(d.year, d.month, d.day)
    }
}

#[doc(hidden)]
impl From<WitReconciliation> for Reconciliation {
    #[inline]
    fn from(r: WitReconciliation) -> Self {
        match r {
            WitReconciliation::Unreconciled => Self::Unreconciled,
            WitReconciliation::Flagged => Self::Flagged,
            WitReconciliation::Reconciled => Self::Reconciled,
        }
    }
}

#[doc(hidden)]
impl TryFrom<WitAmount> for Amount {
    type Error = ExportError;

    #[inline]
    fn try_from(a: WitAmount) -> Result<Self, ExportError> {
        Ok(Self::new(decimal(&a.value)?, a.commodity))
    }
}

#[doc(hidden)]
impl TryFrom<WitQuote> for Quote {
    type Error = ExportError;

    #[inline]
    fn try_from(q: WitQuote) -> Result<Self, ExportError> {
        Ok(match q {
            WitQuote::PerUnit(amount) => Self::PerUnit(amount.try_into()?),
            WitQuote::Total(amount) => Self::Total(amount.try_into()?),
        })
    }
}

#[doc(hidden)]
impl TryFrom<WitCost> for Cost {
    type Error = ExportError;

    #[inline]
    fn try_from(c: WitCost) -> Result<Self, ExportError> {
        Ok(Self::builder()
            .basis(c.basis.try_into()?)
            .maybe_date(c.date.map(Into::into))
            .maybe_label(c.label)
            .build())
    }
}

#[doc(hidden)]
impl TryFrom<WitMetaValue> for MetaValue {
    type Error = ExportError;

    #[inline]
    fn try_from(v: WitMetaValue) -> Result<Self, ExportError> {
        Ok(match v {
            WitMetaValue::Text(text) => Self::Text(text),
            WitMetaValue::Number(number) => Self::Number(decimal(&number)?),
            WitMetaValue::Boolean(flag) => Self::Boolean(flag),
            WitMetaValue::Date(date) => Self::Date(date.into()),
            WitMetaValue::Timestamp(stamp) => Self::Timestamp(stamp),
            WitMetaValue::Amount(amount) => Self::Amount(amount.try_into()?),
            WitMetaValue::Account(path) => Self::Account(path),
        })
    }
}

#[doc(hidden)]
impl TryFrom<WitMetaEntry> for MetaEntry {
    type Error = ExportError;

    #[inline]
    fn try_from(e: WitMetaEntry) -> Result<Self, ExportError> {
        Ok(Self::new(e.key, e.value.try_into()?))
    }
}

#[doc(hidden)]
impl From<WitExportLevel> for ExportLevel {
    #[inline]
    fn from(l: WitExportLevel) -> Self {
        match l {
            WitExportLevel::Working => Self::Working,
            WitExportLevel::Archive => Self::Archive,
        }
    }
}

#[doc(hidden)]
impl TryFrom<WitExportPosting> for ExportPosting {
    type Error = ExportError;

    fn try_from(p: WitExportPosting) -> Result<Self, ExportError> {
        Ok(Self::builder()
            .maybe_handle(p.handle)
            .account(p.account)
            .maybe_amount(p.amount.map(Amount::try_from).transpose()?)
            .maybe_price(p.price.map(Quote::try_from).transpose()?)
            .maybe_cost(p.cost.map(Cost::try_from).transpose()?)
            .tags(p.tags)
            .metadata(all(p.metadata)?)
            .maybe_spread_from(p.spread_from.map(Into::into))
            .maybe_spread_until(p.spread_until.map(Into::into))
            .build())
    }
}

#[doc(hidden)]
impl TryFrom<WitExportTransaction> for ExportTransaction {
    type Error = ExportError;

    fn try_from(t: WitExportTransaction) -> Result<Self, ExportError> {
        Ok(Self::builder()
            .handle(t.handle)
            .date(t.date.into())
            .description(t.description)
            .reconciliation(t.reconciliation.into())
            .tags(t.tags)
            .metadata(all(t.metadata)?)
            .postings(all(t.postings)?)
            .unbalanced(t.unbalanced)
            .build())
    }
}

#[doc(hidden)]
impl TryFrom<WitExportAccount> for ExportAccount {
    type Error = ExportError;

    fn try_from(a: WitExportAccount) -> Result<Self, ExportError> {
        Ok(Self::builder()
            .path(a.path)
            .maybe_opened_on(a.opened_on.map(Into::into))
            .maybe_closed_on(a.closed_on.map(Into::into))
            .commodities(a.commodities)
            .extra(all(a.extra)?)
            .build())
    }
}

#[doc(hidden)]
impl TryFrom<WitExportCommodity> for ExportCommodity {
    type Error = ExportError;

    fn try_from(c: WitExportCommodity) -> Result<Self, ExportError> {
        Ok(Self::builder().code(c.code).extra(all(c.extra)?).build())
    }
}

#[doc(hidden)]
impl TryFrom<WitExportBudget> for ExportBudget {
    type Error = ExportError;

    fn try_from(b: WitExportBudget) -> Result<Self, ExportError> {
        Ok(Self::builder()
            .account(b.account)
            .effective_from(b.effective_from.into())
            .period(b.period)
            .maybe_target(b.target.map(Amount::try_from).transpose()?)
            .maybe_expression(b.expression)
            .extra(all(b.extra)?)
            .build())
    }
}

#[doc(hidden)]
impl TryFrom<WitExportBalance> for ExportBalance {
    type Error = ExportError;

    fn try_from(b: WitExportBalance) -> Result<Self, ExportError> {
        Ok(Self::builder()
            .date(b.date.into())
            .account(b.account)
            .amount(b.amount.try_into()?)
            .build())
    }
}

#[doc(hidden)]
impl TryFrom<WitExportSnapshot> for ExportSnapshot {
    type Error = ExportError;

    fn try_from(s: WitExportSnapshot) -> Result<Self, ExportError> {
        Ok(Self::builder()
            .level(s.level.into())
            .accounts(all(s.accounts)?)
            .commodities(all(s.commodities)?)
            .transactions(all(s.transactions)?)
            .budgets(all(s.budgets)?)
            .balances(all(s.balances)?)
            .build())
    }
}

#[doc(hidden)]
impl From<ExportFile> for WitExportFile {
    #[inline]
    fn from(f: ExportFile) -> Self {
        Self {
            path: f.path,
            contents: f.contents,
        }
    }
}

#[doc(hidden)]
impl From<ExportError> for WitExportError {
    #[inline]
    fn from(e: ExportError) -> Self {
        match e {
            ExportError::Unsupported => Self::Unsupported,
            ExportError::Unspellable(items) => Self::Unspellable(items),
            ExportError::Other(detail) => Self::Other(detail),
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    fn wit_amount(value: &str) -> WitAmount {
        WitAmount {
            value: value.to_owned(),
            commodity: "AUD".to_owned(),
        }
    }

    #[test]
    fn amount_keeps_trailing_zeros() {
        let amount = Amount::try_from(wit_amount("10.50")).expect("valid decimal");
        assert_eq!(amount.value.to_string(), "10.50");
        assert_eq!(amount.value, dec!(10.50));
    }

    #[test]
    fn malformed_amount_is_an_other_error() {
        let err = Amount::try_from(wit_amount("ten")).expect_err("not a decimal");
        assert!(matches!(err, ExportError::Other(_)), "got {err:?}");
    }

    #[test]
    fn snapshot_converts_from_the_wire() {
        let wire = WitExportSnapshot {
            level: WitExportLevel::Archive,
            accounts: vec![WitExportAccount {
                path: "Assets:Bank:Checking".to_owned(),
                opened_on: Some(WitDate {
                    year: 2025_i32,
                    month: 1_u8,
                    day: 2_u8,
                }),
                closed_on: None,
                commodities: vec!["AUD".to_owned()],
                extra: vec![WitMetaEntry {
                    key: "bc-kind".to_owned(),
                    value: WitMetaValue::Text("asset".to_owned()),
                }],
            }],
            commodities: vec![],
            transactions: vec![WitExportTransaction {
                handle: "tx-1".to_owned(),
                date: WitDate {
                    year: 2025_i32,
                    month: 3_u8,
                    day: 4_u8,
                },
                description: "Supermarket".to_owned(),
                reconciliation: WitReconciliation::Flagged,
                tags: vec![],
                metadata: vec![],
                postings: vec![WitExportPosting {
                    handle: None,
                    account: "Assets:Bank:Checking".to_owned(),
                    amount: Some(wit_amount("-12.50")),
                    price: None,
                    cost: None,
                    tags: vec![],
                    metadata: vec![],
                    spread_from: None,
                    spread_until: None,
                }],
                unbalanced: true,
            }],
            budgets: vec![],
            balances: vec![],
        };
        let snapshot = ExportSnapshot::try_from(wire).expect("well-formed snapshot");
        assert_eq!(snapshot.level, ExportLevel::Archive);
        let [account] = snapshot.accounts.as_slice() else {
            panic!("expected one account, got {}", snapshot.accounts.len());
        };
        assert_eq!(account.opened_on, Some(Date::new(2025, 1, 2)));
        let [transaction] = snapshot.transactions.as_slice() else {
            panic!(
                "expected one transaction, got {}",
                snapshot.transactions.len()
            );
        };
        assert_eq!(transaction.reconciliation, Reconciliation::Flagged);
        assert!(transaction.unbalanced);
        let [posting] = transaction.postings.as_slice() else {
            panic!("expected one posting, got {}", transaction.postings.len());
        };
        assert_eq!(posting.amount, Some(Amount::new(dec!(-12.50), "AUD")));
    }

    #[test]
    fn error_crosses_the_wire_intact() {
        let wire = WitExportError::from(ExportError::Unspellable(vec!["a".to_owned()]));
        assert!(matches!(wire, WitExportError::Unspellable(items) if items == ["a"]));
    }
}
