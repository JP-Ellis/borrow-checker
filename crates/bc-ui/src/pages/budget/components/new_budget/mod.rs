//! Form for creating a new budget (first revision, Exact date).

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::AccountNode;
use bc_ipc::BcError;
use bc_ipc::BudgetIntent;
use bc_ipc::RolloverPolicy;
use leptos::prelude::*;
use stylance::import_style;

use crate::pages::budget::components::revision_form::TargetPreview;
use crate::pages::budget::components::revision_form::target;
use crate::pages::budget::keys::default_intent_for;
use crate::pages::budget::keys::intent_from_key;
use crate::pages::budget::keys::intent_key;
use crate::pages::budget::keys::period_from_key;
use crate::pages::budget::keys::rollover_from_key;
use crate::pages::budget::keys::rollover_key;

import_style!(style, "new_budget.module.scss");

/// Form for creating a new budget linked to an account.
///
/// Collects an account selection plus the first revision's fields
/// (name, target, currency, period, rollover, tag filter) and calls
/// [`bc_ipc::client::create_budget`] with `effective_from = today`.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "large view! block combining account picker and all revision fields"
)]
pub fn NewBudget(
    /// Invoked after the budget is successfully created.
    on_created: Callback<()>,
    /// Invoked when the user cancels.
    on_cancel: Callback<()>,
) -> impl IntoView {
    let accounts: LocalResource<Result<Vec<AccountNode>, BcError>> =
        LocalResource::new(move || async move { bc_ipc::client::list_accounts().await });

    let account_input = RwSignal::new(String::new());
    let name_input = RwSignal::new(String::new());
    let target_input = RwSignal::new(String::new());
    let currency_input = RwSignal::new("AUD".to_owned());
    let period_input = RwSignal::new("monthly".to_owned());
    let rollover_input = RwSignal::new(RolloverPolicy::ResetToZero);
    let intent_input = RwSignal::new(BudgetIntent::Goal);
    let tag_input = RwSignal::new(String::new());
    let saving = RwSignal::new(false);
    let error: RwSignal<Option<String>> = RwSignal::new(None);

    let save = move |_| {
        let account_id = account_input.get_untracked();
        if account_id.trim().is_empty() {
            error.set(Some("Please select an account".to_owned()));
            return;
        }

        let name = name_input.get_untracked();
        let name_opt = (!name.trim().is_empty()).then_some(name);
        let target_raw = target_input.get_untracked();
        let target_trim = target_raw.trim();
        if let Err(msg) = target::preview(target_trim) {
            error.set(Some(msg));
            return;
        }
        let target = (!target_trim.is_empty()).then(|| target_trim.to_owned());
        let currency = currency_input.get_untracked();
        let target_currency = target.is_some().then_some(currency);
        let rollover = rollover_input.get_untracked();
        let period = period_from_key(&period_input.get_untracked());
        let intent = intent_input.get_untracked();
        let tag = tag_input.get_untracked();
        let tag_opt = (!tag.trim().is_empty()).then_some(tag);

        /* Client-side mirror of the CapAtTarget invariant. */
        if rollover == RolloverPolicy::CapAtTarget && target.is_none() {
            error.set(Some("Cap at target requires a target amount".to_owned()));
            return;
        }

        saving.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            let effective_from = jiff::Zoned::now().date();
            let result = bc_ipc::client::create_budget(
                &account_id,
                effective_from,
                name_opt.as_deref(),
                target.as_deref(),
                target_currency.as_deref(),
                Some(intent),
                period,
                rollover,
                tag_opt.as_deref(),
            )
            .await;
            saving.set(false);
            match result {
                Ok(_warnings) => on_created.run(()),
                Err(e) => error.set(Some(e.to_string())),
            }
        });
    };

    view! {
        <div class=style::form aria-label="new budget">
            <div class=style::title>"New budget"</div>

            <div class=style::row>
                <span class=style::label>"Account"</span>
                <Suspense fallback=move || {
                    view! {
                        <select class=style::input disabled=true>
                            <option>"Loading\u{2026}"</option>
                        </select>
                    }
                }>
                    {move || {
                        accounts
                            .get()
                            .map(|result| match result {
                                Err(e) => {
                                    view! {
                                        <select class=style::input disabled=true>
                                            <option>{format!("Error: {e}")}</option>
                                        </select>
                                    }
                                        .into_any()
                                }
                                Ok(nodes) => {
                                    let nodes_for_change = nodes.clone();
                                    view! {
                                        <select
                                            class=style::input
                                            on:change=move |ev| {
                                                let v = event_target_value(&ev);
                                                if let Some(a) = nodes_for_change.iter().find(|a| a.id == v)
                                                {
                                                    intent_input.set(default_intent_for(a.account_type));
                                                }
                                                account_input.set(v);
                                            }
                                            prop:value=move || account_input.get()
                                        >
                                            <option value="">"— select account —"</option>
                                            {nodes
                                                .into_iter()
                                                .map(|a| {
                                                    view! {
                                                        <option value=a.id.clone()>{a.name.clone()}</option>
                                                    }
                                                })
                                                .collect::<Vec<_>>()}
                                        </select>
                                    }
                                        .into_any()
                                }
                            })
                    }}
                </Suspense>
            </div>

            <div class=style::row>
                <span class=style::label>"Name"</span>
                <input
                    type="text"
                    class=style::input
                    prop:value=move || name_input.get()
                    on:input=move |ev| name_input.set(event_target_value(&ev))
                />
            </div>

            <div class=style::row>
                <span class=style::label>"Target"</span>
                <input
                    type="text"
                    aria-label="target amount"
                    class=style::input
                    prop:value=move || target_input.get()
                    on:input=move |ev| target_input.set(event_target_value(&ev))
                />
                <input
                    type="text"
                    class=style::input
                    style="max-width:72px"
                    prop:value=move || currency_input.get()
                    on:input=move |ev| currency_input.set(event_target_value(&ev))
                />
            </div>

            <TargetPreview text=target_input.read_only() currency=currency_input.read_only() />

            <div class=style::row>
                <span class=style::label>"Intent"</span>
                <select
                    class=style::input
                    on:change=move |ev| {
                        intent_input.set(intent_from_key(&event_target_value(&ev)));
                    }
                    prop:value=move || intent_key(intent_input.get())
                >
                    <option value="limit">"Limit"</option>
                    <option value="goal">"Goal"</option>
                    <option value="estimate">"Estimate"</option>
                </select>
            </div>

            <div class=style::row>
                <span class=style::label>"Period"</span>
                <select
                    class=style::input
                    on:change=move |ev| period_input.set(event_target_value(&ev))
                    prop:value=move || period_input.get()
                >
                    <option value="daily">"daily"</option>
                    <option value="weekly">"weekly"</option>
                    <option value="fortnightly">"fortnightly"</option>
                    <option value="monthly">"monthly"</option>
                    <option value="quarterly">"quarterly"</option>
                    <option value="calendar_year">"calendar year"</option>
                </select>
            </div>

            <div class=style::row>
                <span class=style::label>"Rollover"</span>
                <select
                    class=style::input
                    on:change=move |ev| {
                        rollover_input.set(rollover_from_key(&event_target_value(&ev)));
                    }
                    prop:value=move || rollover_key(rollover_input.get())
                >
                    <option value="reset_to_zero">"Reset to zero"</option>
                    <option value="carry_forward">"Carry forward"</option>
                    <option value="cap_at_target">"Cap at target"</option>
                </select>
            </div>

            <div class=style::row>
                <span class=style::label>"Tag filter"</span>
                <input
                    type="text"
                    class=style::input
                    placeholder="tag id (optional)"
                    prop:value=move || tag_input.get()
                    on:input=move |ev| tag_input.set(event_target_value(&ev))
                />
            </div>

            {move || error.get().map(|m| view! { <p class=style::err>{m}</p> })}

            <div class=style::btn_row>
                <button class=style::save disabled=move || saving.get() on:click=save>
                    {move || if saving.get() { "Saving\u{2026}" } else { "Create" }}
                </button>
                <button class=style::cancel on:click=move |_| on_cancel.run(())>
                    "Cancel"
                </button>
            </div>
        </div>
    }
}
