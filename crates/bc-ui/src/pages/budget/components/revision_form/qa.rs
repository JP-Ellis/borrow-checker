//! QA showcase for [`RevisionForm`](super::RevisionForm).

use bc_ipc::Amount;
use bc_ipc::BudgetRevisionView;
use bc_ipc::Period;
use bc_ipc::RolloverPolicy;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::RevisionForm;

/// Renders add and amend variants of the revision form.
#[component]
pub fn RevisionFormQa() -> impl IntoView {
    let amend_sample = BudgetRevisionView::builder()
        .id("budget_rev_sample")
        .effective_from(jiff::civil::Date::constant(2027, 1, 1))
        .name("Groceries")
        .target(Amount::new(Decimal::new(25_000, 2), "AUD"))
        .period(Period::Weekly)
        .period_label("weekly")
        .rollover(RolloverPolicy::CarryForward)
        .build();

    let noop = Callback::new(|()| ());

    view! {
        <div style="display:flex;flex-direction:column;gap:24px;max-width:480px;padding:24px">
            <h3>"Add (first revision \u{2014} snap disabled)"</h3>
            <RevisionForm
                budget_id="budget_demo".to_owned()
                allow_snap=false
                on_saved=noop
                on_cancel=noop
            />
            <h3>"Amend (snap enabled)"</h3>
            <RevisionForm
                budget_id="budget_demo".to_owned()
                revision=amend_sample
                allow_snap=true
                on_saved=noop
                on_cancel=noop
            />
        </div>
    }
}
