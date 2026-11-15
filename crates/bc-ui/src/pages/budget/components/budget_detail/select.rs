use bc_ipc::BudgetRevisionView;

/// `true` when the open editor is amending the revision with `id`.
///
/// Mirrors the editor's own `Option<Option<BudgetRevisionView>>` shape (`None`
/// = closed, `Some(None)` = add form, `Some(Some(rev))` = amending `rev`), so
/// it takes that type by reference rather than a two-case abstraction.
#[must_use]
#[expect(
    clippy::option_option,
    reason = "mirrors the editor signal's own None / Some(None) / Some(Some(rev)) shape"
)]
#[expect(
    clippy::ref_option,
    reason = "callers pass &editor.get(), an owned value; Option<&Option<_>> would force them to destructure first"
)]
pub(crate) fn revision_selected(editor: &Option<Option<BudgetRevisionView>>, id: &str) -> bool {
    matches!(editor, Some(Some(rev)) if rev.id == id)
}

/// Revision the add form copies its fields from: the one in force `on`, or the
/// earliest when `on` precedes them all.
///
/// # Arguments
///
/// * `revisions` - The budget's revisions, in any order.
/// * `on` - The add form's default effective date.
///
/// # Returns
///
/// The seed revision, or `None` when the budget has no revisions.
#[must_use]
pub(crate) fn add_seed(
    revisions: &[BudgetRevisionView],
    on: jiff::civil::Date,
) -> Option<&BudgetRevisionView> {
    revisions
        .iter()
        .filter(|rev| rev.effective_from <= on)
        .max_by_key(|rev| rev.effective_from)
        .or_else(|| revisions.iter().min_by_key(|rev| rev.effective_from))
}

/// Title for the amend form, naming which revision the save changes.
#[must_use]
pub(crate) fn amend_title(effective_from: jiff::civil::Date) -> String {
    format!("Amend revision from {effective_from}")
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::BudgetRevisionView;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[test]
    fn selection_tracks_the_edited_revision() {
        let rev = BudgetRevisionView::builder()
            .id("budget_rev_2")
            .effective_from(jiff::civil::Date::constant(2026, 7, 1))
            .period(bc_ipc::Period::Monthly)
            .period_label("monthly")
            .rollover(bc_ipc::RolloverPolicy::ResetToZero)
            .intent(bc_ipc::BudgetIntent::Limit)
            .build();
        let editing = Some(Some(rev));
        assert!(revision_selected(&editing, "budget_rev_2"));
        assert!(!revision_selected(&editing, "budget_rev_1"));
        assert!(!revision_selected(&Some(None), "budget_rev_2"));
        assert!(!revision_selected(&None, "budget_rev_2"));
    }

    fn rev_from(id: &str, effective_from: jiff::civil::Date) -> BudgetRevisionView {
        BudgetRevisionView::builder()
            .id(id)
            .effective_from(effective_from)
            .period(bc_ipc::Period::Monthly)
            .period_label("monthly")
            .rollover(bc_ipc::RolloverPolicy::ResetToZero)
            .intent(bc_ipc::BudgetIntent::Limit)
            .build()
    }

    #[rstest]
    #[case::governing_revision(2026, 8, 15, Some("budget_rev_jul"))]
    #[case::on_the_effective_date(2026, 10, 1, Some("budget_rev_oct"))]
    #[case::after_the_last(2027, 3, 1, Some("budget_rev_oct"))]
    #[case::before_the_first(2026, 1, 1, Some("budget_rev_jul"))]
    fn add_form_seeds_from_the_revision_in_force(
        #[case] year: i16,
        #[case] month: i8,
        #[case] day: i8,
        #[case] want: Option<&str>,
    ) {
        /* Deliberately out of order: the helper must not rely on sorting. */
        let revs = vec![
            rev_from("budget_rev_oct", jiff::civil::Date::constant(2026, 10, 1)),
            rev_from("budget_rev_jul", jiff::civil::Date::constant(2026, 7, 1)),
        ];
        let on = jiff::civil::Date::constant(year, month, day);
        assert_eq!(add_seed(&revs, on).map(|rev| rev.id.as_str()), want);
    }

    #[test]
    fn add_form_has_no_seed_without_revisions() {
        assert!(add_seed(&[], jiff::civil::Date::constant(2026, 7, 1)).is_none());
    }

    #[test]
    fn amend_title_names_the_date() {
        assert_eq!(
            amend_title(jiff::civil::Date::constant(2026, 7, 1)),
            "Amend revision from 2026-07-01"
        );
    }
}
