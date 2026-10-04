//! The collapsed row's header as data: everything it shows that depends
//! only on the transaction and the perspective.

use bc_ipc::Amount;
use bc_ipc::Quote;
use bc_ipc::Reconciliation;
use bc_ipc::Transaction;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::components::meta_editor::model::first_text_by_key;
use crate::components::transaction_row::RowPerspective;
use crate::components::transaction_row::counterpart_names;
use crate::components::transaction_row::headline_amount;
use crate::components::transaction_row::headline_price;
use crate::components::transaction_row::payee_initial;
use crate::label::category_label;

/// How the row's name renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameStyle {
    /// The payee, or the `—` placeholder.
    Payee,
    /// The description, standing in for a missing payee; rendered dimmed.
    Description,
}

/// Sign of the headline amount, which picks its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    /// Above zero.
    Positive,
    /// Below zero.
    Negative,
    /// Exactly zero.
    Zero,
}

/// Everything the collapsed row shows, minus formatting that reads other
/// state: currency display and the context year are applied at render.
#[expect(
    clippy::module_name_repetitions,
    reason = "`RowView` names the type the module exists to define"
)]
#[derive(Clone, Debug, PartialEq)]
pub struct RowView {
    /// Transaction date.
    pub date: Date,
    /// Payee, else description, else `—`.
    pub name: String,
    /// How `name` renders.
    pub name_style: NameStyle,
    /// Avatar letter.
    pub initial: char,
    /// Counterpart category label.
    pub category: String,
    /// Headline amount from the perspective's focal postings.
    pub amount: Amount,
    /// Sign of `amount`.
    pub sign: Sign,
    /// The one focal leg's price, if any.
    pub price: Option<Quote>,
    /// Transaction tag paths.
    pub tags: Vec<String>,
    /// Reconciliation is flagged.
    pub flagged: bool,
    /// Reconciliation is unreconciled.
    pub unreconciled: bool,
    /// The postings do not balance.
    pub unbalanced: bool,
    /// Posting count when there are more than two postings.
    pub split: Option<usize>,
}

impl RowView {
    /// Builds the header for `tx` seen from `perspective`.
    #[must_use]
    pub fn new(tx: &Transaction, perspective: &RowPerspective) -> Self {
        // `payee` is ordinary metadata; a flagged entry still reads as typed.
        let payee = first_text_by_key(&tx.metadata, "payee").filter(|text| !text.is_empty());
        let (name, name_style) = match payee {
            Some(p) => (p.to_owned(), NameStyle::Payee),
            None if !tx.description.is_empty() => (tx.description.clone(), NameStyle::Description),
            None => ("\u{2014}".to_owned(), NameStyle::Payee),
        };
        let amount = headline_amount(tx, perspective);
        let sign = match amount.value.cmp(&Decimal::ZERO) {
            core::cmp::Ordering::Greater => Sign::Positive,
            core::cmp::Ordering::Less => Sign::Negative,
            core::cmp::Ordering::Equal => Sign::Zero,
        };
        Self {
            date: tx.date,
            initial: payee_initial(payee.unwrap_or(&tx.description)),
            name,
            name_style,
            category: category_label(&counterpart_names(tx, perspective)),
            price: headline_price(tx, perspective),
            amount,
            sign,
            tags: tx.tags.iter().map(|t| t.path.clone()).collect(),
            flagged: tx.reconciliation == Reconciliation::Flagged,
            unreconciled: tx.reconciliation == Reconciliation::Unreconciled,
            unbalanced: !tx.balanced,
            split: (tx.postings.len() > 2).then_some(tx.postings.len()),
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::AccountRef;
    use bc_ipc::MetaEntryDto;
    use bc_ipc::MetaValueDto;
    use bc_ipc::Posting;
    use bc_ipc::PostingAmount;
    use bc_ipc::TagInfo;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    fn posting(acct: &str, minor: i64) -> Posting {
        Posting::new(
            format!("p-{acct}"),
            AccountRef::new(acct, acct),
            PostingAmount::Stored(Amount::new(Decimal::new(minor, 2), "AUD")),
            vec![],
            vec![],
            None,
            None,
        )
    }

    fn tx(
        description: &str,
        payee: Option<&str>,
        reconciliation: Reconciliation,
        postings: Vec<Posting>,
        balanced: bool,
    ) -> Transaction {
        let metadata = payee
            .map(|p| vec![MetaEntryDto::new("payee", MetaValueDto::Text(p.to_owned()))])
            .unwrap_or_default();
        Transaction::new(
            "tx-1",
            Date::constant(2026, 4, 30),
            description,
            metadata,
            reconciliation,
            vec![TagInfo::new("t-food", "food")],
            postings,
            vec![],
            balanced,
        )
    }

    fn checking() -> RowPerspective {
        RowPerspective::Account {
            account_ids: vec!["checking".to_owned()],
        }
    }

    fn pair() -> Vec<Posting> {
        vec![posting("checking", -1250), posting("groceries", 1250)]
    }

    #[rstest]
    #[case::payee(Some("Corner Shop"), "weekly", "Corner Shop", NameStyle::Payee, 'C')]
    #[case::description(None, "weekly shop", "weekly shop", NameStyle::Description, 'W')]
    #[case::empty_payee(Some(""), "weekly shop", "weekly shop", NameStyle::Description, 'W')]
    #[case::nothing(None, "", "\u{2014}", NameStyle::Payee, '?')]
    fn name_falls_back_from_payee_to_description_to_dash(
        #[case] payee: Option<&str>,
        #[case] description: &str,
        #[case] name: &str,
        #[case] style: NameStyle,
        #[case] initial: char,
    ) {
        let view = RowView::new(
            &tx(description, payee, Reconciliation::Reconciled, pair(), true),
            &checking(),
        );
        assert_eq!(
            (view.name.as_str(), view.name_style, view.initial),
            (name, style, initial)
        );
    }

    #[rstest]
    #[case::flagged(Reconciliation::Flagged, true, false)]
    #[case::unreconciled(Reconciliation::Unreconciled, false, true)]
    #[case::reconciled(Reconciliation::Reconciled, false, false)]
    fn reconciliation_sets_one_glyph(
        #[case] reconciliation: Reconciliation,
        #[case] flagged: bool,
        #[case] unreconciled: bool,
    ) {
        let view = RowView::new(&tx("x", None, reconciliation, pair(), true), &checking());
        assert_eq!((view.flagged, view.unreconciled), (flagged, unreconciled));
    }

    #[test]
    fn split_counts_postings_past_two() {
        let three = vec![
            posting("checking", -1500),
            posting("groceries", 1000),
            posting("dining", 500),
        ];
        assert_eq!(
            RowView::new(
                &tx("x", None, Reconciliation::Reconciled, three, true),
                &checking()
            )
            .split,
            Some(3)
        );
        assert_eq!(
            RowView::new(
                &tx("x", None, Reconciliation::Reconciled, pair(), true),
                &checking()
            )
            .split,
            None
        );
    }

    #[test]
    fn unbalanced_mirrors_the_balanced_flag() {
        assert!(
            RowView::new(
                &tx("x", None, Reconciliation::Reconciled, pair(), false),
                &checking()
            )
            .unbalanced
        );
        assert!(
            !RowView::new(
                &tx("x", None, Reconciliation::Reconciled, pair(), true),
                &checking()
            )
            .unbalanced
        );
    }

    #[rstest]
    #[case::outflow("checking", Sign::Negative)]
    #[case::inflow("groceries", Sign::Positive)]
    fn sign_follows_the_focal_amount(#[case] focal: &str, #[case] sign: Sign) {
        let perspective = RowPerspective::Account {
            account_ids: vec![focal.to_owned()],
        };
        let view = RowView::new(
            &tx("x", None, Reconciliation::Reconciled, pair(), true),
            &perspective,
        );
        assert_eq!(view.sign, sign);
    }

    #[test]
    fn rolled_up_focal_set_leaves_only_outside_counterparts() {
        let three = vec![
            posting("checking", -1500),
            posting("groceries", 1000),
            posting("dining", 500),
        ];
        let rolled = RowPerspective::Account {
            account_ids: vec!["groceries".to_owned(), "dining".to_owned()],
        };
        let view = RowView::new(
            &tx("x", None, Reconciliation::Reconciled, three, true),
            &rolled,
        );
        assert_eq!(view.category, "checking");
        assert_eq!(view.amount, Amount::new(Decimal::new(1500, 2), "AUD"));
    }

    #[test]
    fn single_account_category_is_the_lone_counterpart() {
        let view = RowView::new(
            &tx("x", None, Reconciliation::Reconciled, pair(), true),
            &checking(),
        );
        assert_eq!(view.category, "groceries");
    }

    #[test]
    fn view_carries_date_and_tags() {
        let view = RowView::new(
            &tx("x", None, Reconciliation::Reconciled, pair(), true),
            &checking(),
        );
        assert_eq!(view.date, Date::constant(2026, 4, 30));
        assert_eq!(view.tags, vec!["food".to_owned()]);
    }
}
