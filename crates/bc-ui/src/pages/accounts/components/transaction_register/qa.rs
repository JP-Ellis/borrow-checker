//! QA page for [`super::TransactionRegister`].

use leptos::prelude::*;

use super::TransactionRegister;
use crate::pages::accounts::types::TRANSACTIONS;

/// Renders [`TransactionRegister`] with full and empty data sets.
#[component]
pub fn TransactionRegisterQa() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "typical — Smart Access transactions (use j/k/Enter to navigate)"
                </p>
                <TransactionRegister
                    transactions=&*TRANSACTIONS
                    viewing_account_id="cb-smart-access"
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "empty — no transactions"
                </p>
                <TransactionRegister transactions=&[] viewing_account_id="cb-smart-access" />
            </section>

        </div>
    }
}
