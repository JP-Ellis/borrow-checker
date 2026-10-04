//! Pure filter-shaping helpers for the budget page (native-testable).

use bc_ipc::Filter;
use bc_query::print;
use bc_query::shape;

use crate::filter_ctx::query_expr;

/// The filter the budget sends: the query without its top-level `date` and
/// balance-`status` conjuncts, and no window (budgets follow `PeriodNav`).
///
/// # Arguments
///
/// * `user` - The active global filter.
#[must_use]
pub fn budget_effective_filter(user: &Filter) -> Filter {
    let kept = query_expr(user).and_then(|expr| shape::budget_query(&expr).kept);
    Filter::new(
        kept.map(|expr| print(&expr)).unwrap_or_default(),
        None,
        None,
    )
}

/// The inert-filter hint, or `None` when nothing is inert.
///
/// # Arguments
///
/// * `filter` - The active global filter.
#[must_use]
pub fn inert_hint(filter: &Filter) -> Option<String> {
    let split = shape::budget_query(&query_expr(filter)?);
    let mut sentences: Vec<String> = Vec::new();
    if !split.stripped.is_empty() {
        sentences.push(format!(
            "Date and balance filters don\u{2019}t apply to budgets \u{2014} ignoring {}; using the selected period.",
            split.stripped.join(" ")
        ));
    }
    if split.nested {
        sentences.push(
            "Date and balance terms inside or, - or any:(\u{2026}) still apply to budgets."
                .to_owned(),
        );
    }
    (!sentences.is_empty()).then(|| sentences.join(" "))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::Filter;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::budget_effective_filter;
    use super::inert_hint;

    #[rstest]
    #[case("date:>=2026-06-01 rent", "rent")]
    #[case("status:unbalanced rent", "rent")]
    #[case("date:2026", "")]
    #[case("rent or date:2026", "rent or date:2026")]
    #[case("status:reconciled", "status:reconciled")]
    fn strips_top_level_inert_terms(#[case] query: &str, #[case] sent: &str) {
        let eff = budget_effective_filter(&Filter::new(query, None, None));
        assert_eq!(eff, Filter::new(sent, None, None));
    }

    #[test]
    fn the_hint_names_what_was_stripped_and_what_still_applies() {
        assert_eq!(inert_hint(&Filter::new("rent", None, None)), None);
        assert_eq!(
            inert_hint(&Filter::new("date:>=2026-01-01 rent", None, None)).as_deref(),
            Some(
                "Date and balance filters don\u{2019}t apply to budgets \u{2014} ignoring date:>=2026-01-01; using the selected period."
            )
        );
        assert_eq!(
            inert_hint(&Filter::new("rent or status:unbalanced", None, None)).as_deref(),
            Some("Date and balance terms inside or, - or any:(\u{2026}) still apply to budgets.")
        );
    }
}
