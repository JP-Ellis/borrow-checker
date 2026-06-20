//! Add/amend form for a single budget revision, with an exact/snap date toggle.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BudgetRevisionView;
use bc_ipc::Period;
use bc_ipc::RolloverPolicy;
use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "form.module.scss");

/// Parses a target input string into minor units at scale 2.
///
/// Returns `None` for empty input, bare `.`, negative values, or non-numeric
/// input. Supports both integer (`"250"`) and decimal (`"2.50"`) forms.
/// Multiple dots → `None`. At least one digit must be present.
///
/// Fractional digits beyond 2 are rounded half-up (round half away from zero)
/// at the 2nd decimal place, so `"1.999"` → `200` and `"1.994"` → `199`.
/// Overflow (result too large for `i64`) returns `None`.
#[must_use]
fn parse_target_minor(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() || s.starts_with('-') {
        return None;
    }

    // Reject multiple dots.
    let dot_count = s.chars().filter(|&c| c == '.').count();
    if dot_count > 1 {
        return None;
    }

    match s.split_once('.') {
        None => {
            // Integer form: all chars must be digits.
            if !s.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let whole: i64 = s.parse().ok()?;
            whole.checked_mul(100)
        }
        Some((whole, frac)) => {
            // Both sides must be all-digit (each may be empty, but together
            // at least one digit must exist).
            if !whole.chars().all(|c| c.is_ascii_digit())
                || !frac.chars().all(|c| c.is_ascii_digit())
            {
                return None;
            }
            // Require at least one digit somewhere.
            if whole.is_empty() && frac.is_empty() {
                return None;
            }

            let whole: i64 = if whole.is_empty() {
                0
            } else {
                whole.parse().ok()?
            };

            // Build a fixed-point minor value with rounding.
            // We need the first 2 frac digits plus whether any digit >= 5
            // exists at position 3 (the third decimal place).
            #[expect(
                clippy::string_slice,
                reason = "frac is validated as all-ASCII-digit by the guard above; byte-index slicing is safe"
            )]
            #[expect(
                clippy::indexing_slicing,
                reason = "frac.len() >= 3 is guaranteed by the match arm above"
            )]
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "d, prefix are small digit values; saturating ops not needed for this fixed-scale decimal"
            )]
            let cents: i64 = match frac.len() {
                0 => 0,
                1 => {
                    /* e.g. "5" → 50 */
                    let d: i64 = frac.parse().ok()?;
                    d * 10
                }
                2 => frac.parse().ok()?,
                _ => {
                    /* Round the 2-digit prefix based on the third digit. */
                    let prefix: i64 = frac[..2].parse().ok()?;
                    let third: u8 = frac.as_bytes()[2] - b'0';
                    if third >= 5 { prefix + 1 } else { prefix }
                }
            };

            whole.checked_mul(100)?.checked_add(cents)
        }
    }
}

/// Period choices offered in the dropdown.
const PERIOD_CHOICES: [(&str, Period); 5] = [
    ("weekly", Period::Weekly),
    ("fortnightly", Period::Fortnightly),
    ("monthly", Period::Monthly),
    ("quarterly", Period::Quarterly),
    ("calendar_year", Period::CalendarYear),
];

/// Maps a period dropdown key to its [`Period`].
#[must_use]
fn period_from_key(key: &str) -> Period {
    PERIOD_CHOICES
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(Period::Monthly, |(_, p)| p.clone())
}

/// Maps a [`Period`] to its dropdown key.
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "Period is #[non_exhaustive]; the wildcard catches FinancialQuarter and any future variants, mapping them to the 'monthly' fallback"
)]
fn period_key(period: &Period) -> &'static str {
    match period {
        Period::Weekly => "weekly",
        Period::Fortnightly => "fortnightly",
        Period::Quarterly => "quarterly",
        Period::CalendarYear | Period::FinancialYear { .. } => "calendar_year",
        _ => "monthly",
    }
}

/// Maps a [`RolloverPolicy`] to its dropdown key.
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "RolloverPolicy is #[non_exhaustive]; the wildcard catches any future variants as reset_to_zero"
)]
fn rollover_key(policy: RolloverPolicy) -> &'static str {
    match policy {
        RolloverPolicy::CarryForward => "carry_forward",
        RolloverPolicy::CapAtTarget => "cap_at_target",
        _ => "reset_to_zero",
    }
}

/// Add or amend a budget revision.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos component props must be owned values"
)]
#[expect(
    clippy::too_many_lines,
    reason = "large view! block combining all revision fields"
)]
pub fn RevisionForm(
    /// Budget this revision belongs to.
    budget_id: String,
    /// Existing revision to amend, or `None` to add a new one.
    #[prop(optional)]
    revision: Option<BudgetRevisionView>,
    /// Whether the Snap option is offered (false for the first revision).
    allow_snap: bool,
    /// Invoked after a successful save.
    on_saved: Callback<()>,
    /// Invoked when the user cancels.
    on_cancel: Callback<()>,
) -> impl IntoView {
    let revision_id = revision.as_ref().map(|r| r.id.clone());
    let is_amend = revision_id.is_some();

    let init_eff = revision
        .as_ref()
        .map_or_else(|| jiff::Zoned::now().date(), |r| r.effective_from);
    let init_name = revision
        .as_ref()
        .and_then(|r| r.name.clone())
        .unwrap_or_default();
    let init_target = revision
        .as_ref()
        .and_then(|r| r.target.as_ref())
        .map_or_else(String::new, |a| {
            crate::components::num::to_decimal_string(a.minor_units.unsigned_abs(), a.scale)
        });
    let init_currency = revision
        .as_ref()
        .and_then(|r| r.target.as_ref())
        .map_or_else(|| "AUD".to_owned(), |a| a.currency_code.clone());
    let init_period = revision
        .as_ref()
        .map_or("monthly", |r| period_key(&r.period));
    let init_rollover = revision
        .as_ref()
        .map_or(RolloverPolicy::ResetToZero, |r| r.rollover);
    let init_tag = revision
        .as_ref()
        .and_then(|r| r.tag_filter.clone())
        .unwrap_or_default();

    let eff_input = RwSignal::new(init_eff.to_string());
    let snap = RwSignal::new(false);
    let name_input = RwSignal::new(init_name);
    let target_input = RwSignal::new(init_target);
    let currency_input = RwSignal::new(init_currency);
    let period_input = RwSignal::new(init_period.to_owned());
    let rollover_input = RwSignal::new(init_rollover);
    let tag_input = RwSignal::new(init_tag);
    let resolved_hint: RwSignal<Option<String>> = RwSignal::new(None);
    let saving = RwSignal::new(false);
    let error: RwSignal<Option<String>> = RwSignal::new(None);

    let budget_id_hint = StoredValue::new(budget_id.clone());
    let rev_id_hint = StoredValue::new(revision_id.clone());

    // Recompute the resolved-date hint when snap is on and the date parses.
    Effect::new(move |_| {
        if !snap.get() {
            resolved_hint.set(None);
            return;
        }
        let Ok(d) = eff_input.get().parse::<jiff::civil::Date>() else {
            resolved_hint.set(Some("enter a valid date (YYYY-MM-DD)".to_owned()));
            return;
        };
        let bid = budget_id_hint.get_value();
        let exclude = rev_id_hint.get_value();
        leptos::task::spawn_local(async move {
            match bc_ipc::client::resolve_effective_date(&bid, d, exclude.as_deref()).await {
                Ok(resolved) => resolved_hint.set(Some(format!("stores {resolved}"))),
                Err(e) => resolved_hint.set(Some(format!("snap error: {e}"))),
            }
        });
    });

    let budget_id_save = StoredValue::new(budget_id);
    let rev_id_save = StoredValue::new(revision_id);
    let save = move |_| {
        let Ok(typed_date) = eff_input.get_untracked().parse::<jiff::civil::Date>() else {
            error.set(Some("Effective date must be YYYY-MM-DD".to_owned()));
            return;
        };
        let name = name_input.get_untracked();
        let name_opt = (!name.trim().is_empty()).then_some(name);
        let target_minor = parse_target_minor(&target_input.get_untracked());
        let currency = currency_input.get_untracked();
        let target_currency = target_minor.is_some().then_some(currency);
        let rollover = rollover_input.get_untracked();
        let period = period_from_key(&period_input.get_untracked());
        let tag = tag_input.get_untracked();
        let tag_opt = (!tag.trim().is_empty()).then_some(tag);
        let use_snap = snap.get_untracked();

        // Client-side mirror of the CapAtTarget invariant.
        if rollover == RolloverPolicy::CapAtTarget && target_minor.is_none() {
            error.set(Some("Cap at target requires a target amount".to_owned()));
            return;
        }

        let bid = budget_id_save.get_value();
        let rid = rev_id_save.get_value();
        saving.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            let effective_from = if use_snap {
                match bc_ipc::client::resolve_effective_date(&bid, typed_date, rid.as_deref()).await
                {
                    Ok(d) => d,
                    Err(e) => {
                        saving.set(false);
                        error.set(Some(e.to_string()));
                        return;
                    }
                }
            } else {
                typed_date
            };
            let result = bc_ipc::client::revise_budget(
                &bid,
                rid.as_deref(),
                effective_from,
                name_opt.as_deref(),
                target_minor,
                target_currency.as_deref(),
                rollover,
                period,
                tag_opt.as_deref(),
            )
            .await;
            saving.set(false);
            match result {
                Ok(()) => on_saved.run(()),
                Err(e) => error.set(Some(e.to_string())),
            }
        });
    };

    let title = if is_amend {
        "Amend revision"
    } else {
        "Add revision"
    };

    view! {
        <div class=style::form aria-label="revision form">
            <div class=style::title>{title}</div>

            <div class=style::row>
                <span class=style::label>"Effective"</span>
                <input
                    type="date"
                    class=style::input
                    prop:value=move || eff_input.get()
                    on:input=move |ev| eff_input.set(event_target_value(&ev))
                />
            </div>

            <Show when=move || allow_snap>
                <div class=style::row>
                    <span class=style::label>"Date mode"</span>
                    <div class=style::seg>
                        <button
                            class=move || {
                                if snap.get() { style::seg_btn } else { style::seg_btn_on }
                            }
                            on:click=move |_| snap.set(false)
                        >
                            "Exact"
                        </button>
                        <button
                            class=move || {
                                if snap.get() { style::seg_btn_on } else { style::seg_btn }
                            }
                            on:click=move |_| snap.set(true)
                        >
                            "Snap to boundary"
                        </button>
                    </div>
                </div>
                {move || {
                    resolved_hint.get().map(|h| view! { <div class=style::hint>{h}</div> })
                }}
            </Show>

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
                    type="number"
                    step="0.01"
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

            <div class=style::row>
                <span class=style::label>"Period"</span>
                <select
                    class=style::input
                    on:change=move |ev| period_input.set(event_target_value(&ev))
                    prop:value=move || period_input.get()
                >
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
                        let v = event_target_value(&ev);
                        rollover_input
                            .set(
                                match v.as_str() {
                                    "carry_forward" => RolloverPolicy::CarryForward,
                                    "cap_at_target" => RolloverPolicy::CapAtTarget,
                                    _ => RolloverPolicy::ResetToZero,
                                },
                            );
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
                    {move || if saving.get() { "Saving\u{2026}" } else { "Save" }}
                </button>
                <button class=style::cancel on:click=move |_| on_cancel.run(())>
                    "Cancel"
                </button>
            </div>
        </div>
    }
}
