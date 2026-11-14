//! Budget period editor: a kind dropdown plus the parameter fields of the
//! selected kind.

#[cfg(debug_assertions)]
pub(crate) mod qa;

/// Draft state and validation, Leptos-free for host tests.
pub(crate) mod draft;

use bc_ipc::Period;
use leptos::prelude::*;
use stylance::import_style;

use self::draft::PeriodDraft;
use self::draft::PeriodKind;
use crate::period_defaults::PeriodDefaults;
use crate::period_defaults::use_period_defaults;

import_style!(style, "period_editor.module.scss");

/// Edits a budget period.
///
/// Reports `Ok(initial)` once on mount, so a form the user never touches
/// submits the period it loaded. A variant this build does not know renders
/// as a disabled "unsupported" option and keeps reporting `Ok(initial)`.
#[component]
pub fn PeriodEditor(
    /// The period the editor opens with.
    initial: Period,
    /// Whether every control is disabled.
    #[prop(into)]
    disabled: Signal<bool>,
    /// Receives the edited period, or the message explaining why it is invalid.
    on_change: Callback<Result<Period, String>>,
) -> impl IntoView {
    let defaults = use_period_defaults();
    on_change.run(Ok(initial.clone()));
    match PeriodDraft::from_period(&initial, defaults) {
        Some(start) => view! {
            <DraftEditor
                start=start
                original=initial
                defaults=defaults
                disabled=disabled
                on_change=on_change
            />
        }
        .into_any(),
        None => view! {
            <div class=style::editor>
                <select class=style::input aria-label="period kind" disabled=true>
                    <option>"unsupported"</option>
                </select>
            </div>
        }
        .into_any(),
    }
}

/// The editor body, starting from an arbitrary draft.
///
/// [`PeriodEditor`] starts it from the loaded period; the QA page also starts
/// it from drafts no `Period` describes, such as Fortnightly without an anchor.
#[component]
pub fn DraftEditor(
    /// The draft the fields open with.
    start: PeriodDraft,
    /// The period the editor opened with; seeds a kind the user flips back to.
    original: Period,
    /// The configured FY start and fortnightly anchor.
    defaults: PeriodDefaults,
    /// Whether every control is disabled.
    #[prop(into)]
    disabled: Signal<bool>,
    /// Receives the edited period, or the message explaining why it is invalid.
    on_change: Callback<Result<Period, String>>,
) -> impl IntoView {
    let draft = RwSignal::new(start);
    let original = StoredValue::new(original);
    let kind = Memo::new(move |_| draft.with(|d| d.kind));
    let message = Memo::new(move |_| draft.with(|d| d.to_period().err()));

    let fields = move || kind_fields(kind.get(), draft, disabled, on_change);

    view! {
        <div class=style::editor>
            <select
                class=style::input
                aria-label="period kind"
                disabled=disabled
                on:change=move |ev| {
                    if let Some(next) = PeriodKind::from_key(&event_target_value(&ev)) {
                        commit(
                            draft,
                            on_change,
                            |d| *d = original.with_value(|o| d.switch_to(next, o, defaults)),
                        );
                    }
                }
                prop:value=move || kind.get().key()
            >
                {PeriodKind::ALL
                    .into_iter()
                    .map(|k| {
                        view! {
                            <option value=k.key() selected=move || kind.get() == k>
                                {k.text()}
                            </option>
                        }
                    })
                    .collect_view()}
            </select>
            {fields}
            {move || {
                message
                    .get()
                    .map(|m| {
                        view! {
                            <p class=style::err role="alert">
                                {m}
                            </p>
                        }
                    })
            }}
        </div>
    }
}

/// The parameter fields of `kind`; `None` for a kind that takes none.
fn kind_fields(
    kind: PeriodKind,
    draft: RwSignal<PeriodDraft>,
    disabled: Signal<bool>,
    on_change: Callback<Result<Period, String>>,
) -> Option<AnyView> {
    let text_field = move |label: &'static str,
                           input_type: &'static str,
                           read: fn(&PeriodDraft) -> String,
                           write: fn(&mut PeriodDraft, String)| {
        view! {
            <Field
                label=label
                input_type=input_type
                value=Signal::derive(move || draft.with(read))
                disabled=disabled
                on_input=Callback::new(move |value: String| {
                    commit(draft, on_change, |d| write(d, value));
                })
            />
        }
    };

    match kind {
        PeriodKind::Daily
        | PeriodKind::Weekly
        | PeriodKind::Monthly
        | PeriodKind::Quarterly
        | PeriodKind::CalendarYear => None,
        PeriodKind::Fortnightly => Some(
            view! {
                <div class=style::fields>
                    {text_field("anchor", "date", |d| d.anchor.clone(), |d, v| d.anchor = v)}
                </div>
            }
            .into_any(),
        ),
        PeriodKind::FinancialYear | PeriodKind::FinancialQuarter => Some(
            view! {
                <div class=style::fields>
                    {text_field(
                        "start month",
                        "text",
                        |d| d.start_month.clone(),
                        |d, v| d.start_month = v,
                    )}
                    {text_field(
                        "start day",
                        "text",
                        |d| d.start_day.clone(),
                        |d, v| d.start_day = v,
                    )}
                </div>
            }
            .into_any(),
        ),
        PeriodKind::Custom => Some(
            view! {
                <div class=style::fields>
                    {text_field("months", "text", |d| d.months.clone(), |d, v| d.months = v)}
                    {text_field("weeks", "text", |d| d.weeks.clone(), |d, v| d.weeks = v)}
                    {text_field("days", "text", |d| d.days.clone(), |d, v| d.days = v)}
                </div>
            }
            .into_any(),
        ),
    }
}

/// One labelled parameter field.
#[component]
fn Field(
    /// Caption above the input, also its accessible name.
    label: &'static str,
    /// The input's `type` attribute.
    input_type: &'static str,
    /// The field's text.
    value: Signal<String>,
    /// Whether the input is disabled.
    disabled: Signal<bool>,
    /// Receives the text after each keystroke.
    on_input: Callback<String>,
) -> impl IntoView {
    view! {
        <label class=style::field>
            <span class=style::field_label>{label}</span>
            <input
                type=input_type
                inputmode=(input_type == "text").then_some("numeric")
                class=style::input
                disabled=disabled
                prop:value=move || value.get()
                on:input=move |ev| on_input.run(event_target_value(&ev))
            />
        </label>
    }
}

/// Applies `change` to the draft and reports the resulting period.
fn commit(
    draft: RwSignal<PeriodDraft>,
    on_change: Callback<Result<Period, String>>,
    change: impl FnOnce(&mut PeriodDraft),
) {
    draft.update(change);
    on_change.run(draft.with_untracked(PeriodDraft::to_period));
}
