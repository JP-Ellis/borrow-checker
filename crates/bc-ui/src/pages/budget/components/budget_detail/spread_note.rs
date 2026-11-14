use bc_ipc::Amount;
use bc_ipc::BudgetRowTransaction;
use jiff::civil::Date;

/// The parenthetical after a spread posting's share: its full amount and its
/// spread range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpreadNote {
    /// The posting's full amount.
    pub amount: Amount,
    /// The spread range, as `1 Jan–31 Dec`; each end carries its year unless
    /// both fall in the window's year.
    pub range: String,
}

/// The note for `row` when the row counted exactly one posting and that
/// posting has a valid spread: both ends set and `from <= until`.
///
/// # Arguments
///
/// * `row` - One transaction of a budget row's list.
/// * `window_year` - The year of the window in view.
///
/// # Returns
///
/// The note, or `None` when the row counted several postings, the posting
/// is unspread, one-ended or inverted, or its amount is not a single value.
#[must_use]
pub(crate) fn spread_note(row: &BudgetRowTransaction, window_year: i16) -> Option<SpreadNote> {
    let mut counted = row
        .transaction
        .postings
        .iter()
        .filter(|p| row.counted.contains(&p.id));
    let posting = counted.next()?;
    if counted.next().is_some() {
        return None;
    }
    let (from, until) = posting.spread_from.zip(posting.spread_until)?;
    if from > until {
        return None;
    }
    let amount = posting.amount.display_amount()?.clone();
    let with_year = from.year() != window_year || until.year() != window_year;
    let end = |d: Date| {
        if with_year {
            d.strftime("%-d %b %Y").to_string()
        } else {
            d.strftime("%-d %b").to_string()
        }
    };
    Some(SpreadNote {
        amount,
        range: format!("{}\u{2013}{}", end(from), end(until)),
    })
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::AccountRef;
    use bc_ipc::Posting;
    use bc_ipc::PostingAmount;
    use bc_ipc::Reconciliation;
    use bc_ipc::Transaction;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::*;

    fn d(y: i16, m: i8, day: i8) -> Date {
        Date::constant(y, m, day)
    }

    fn aud(minor: i64) -> Amount {
        Amount::new(Decimal::new(minor, 2), "AUD")
    }

    fn posting(id: &str, amount: Amount, spread: (Option<Date>, Option<Date>)) -> Posting {
        Posting::new(
            id,
            AccountRef::new(id, id),
            PostingAmount::Stored(amount),
            vec![],
            vec![],
            spread.0,
            spread.1,
        )
    }

    fn row(postings: Vec<Posting>, counted: &[&str]) -> BudgetRowTransaction {
        BudgetRowTransaction::new(
            Transaction::new(
                "tx-1",
                d(2026, 1, 1),
                "Annual premium",
                vec![],
                Reconciliation::Reconciled,
                vec![],
                postings,
                vec![],
                true,
            ),
            None,
            false,
            counted.iter().map(|&c| c.to_owned()).collect(),
            Some(aud(10_192)),
            vec![],
        )
    }

    /// The 1,200 AUD bill and its bank leg, the bill spread over `spread`.
    fn bill(spread: (Option<Date>, Option<Date>)) -> Vec<Posting> {
        vec![
            posting("bill", aud(120_000), spread),
            posting("bank", aud(-120_000), (None, None)),
        ]
    }

    #[test]
    fn a_spread_in_the_window_year_omits_years() {
        let got = spread_note(
            &row(
                bill((Some(d(2026, 1, 1)), Some(d(2026, 12, 31)))),
                &["bill"],
            ),
            2026,
        );
        assert_eq!(
            got,
            Some(SpreadNote {
                amount: aud(120_000),
                range: "1 Jan\u{2013}31 Dec".to_owned(),
            })
        );
    }

    #[test]
    fn a_spread_across_years_names_them() {
        let got = spread_note(
            &row(
                bill((Some(d(2026, 12, 1)), Some(d(2027, 11, 30)))),
                &["bill"],
            ),
            2027,
        );
        assert_eq!(
            got.map(|n| n.range),
            Some("1 Dec 2026\u{2013}30 Nov 2027".to_owned())
        );
    }

    #[rstest]
    #[case::unspread((None, None))]
    #[case::one_ended((Some(d(2026, 1, 1)), None))]
    #[case::inverted((Some(d(2026, 12, 31)), Some(d(2026, 1, 1))))]
    fn an_invalid_spread_has_no_note(#[case] spread: (Option<Date>, Option<Date>)) {
        assert_eq!(spread_note(&row(bill(spread), &["bill"]), 2026), None);
    }

    #[test]
    fn several_counted_postings_have_no_note() {
        let spread = (Some(d(2026, 1, 1)), Some(d(2026, 12, 31)));
        let postings = vec![
            posting("bill", aud(60_000), spread),
            posting("levy", aud(60_000), spread),
            posting("bank", aud(-120_000), (None, None)),
        ];
        assert_eq!(spread_note(&row(postings, &["bill", "levy"]), 2026), None);
    }
}
