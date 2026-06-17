//! QA page for [`super::StickyBar`].

use bc_ipc::Amount;
use bc_ipc::BcError;
use bc_ipc::BudgetSummary;
use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;

use super::StickyBar;

/// Creates a fixture [`BudgetSummary`] for QA display.
fn sample_summary() -> BudgetSummary {
    BudgetSummary::new(
        Some(Amount::new(500_000, "AUD", 2)),
        Some(Amount::new(312_450, "AUD", 2)),
        Some(Amount::new(187_550, "AUD", 2)),
        false,
        0,
    )
}

/// Renders [`StickyBar`] with fixture data in stub state.
///
/// Note: this component is a stub. The QA page will be expanded when the
/// component is implemented in a later task.
#[component]
pub fn StickyBarQa() -> impl IntoView {
    let summary = sample_summary();
    let overview: LocalResource<Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError>> =
        LocalResource::new(move || {
            let s = summary.clone();
            async move { Ok::<_, BcError>((s, vec![])) }
        });

    view! {
        <div style="padding:24px">
            <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                "stub — will be expanded when StickyBar is implemented"
            </p>
            <StickyBar overview=overview />
        </div>
    }
}
