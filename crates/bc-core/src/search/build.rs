//! Builders for typed queries the engine assembles itself: the CLI report's
//! accounts and tags, and the old filter fields until the IPC filter carries
//! query text.

use bc_models::AccountId;
use bc_models::Reconciliation;
use bc_models::TagId;
use bc_query::filter::AmountPred;
use bc_query::filter::Bound;
use bc_query::filter::NumRange;
use bc_query::filter::Pred;
use bc_query::filter::ResolvedExpr;
use bc_query::filter::Status;
use bc_query::filter::TagPred;
use bc_query::filter::TextMatch;
use rust_decimal::Decimal;

use crate::BcError;
use crate::BcResult;

/// Legs on any of `ids` or beneath them; `None` when `ids` is empty.
pub(crate) fn accounts(ids: &[AccountId]) -> Option<ResolvedExpr> {
    any_of(
        ids.iter()
            .map(|id| {
                ResolvedExpr::Pred(Pred::Account {
                    id: id.to_string(),
                    subtree: true,
                })
            })
            .collect(),
    )
}

/// Legs carrying, or in a transaction carrying, any of exactly these tags;
/// `None` when `ids` is empty.
pub(crate) fn tags(ids: &[TagId]) -> Option<ResolvedExpr> {
    any_of(
        ids.iter()
            .map(|id| {
                ResolvedExpr::Pred(Pred::Tag(TagPred::Tag {
                    id: id.to_string(),
                    subtree: false,
                }))
            })
            .collect(),
    )
}

/// A description containing `needle`, ASCII-folded.
pub(crate) fn text(needle: &str) -> ResolvedExpr {
    ResolvedExpr::Pred(Pred::Description(TextMatch::Contains(
        needle.to_ascii_lowercase(),
    )))
}

/// A leg magnitude in `[min, max]`, in `commodity` when given.
pub(crate) fn amount(
    min: Option<Decimal>,
    max: Option<Decimal>,
    commodity: Option<&str>,
) -> ResolvedExpr {
    ResolvedExpr::Pred(Pred::Amount(AmountPred::new(
        NumRange::new(
            min.map(|v| Bound::new(v, true)),
            max.map(|v| Bound::new(v, true)),
        ),
        commodity.map(str::to_owned),
    )))
}

/// The transaction's reconciliation state.
///
/// # Errors
///
/// Returns [`BcError::InvalidInput`] for a state this builder does not know,
/// so a new state fails loudly instead of filtering as another.
pub(crate) fn reconciliation(state: Reconciliation) -> BcResult<ResolvedExpr> {
    let status = match state {
        Reconciliation::Unreconciled => Status::Unreconciled,
        Reconciliation::Flagged => Status::Flagged,
        Reconciliation::Reconciled => Status::Reconciled,
        unknown => {
            return Err(BcError::InvalidInput(format!(
                "unknown reconciliation state {unknown:?}"
            )));
        }
    };
    Ok(ResolvedExpr::Pred(Pred::Status(status)))
}

/// Whether the transaction balances.
pub(crate) const fn balanced(want: bool) -> ResolvedExpr {
    ResolvedExpr::Pred(Pred::Status(if want {
        Status::Balanced
    } else {
        Status::Unbalanced
    }))
}

/// `items` joined by `or`; `None` when empty, the item itself when alone.
pub(crate) fn any_of(mut items: Vec<ResolvedExpr>) -> Option<ResolvedExpr> {
    if items.len() > 1 {
        return Some(ResolvedExpr::Or(items));
    }
    items.pop()
}

/// `items` joined by `and`; `None` when empty, the item itself when alone.
pub(crate) fn all_of(mut items: Vec<ResolvedExpr>) -> Option<ResolvedExpr> {
    if items.len() > 1 {
        return Some(ResolvedExpr::And(items));
    }
    items.pop()
}
