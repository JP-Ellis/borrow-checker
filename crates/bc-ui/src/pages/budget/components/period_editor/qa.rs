//! QA showcase for [`PeriodEditor`].

use bc_ipc::Period;
use jiff::civil::Date;
use leptos::prelude::*;

use super::DraftEditor;
use super::PeriodEditor;
use super::draft::PeriodDraft;
use super::draft::PeriodKind;
use crate::period_defaults::PeriodDefaults;

/// Renders the editor once per kind, plus the missing-anchor, invalid-Custom
/// and locked states. Each instance shows the value it last reported.
#[component]
pub fn PeriodEditorQa() -> impl IntoView {
    /* Outside the shell; supply an anchor so switching to fortnightly seeds one. */
    provide_context(PeriodDefaults {
        fortnightly_anchor: Some(Date::constant(2026, 1, 5)),
        ..PeriodDefaults::FALLBACK
    });

    let kinds = kind_samples();

    /* An all-empty Custom never comes from the server; build it directly. */
    let invalid_custom = Period::Custom {
        days: None,
        weeks: None,
        months: None,
    };
    let missing_anchor =
        PeriodDraft::from_period(&Period::Monthly, PeriodDefaults::FALLBACK).map(|d| {
            d.switch_to(
                PeriodKind::Fortnightly,
                &Period::Monthly,
                PeriodDefaults::FALLBACK,
            )
        });

    let rows = kinds
        .into_iter()
        .map(|(title, period)| {
            instance(title, move |report| {
                view! { <PeriodEditor initial=period disabled=false on_change=report /> }.into_any()
            })
        })
        .collect_view();

    view! {
        <div style="display:flex;flex-direction:column;gap:24px;max-width:480px;padding:24px">
            {rows}
            {instance(
                "fortnightly, no anchor configured",
                move |report| {
                    missing_anchor
                        .map(|start| {
                            report.run(start.to_period());
                            view! {
                                <DraftEditor
                                    start=start
                                    original=Period::Monthly
                                    defaults=PeriodDefaults::FALLBACK
                                    disabled=false
                                    on_change=report
                                />
                            }
                        })
                        .into_any()
                },
            )}
            {instance(
                "custom with no duration (the server cannot send this; only the draft error shows)",
                move |report| {
                    view! {
                        <PeriodEditor initial=invalid_custom disabled=false on_change=report />
                    }
                        .into_any()
                },
            )}
            {instance(
                "locked (financial year)",
                |report| {
                    let fy = Period::FinancialYear {
                        start_month: 7,
                        start_day: 1,
                    };
                    view! { <PeriodEditor initial=fy disabled=true on_change=report /> }.into_any()
                },
            )}
        </div>
    }
}

/// One titled sample period per kind.
fn kind_samples() -> [(&'static str, Period); 9] {
    [
        ("daily", Period::Daily),
        ("weekly", Period::Weekly),
        (
            "fortnightly",
            Period::Fortnightly {
                anchor: Date::constant(2026, 3, 2),
            },
        ),
        ("monthly", Period::Monthly),
        ("quarterly", Period::Quarterly),
        ("calendar year", Period::CalendarYear),
        (
            "financial year (1 January)",
            Period::FinancialYear {
                start_month: 1,
                start_day: 1,
            },
        ),
        (
            "financial quarter",
            Period::FinancialQuarter {
                start_month: 7,
                start_day: 1,
            },
        ),
        (
            "custom (every 10 days)",
            Period::Custom {
                days: Some(10),
                weeks: None,
                months: None,
            },
        ),
    ]
}

/// One titled editor with the value it last reported beneath it.
///
/// # Arguments
///
/// * `title` - Caption naming the state shown.
/// * `build` - Builds the editor, given the callback that records its reports.
fn instance(
    title: &'static str,
    build: impl FnOnce(Callback<Result<Period, String>>) -> AnyView,
) -> impl IntoView {
    let reported: RwSignal<Option<Result<Period, String>>> = RwSignal::new(None);
    let report = Callback::new(move |value| reported.set(Some(value)));
    view! {
        <section>
            <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">{title}</p>
            {build(report)}
            <p style="font-size:11px;color:var(--bc-ink-soft);margin-top:8px;">
                {move || match reported.get() {
                    Some(Ok(period)) => format!("reports {}", period.label()),
                    Some(Err(message)) => format!("reports error: {message}"),
                    None => "reports nothing".to_owned(),
                }}
            </p>
        </section>
    }
}
