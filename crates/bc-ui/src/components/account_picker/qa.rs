//! QA showcase for [`AccountPicker`](super::AccountPicker).

use bc_ipc::AccountRef;
use leptos::prelude::*;

use super::AccountPicker;

/// Returns a fixed list of accounts used across QA showcase instances.
fn sample_accounts() -> Vec<AccountRef> {
    vec![
        AccountRef::new("a1", "Assets :: Checking"),
        AccountRef::new("a2", "Assets :: Savings"),
        AccountRef::new("e1", "Expenses :: Groceries"),
        AccountRef::new("e2", "Expenses :: Dining"),
        AccountRef::new("i1", "Income :: Salary"),
    ]
}

/// Renders the picker against a fixed account list.
#[component]
pub fn AccountPickerQa() -> impl IntoView {
    let id = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());

    view! {
        <div style="max-width: 28rem; padding: 1rem; display: flex; flex-direction: column; gap: 2rem;">
            <section>
                <h3>"Account picker"</h3>
                <AccountPicker
                    accounts=sample_accounts()
                    selected_id=id
                    selected_name=name
                    on_pick=Callback::new(|_a: AccountRef| {})
                />
                <p>"selected id: " {move || id.get()}</p>
            </section>
        </div>
    }
}
