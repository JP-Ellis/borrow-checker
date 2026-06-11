//! Inline form for creating a new double-entry transaction.

use bc_ipc::AccountNode;
use bc_ipc::Amount;
use bc_ipc::NewPosting;
use bc_ipc::NewTransaction;
use bc_ipc::TxStatus;
use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "add_transaction.module.scss");

/// Locale-aware JS helpers for decimal and grouping separators.
///
/// Uses `Intl.NumberFormat().formatToParts()` to detect the user's locale
/// separators at runtime.  `inline_js` generates static compiled JavaScript
/// — no dynamic `eval` — so it is safe under Tauri's default CSP.
#[cfg(target_arch = "wasm32")]
mod locale_js {
    use wasm_bindgen::prelude::wasm_bindgen;

    #[wasm_bindgen(inline_js = "
        export function locale_decimal_sep() {
            var p = new Intl.NumberFormat().formatToParts(1111.1);
            var d = p.find(function(x) { return x.type === 'decimal'; });
            return d ? d.value : '.';
        }
        export function locale_group_sep() {
            var p = new Intl.NumberFormat().formatToParts(1111111.1);
            var g = p.find(function(x) { return x.type === 'group'; });
            return g ? g.value : '';
        }
    ")]
    extern "C" {
        pub fn locale_decimal_sep() -> String;
        pub fn locale_group_sep() -> String;
    }
}

/// Parses a locale-aware decimal string into an [`Amount`] using integer arithmetic.
///
/// Detects the user's locale decimal and grouping separators at runtime via
/// `Intl.NumberFormat` (WASM target) or falls back to `'.'` / `''` (native
/// tests).  Strips grouping separators, replaces the locale decimal separator
/// with `'.'`, then splits on `'.'` and computes the minor units exactly —
/// no `f64` rounding.
///
/// # Arguments
///
/// * `input` - User-entered decimal string (e.g. `"-84.20"`, `"1.000,50"`, `"CHF 1'000,00"`).
/// * `currency_code` - ISO 4217 code for the resulting [`Amount`].
/// * `scale` - Number of decimal places (e.g. `2` for AUD cents).
fn parse_amount(input: &str, currency_code: &str, scale: u8) -> Option<Amount> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    #[cfg(target_arch = "wasm32")]
    let (decimal_sep, group_sep) = (
        locale_js::locale_decimal_sep(),
        locale_js::locale_group_sep(),
    );
    #[cfg(not(target_arch = "wasm32"))]
    let (decimal_sep, group_sep) = (".".to_owned(), String::new());

    let normalised = if group_sep.is_empty() {
        trimmed.replace(decimal_sep.as_str(), ".")
    } else {
        trimmed
            .replace(group_sep.as_str(), "")
            .replace(decimal_sep.as_str(), ".")
    };

    let negative = normalised.starts_with('-');
    let digits = normalised.trim_start_matches('-');

    let (int_str, frac_str) = match digits.split_once('.') {
        Some((i, f)) => (i, f),
        None => (digits, ""),
    };

    let int_val: i64 = if int_str.is_empty() {
        0
    } else {
        int_str.parse().ok()?
    };

    let scale_usize = usize::from(scale);
    let scale_pow = 10_i64.pow(u32::from(scale));

    let frac_val: i64 = if scale_usize == 0 || frac_str.is_empty() {
        0
    } else {
        let padded = format!("{frac_str:0<scale_usize$}");
        padded.get(..scale_usize)?.parse().ok()?
    };

    let minor_abs = int_val.checked_mul(scale_pow)?.checked_add(frac_val)?;
    let minor = if negative {
        minor_abs.checked_neg()?
    } else {
        minor_abs
    };

    Some(Amount::new(minor, currency_code, scale))
}

/// Inline form for creating a new double-entry transaction from the current account.
///
/// The form produces a [`NewTransaction`] with one posting for the primary
/// account and one or more offset postings.  Additional offset postings can be
/// added via the "+ posting" button, supporting split transactions.  The backend
/// enforces the sum-to-zero double-entry invariant.
///
/// # Arguments
///
/// * `accounts` - Full account list for the offset-account dropdowns.
/// * `current_account_id` - The account whose register is currently open (first posting).
/// * `currency_code` - ISO 4217 code inferred from the current account's balance.
/// * `scale` - Decimal scale of the current account's currency (e.g. `2` for AUD).
/// * `on_submit` - Called with the completed [`NewTransaction`] when the user submits.
/// * `on_cancel` - Called when the user cancels or closes the form.
/// * `submit_error` - Reactive signal carrying the current IPC error string, if any.
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
    /// All accounts available for the offset posting dropdowns.
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
    /// Reactive signal carrying the current IPC error string, if any.
    submit_error: Signal<Option<String>>,
) -> impl IntoView {
    // Default date to today via js_sys::Date (wasm32-unknown-unknown only).
    let default_date: String = js_sys::Date::new_0()
        .to_iso_string()
        .as_string()
        .unwrap_or_default()
        .chars()
        .take(10)
        .collect();

    // Primary account display name shown in the first posting row.
    let primary_account_name = accounts
        .iter()
        .find(|a| a.id == current_account_id)
        .map_or_else(|| current_account_id.clone(), |a| a.name.clone());

    // Offset account options (all accounts except the primary).
    let offset_options: Vec<(String, String)> = accounts
        .iter()
        .filter(|a| a.id != current_account_id)
        .map(|a| (a.id.clone(), a.name.clone()))
        .collect();

    let default_offset_id = offset_options
        .first()
        .map(|(id, _)| id.clone())
        .unwrap_or_default();

    let date_input = RwSignal::new(default_date);
    let payee_input = RwSignal::new(String::new());
    let status_input = RwSignal::new(TxStatus::Pending);
    let errors: RwSignal<Vec<&'static str>> = RwSignal::new(vec![]);

    // Primary posting amount (current account).
    let primary_amount = RwSignal::new(String::new());

    // Extra postings: at least one, extendable via "+ posting".
    // Each element: (account_id signal, amount signal).
    let extra_postings: RwSignal<Vec<(RwSignal<String>, RwSignal<String>)>> =
        RwSignal::new(vec![(
            RwSignal::new(default_offset_id.clone()),
            RwSignal::new(String::new()),
        )]);

    let add_posting = {
        let dflt = default_offset_id.clone();
        move |_: leptos::ev::MouseEvent| {
            extra_postings.update(|ps| {
                ps.push((
                    RwSignal::new(dflt.clone()),
                    RwSignal::new(String::new()),
                ));
            });
        }
    };

    let currency_code_submit = currency_code.clone();
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

        let primary_amt_opt =
            parse_amount(&primary_amount.get(), &currency_code_submit, scale);
        if primary_amt_opt.is_none() {
            errs.push("primary amount must be a valid number");
        }

        let snapshot = extra_postings.get();
        let mut any_missing_account = false;
        let mut any_bad_amount = false;
        let parsed_extras: Vec<(String, Option<Amount>)> = snapshot
            .iter()
            .map(|(acc_id, amt)| {
                let id = acc_id.get();
                if id.is_empty() {
                    any_missing_account = true;
                }
                let parsed = parse_amount(&amt.get(), &currency_code_submit, scale);
                if parsed.is_none() {
                    any_bad_amount = true;
                }
                (id, parsed)
            })
            .collect();
        if any_missing_account {
            errs.push("all offset accounts must be selected");
        }
        if any_bad_amount {
            errs.push("all amounts must be valid numbers");
        }

        if !errs.is_empty() {
            errors.set(errs);
            return;
        }

        let Some(primary_amt) = primary_amt_opt else {
            return;
        };

        let mut postings = Vec::with_capacity(parsed_extras.len().saturating_add(1));
        postings.push(NewPosting::new(
            current_account_id_submit.clone(),
            primary_amt,
            None::<&str>,
        ));
        for (acc_id, amt_opt) in parsed_extras {
            let Some(amt) = amt_opt else {
                return;
            };
            postings.push(NewPosting::new(acc_id, amt, None::<&str>));
        }

        let tx = NewTransaction::new(date, payee, status_input.get(), vec![], postings);
        on_submit.run(tx);
    };

    let on_cancel_header = move |_: leptos::ev::MouseEvent| {
        on_cancel.run(());
    };
    let on_cancel_footer = move |_: leptos::ev::MouseEvent| {
        on_cancel.run(());
    };

    // Snapshot for use in the reactive posting-rows closure.
    let offset_options_view = offset_options.clone();

    view! {
        <form class=style::form data-testid="add-transaction-form" on:submit=on_form_submit>
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
                    type="date"
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

            </div>

            <div class=style::postings_section>
                <div class=style::postings_header>
                    {format!("postings ({currency_code})")}
                </div>

                <div class=style::posting_row_primary>
                    <span class=style::posting_account_label>
                        {primary_account_name}
                    </span>
                    <input
                        id="atf-primary-amount"
                        class=style::input
                        type="text"
                        placeholder="e.g. -84.20"
                        prop:value=move || primary_amount.get()
                        on:input=move |e| {
                            primary_amount.set(event_target_value(&e));
                        }
                    />
                </div>

                {move || {
                    let opts = offset_options_view.clone();
                    let can_remove = extra_postings.get().len() > 1;
                    extra_postings
                        .get()
                        .into_iter()
                        .enumerate()
                        .map(|(i, (acc_id, amt))| {
                            let opts2 = opts.clone();
                            let remove = move |_: leptos::ev::MouseEvent| {
                                extra_postings
                                    .update(|ps| {
                                        if ps.len() > 1 {
                                            ps.remove(i);
                                        }
                                    });
                            };
                            view! {
                                <div class=style::posting_row>
                                    <select
                                        class=style::input
                                        data-testid=format!("atf-offset-account-{i}")
                                        on:change=move |e| {
                                            acc_id.set(event_target_value(&e));
                                        }
                                    >
                                        {opts2
                                            .into_iter()
                                            .map(|(id, name)| {
                                                let id_clone = id.clone();
                                                view! {
                                                    <option
                                                        value=id
                                                        selected=move || acc_id.get() == id_clone
                                                    >
                                                        {name}
                                                    </option>
                                                }
                                            })
                                            .collect::<Vec<_>>()}
                                    </select>
                                    <input
                                        class=style::input
                                        type="text"
                                        placeholder="e.g. 84.20"
                                        data-testid=format!("atf-offset-amount-{i}")
                                        prop:value=move || amt.get()
                                        on:input=move |e| {
                                            amt.set(event_target_value(&e));
                                        }
                                    />
                                    <button
                                        type="button"
                                        class=style::remove_posting_btn
                                        on:click=remove
                                        aria-label="remove posting"
                                        prop:disabled=!can_remove
                                    >
                                        "×"
                                    </button>
                                </div>
                            }
                        })
                        .collect::<Vec<_>>()
                }}

                <button
                    type="button"
                    class=style::add_posting_btn
                    on:click=add_posting
                >
                    "+ posting"
                </button>
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

            {move || {
                submit_error
                    .get()
                    .map(|err| view! { <div class=style::ipc_error>{err}</div> })
            }}

            <div class=style::form_footer>
                <button type="button" class=style::cancel_btn on:click=on_cancel_footer>
                    "cancel"
                </button>
                <button type="submit" class=style::submit_btn data-testid="add-transaction-submit">
                    "add transaction"
                </button>
            </div>
        </form>
    }
}

#[cfg(debug_assertions)]
pub mod qa;

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::parse_amount;

    #[test]
    fn parse_amount_positive_decimal() {
        let amt = parse_amount("84.20", "AUD", 2).expect("parses positive decimal");
        assert_eq!(amt.minor_units, 8_420);
        assert_eq!(amt.currency_code, "AUD");
        assert_eq!(amt.scale, 2);
    }

    #[test]
    fn parse_amount_negative_decimal() {
        let amt = parse_amount("-84.20", "AUD", 2).expect("parses negative decimal");
        assert_eq!(amt.minor_units, -8_420);
    }

    #[test]
    fn parse_amount_zero_is_allowed() {
        let amt = parse_amount("0", "AUD", 2).expect("parses zero");
        assert_eq!(amt.minor_units, 0);
    }

    #[test]
    fn parse_amount_empty_returns_none() {
        assert!(parse_amount("", "AUD", 2).is_none());
    }

    #[test]
    fn parse_amount_whitespace_only_returns_none() {
        assert!(parse_amount("   ", "AUD", 2).is_none());
    }

    #[test]
    fn parse_amount_non_numeric_returns_none() {
        assert!(parse_amount("abc", "AUD", 2).is_none());
    }

    #[test]
    fn parse_amount_scale_conversion() {
        // scale=3 means minor units are thousandths
        let amt = parse_amount("1.234", "USD", 3).expect("parses scale=3");
        assert_eq!(amt.minor_units, 1_234);
    }

    #[test]
    fn parse_amount_trims_whitespace() {
        let amt = parse_amount("  10.00  ", "AUD", 2).expect("trims whitespace");
        assert_eq!(amt.minor_units, 1_000);
    }

    #[test]
    fn parse_amount_integer_only() {
        let amt = parse_amount("42", "AUD", 2).expect("parses integer");
        assert_eq!(amt.minor_units, 4_200);
    }

    #[test]
    fn parse_amount_short_fraction_pads() {
        // "84.2" with scale=2 should yield 8420 (pad frac to "20")
        let amt = parse_amount("84.2", "AUD", 2).expect("pads short fraction");
        assert_eq!(amt.minor_units, 8_420);
    }

    #[test]
    fn parse_amount_scale_zero() {
        let amt = parse_amount("100", "JPY", 0).expect("parses scale=0");
        assert_eq!(amt.minor_units, 100);
    }
}
