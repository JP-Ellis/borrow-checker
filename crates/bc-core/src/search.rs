//! Structured transaction search: query types, per-leg match attribution, and
//! the `Service::search` query surface.

use bc_models::AccountId;
use bc_models::CommodityCode;
use bc_models::Reconciliation;
use bc_models::TagId;
use jiff::civil::Date;
use rust_decimal::Decimal;

/// Magnitude predicate for the amount dimension (parsed from `bc_ipc::AmountFilter`).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct AmountQuery {
    /// Inclusive lower bound on the magnitude.
    pub min: Option<Decimal>,
    /// Inclusive upper bound on the magnitude.
    pub max: Option<Decimal>,
    /// Restrict to a single commodity when set.
    pub commodity: Option<CommodityCode>,
}

/// A parsed transaction query: `bc_ipc::Filter` with ids resolved to domain types.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct TransactionQuery {
    /// Inclusive lower date bound.
    pub date_from: Option<Date>,
    /// Exclusive upper date bound.
    pub date_until: Option<Date>,
    /// Account ids; each matches its subtree; multiple union.
    pub accounts: Vec<AccountId>,
    /// Tag ids; multiple union.
    pub tags: Vec<TagId>,
    /// Case-insensitive substring over payee OR narration.
    pub text: Option<String>,
    /// Magnitude predicate.
    pub amount: Option<AmountQuery>,
    /// Exact reconciliation status.
    pub reconciliation: Option<Reconciliation>,
}

#[cfg(test)]
#[cfg(feature = "ipc")]
mod tests {
    use bc_models::AccountId;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::TransactionQuery;

    #[test]
    fn try_from_filter_parses_ids_and_scalars() {
        let acc = AccountId::new();
        let mut filter = bc_ipc::Filter::default();
        filter.accounts = vec![acc.to_string()];
        filter.text = Some("coffee".to_owned());
        let mut amount_filter = bc_ipc::AmountFilter::default();
        amount_filter.min = Some(Decimal::new(5, 0));
        amount_filter.commodity = Some("AUD".to_owned());
        filter.amount = Some(amount_filter);

        let query = TransactionQuery::try_from(filter).expect("valid filter");
        assert_eq!(query.accounts, vec![acc]);
        assert_eq!(query.text.as_deref(), Some("coffee"));
        let amount = query.amount.expect("amount present");
        assert_eq!(amount.min, Some(Decimal::new(5, 0)));
        assert_eq!(
            amount.commodity.map(|c| c.as_str().to_owned()),
            Some("AUD".to_owned())
        );
    }

    #[test]
    fn try_from_filter_rejects_bad_account_id() {
        let mut filter = bc_ipc::Filter::default();
        filter.accounts = vec!["not-a-valid-id!!".to_owned()];
        let err = TransactionQuery::try_from(filter).expect_err("invalid account id must fail");
        assert!(matches!(err, crate::BcError::BadData(_)));
    }
}
