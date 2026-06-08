//! Inline form for creating a new double-entry transaction.

use bc_ipc::AccountNode;
use bc_ipc::Amount;
use bc_ipc::NewPosting;
use bc_ipc::NewTransaction;
use bc_ipc::TxStatus;
use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "add_transaction.module.scss");

/// Parses a decimal string into an [`Amount`].
///
/// Trims whitespace, parses as `f64`, multiplies by `10^scale`, rounds to
/// `i64`.  Returns `None` if the input is empty, not a valid number, or rounds
/// to zero.
///
/// # Arguments
///
/// * `input` - User-entered decimal string (e.g. `"-84.20"`).
/// * `currency_code` - ISO 4217 code for the resulting [`Amount`].
/// * `scale` - Number of decimal places (e.g. `2` for AUD cents).
#[expect(
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    reason = "f64 rounded value cast to i64; truncation is acceptable for minor-unit conversion"
)]
#[expect(
    clippy::float_arithmetic,
    reason = "minor-unit conversion requires multiplication and rounding"
)]
fn parse_amount(input: &str, currency_code: &str, scale: u8) -> Option<Amount> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value: f64 = trimmed.parse().ok()?;
    let factor = 10_f64.powi(i32::from(scale));
    let minor = (value * factor).round() as i64;
    if minor == 0 {
        return None;
    }
    Some(Amount::new(minor, currency_code, scale))
}

/// Inline form for creating a new double-entry transaction from the current account.
///
/// The form produces a [`NewTransaction`] with exactly two postings: one for the
/// current account (amount as entered) and one for the selected offset account
/// (negated amount).
///
/// # Arguments
///
/// * `accounts` - Full account list for the offset-account dropdown.
/// * `current_account_id` - The account whose register is currently open (first posting).
/// * `currency_code` - ISO 4217 code inferred from the current account's balance.
/// * `scale` - Decimal scale of the current account's currency (e.g. `2` for AUD).
/// * `on_submit` - Called with the completed [`NewTransaction`] when the user submits.
/// * `on_cancel` - Called when the user clicks "cancel".
/// * `submit_error` - Optional error message to display beneath the submit button.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos component props require owned types"
)]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn AddTransactionForm(
    /// All accounts available for the offset posting dropdown.
    accounts: Vec<AccountNode>,
    /// The account whose page is currently open (receives the first posting).
    #[prop(into)]
    current_account_id: String,
    /// Currency code of the current account, e.g. `"AUD"`. Used for [`Amount`] construction.
    #[prop(into)]
    currency_code: String,
    /// Decimal scale of the current account's currency (e.g. `2` for AUD cents).
    scale: u8,
    /// Called with the completed [`NewTransaction`] when the user submits.
    on_submit: Callback<NewTransaction>,
    /// Called when the user cancels or closes the form.
    on_cancel: Callback<()>,
    /// If `Some(err_msg)`, renders the error beneath the submit button.
    /// Pass `None` when there is no IPC error to display.
    submit_error: Option<String>,
) -> impl IntoView {
    // Default date to today via js_sys::Date (wasm32-unknown-unknown only).
    let default_date: String = js_sys::Date::new_0()
        .to_iso_string()
        .as_string()
        .unwrap_or_default()
        .chars()
        .take(10)
        .collect();

    let default_offset_id = accounts
        .iter()
        .find(|a| a.id != current_account_id)
        .map(|a| a.id.clone())
        .unwrap_or_default();

    let date_input = RwSignal::new(default_date);
    let payee_input = RwSignal::new(String::new());
    let status_input = RwSignal::new(TxStatus::Pending);
    let amount_input = RwSignal::new(String::new());
    let offset_id_input = RwSignal::new(default_offset_id);
    let errors: RwSignal<Vec<&'static str>> = RwSignal::new(vec![]);

    let currency_code_submit = currency_code.clone();
    // Clone before the submit closure captures `current_account_id` by move,
    // so the same value remains available for building `offset_options` below.
    let current_account_id_submit = current_account_id.clone();

    let on_form_submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        errors.set(vec![]);
        let mut errs: Vec<&'static str> = vec![];

        let payee = payee_input.get();
        if payee.trim().is_empty() {
            errs.push("payee is required");
        }

        let date = date_input.get();
        if date.trim().is_empty() {
            errs.push("date is required");
        }

        let offset_id = offset_id_input.get();
        if offset_id.is_empty() {
            errs.push("offset account is required");
        }

        let amount_opt = parse_amount(&amount_input.get(), &currency_code_submit, scale);
        if amount_opt.is_none() {
            errs.push("amount must be a non-zero number");
        }

        if !errs.is_empty() {
            errors.set(errs);
            return;
        }

        // `amount_opt` is `Some` — the `is_none()` branch above would have
        // returned early.  Return early defensively if somehow `None`.
        let Some(amount) = amount_opt else {
            return;
        };
        let offset_amount = Amount::new(
            amount.minor_units.saturating_neg(),
            amount.currency_code.clone(),
            amount.scale,
        );

        let tx = NewTransaction::new(
            date,
            payee,
            status_input.get(),
            vec![],
            vec![
                NewPosting::new(current_account_id_submit.clone(), amount, None::<&str>),
                NewPosting::new(offset_id, offset_amount, None::<&str>),
            ],
        );
        on_submit.run(tx);
    };

    let on_cancel_header = move |_: leptos::ev::MouseEvent| {
        on_cancel.run(());
    };
    let on_cancel_footer = move |_: leptos::ev::MouseEvent| {
        on_cancel.run(());
    };

    // Build account options for the offset dropdown, excluding the current account.
    let offset_options: Vec<(String, String)> = accounts
        .iter()
        .filter(|a| a.id != current_account_id)
        .map(|a| (a.id.clone(), a.name.clone()))
        .collect();

    view! {
        <form class=style::form on:submit=on_form_submit>
            <div class=style::form_header>
                <span class=style::form_title>"new transaction"</span>
                <button
                    type="button"
                    class=style::close_btn
                    on:click=on_cancel_header
                    aria-label="cancel"
                >
                    "✕"
                </button>
            </div>

            <div class=style::form_grid>

                <label class=style::label for="atf-date">
                    "date"
                </label>
                <input
                    id="atf-date"
                    class=style::input
                    type="text"
                    placeholder="YYYY-MM-DD"
                    prop:value=move || date_input.get()
                    on:input=move |e| {
                        date_input.set(event_target_value(&e));
                    }
                />

                <label class=style::label for="atf-payee">
                    "payee"
                </label>
                <input
                    id="atf-payee"
                    class=style::input
                    type="text"
                    placeholder="payee name"
                    prop:value=move || payee_input.get()
                    on:input=move |e| {
                        payee_input.set(event_target_value(&e));
                    }
                />

                <label class=style::label for="atf-status">
                    "status"
                </label>
                <select
                    id="atf-status"
                    class=style::input
                    on:change=move |e| {
                        let val = event_target_value(&e);
                        status_input
                            .set(
                                if val == "cleared" { TxStatus::Cleared } else { TxStatus::Pending },
                            );
                    }
                >
                    <option
                        value="pending"
                        selected=move || { status_input.get() == TxStatus::Pending }
                    >
                        "pending"
                    </option>
                    <option
                        value="cleared"
                        selected=move || { status_input.get() == TxStatus::Cleared }
                    >
                        "cleared"
                    </option>
                </select>

                <label class=style::label for="atf-amount">
                    {format!("amount ({})", currency_code.clone())}
                </label>
                <input
                    id="atf-amount"
                    class=style::input
                    type="text"
                    placeholder="e.g. -84.20"
                    prop:value=move || amount_input.get()
                    on:input=move |e| {
                        amount_input.set(event_target_value(&e));
                    }
                />

                <label class=style::label for="atf-offset">
                    "offset account"
                </label>
                <select
                    id="atf-offset"
                    class=style::input
                    on:change=move |e| {
                        offset_id_input.set(event_target_value(&e));
                    }
                >
                    {offset_options
                        .into_iter()
                        .map(|(id, name)| {
                            let id_clone = id.clone();
                            view! {
                                <option value=id selected=move || offset_id_input.get() == id_clone>
                                    {name}
                                </option>
                            }
                        })
                        .collect::<Vec<_>>()}
                </select>
            </div>

            {move || {
                let errs = errors.get();
                if errs.is_empty() {
                    None
                } else {
                    Some(
                        view! {
                            <ul class=style::error_list>
                                {errs
                                    .into_iter()
                                    .map(|e| view! { <li>{e}</li> })
                                    .collect::<Vec<_>>()}
                            </ul>
                        },
                    )
                }
            }}

            {submit_error
                .map(|err| {
                    view! { <div class=style::ipc_error>{err}</div> }
                })}

            <div class=style::form_footer>
                <button type="button" class=style::cancel_btn on:click=on_cancel_footer>
                    "cancel"
                </button>
                <button type="submit" class=style::submit_btn>
                    "add transaction"
                </button>
            </div>
        </form>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
