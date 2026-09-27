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

    #[test]
    fn amend_title_names_the_date() {
        assert_eq!(
            amend_title(jiff::civil::Date::constant(2026, 7, 1)),
            "Amend revision from 2026-07-01"
        );
    }
}
