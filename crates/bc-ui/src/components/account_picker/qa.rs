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
///
/// Two instances are shown: a plain picker and one wired with `on_create`
/// that logs created names to a signal.
#[component]
pub fn AccountPickerQa() -> impl IntoView {
    let id = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());

    let id2 = RwSignal::new(String::new());
    let name2 = RwSignal::new(String::new());
    let create_log: RwSignal<Vec<String>> = RwSignal::new(Vec::new());

    view! {
        <div style="max-width: 28rem; padding: 1rem; display: flex; flex-direction: column; gap: 2rem;">
            <section>
                <h3>"Plain picker (no on_create)"</h3>
                <AccountPicker
                    accounts=sample_accounts()
                    selected_id=id
                    selected_name=name
                    on_pick=Callback::new(|_a: AccountRef| {})
                />
                <p>"selected id: " {move || id.get()}</p>
            </section>

            <section>
                <h3>"Picker with on_create"</h3>
                <AccountPicker
                    accounts=sample_accounts()
                    selected_id=id2
                    selected_name=name2
                    on_pick=Callback::new(|_a: AccountRef| {})
                    on_create=Callback::new(move |created: String| {
                        create_log.update(|l| l.push(format!("Created: {created}")));
                    })
                />
                <p>"selected id2: " {move || id2.get()}</p>
                <ul>
                    {move || {
                        create_log
                            .get()
                            .into_iter()
                            .map(|entry| view! { <li>{entry}</li> })
                            .collect::<Vec<_>>()
                    }}
                </ul>
            </section>
        </div>
    }
}
