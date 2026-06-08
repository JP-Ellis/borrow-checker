//! QA showcase for [`AddTransactionForm`] — rendered at `/__test/add-transaction-form`.

use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use bc_ipc::Amount;
use leptos::prelude::*;

use super::AddTransactionForm;

/// Sample account list used in the QA showcase.
#[expect(
    dead_code,
    reason = "used only within QA module, compiler may not see the call site"
)]
fn sample_accounts() -> Vec<AccountNode> {
    vec![
        AccountNode::new(
            "acc-checking",
            "Smart Access",
            Some("4421"),
            Amount::new(-150_000, "AUD", 2),
            None::<&str>,
            AccountType::Asset,
            vec![],
        ),
        AccountNode::new(
            "acc-groceries",
            "Groceries",
            None::<&str>,
            Amount::new(0, "AUD", 2),
            None::<&str>,
            AccountType::Expense,
            vec![],
        ),
        AccountNode::new(
            "acc-salary",
            "Salary",
            None::<&str>,
            Amount::new(0, "AUD", 2),
            None::<&str>,
            AccountType::Income,
            vec![],
        ),
    ]
}

/// QA page showcasing the add-transaction form in isolation.
#[component]
pub fn AddTransactionFormQa() -> impl IntoView {
    let last_submit = RwSignal::new(Option::<String>::None);
    let show_error = RwSignal::new(false);

    view! {
        <div style="padding: 2rem; max-width: 640px;">
            <h2 style="font-family: var(--bc-font-mono); margin-bottom: 1rem;">
                "AddTransactionForm QA"
            </h2>

            {move || {
                let err = show_error.get().then(|| "IPC error: backend unavailable".to_owned());
                view! {
                    <AddTransactionForm
                        accounts=sample_accounts()
                        current_account_id="acc-checking"
                        currency_code="AUD"
                        scale=2
                        on_submit=Callback::new(move |tx| {
                            last_submit.set(Some(format!("{tx:?}")));
                        })
                        on_cancel=Callback::new(|()| leptos::logging::log!("cancelled"))
                        submit_error=err
                    />
                }
            }}

            {move || {
                last_submit
                    .get()
                    .map(|s| {
                        view! {
                            <pre style="margin-top:1rem; font-size:0.75rem; \
                            white-space:pre-wrap; color: var(--bc-ink-soft);">{s}</pre>
                        }
                    })
            }}

            <button
                style="margin-top: 1rem; font-family: var(--bc-font-mono);"
                on:click=move |_| show_error.update(|v| *v = !*v)
            >
                "toggle IPC error"
            </button>
        </div>
    }
}
