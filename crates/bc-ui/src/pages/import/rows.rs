//! The rows table: every parsed row with its fate, filtered and paged.

use bc_ipc::PreviewLeg;
use bc_ipc::PreviewRow;
use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::components::num::Num;
use crate::components::status_pill::StatusPill;
use crate::components::toast::ToastKind;
use crate::components::toast::use_toasts;
use crate::pages::import::model;
use crate::pages::import::model::FateFilter;
use crate::pages::import::model::FateKind;
use crate::pages::import::model::OwnerTarget;
use crate::pages::import::style;

/// Fate chips with counts, then the rows they let through, [`model::PAGE`]
/// at a time.
#[component]
pub fn RowsTable(
    /// The preview's rows, in source order.
    rows: Vec<PreviewRow>,
) -> impl IntoView {
    let rows = StoredValue::new(rows);
    let filter = RwSignal::new(FateFilter::default());
    let limit = RwSignal::new(model::PAGE);
    let counts = rows.with_value(|r| model::fate_counts(r));
    let visible = Memo::new(move |_| {
        let f = filter.get();
        let l = limit.get();
        rows.with_value(|r| model::visible_rows(r, f, l))
    });
    view! {
        <div class=style::chips role="group" aria-label="filter rows by fate">
            {counts
                .into_iter()
                .map(|(kind, n)| {
                    let on = move || filter.with(|f| f.contains(kind));
                    view! {
                        <button
                            type="button"
                            class=move || {
                                if on() {
                                    format!("{} {}", style::chip, style::chip_on)
                                } else {
                                    style::chip.to_owned()
                                }
                            }
                            aria-pressed=move || on().to_string()
                            data-testid=format!("import-fate-{}", kind.slug())
                            on:click=move |_| {
                                filter.update(|f| f.toggle(kind));
                                limit.set(model::PAGE);
                            }
                        >
                            {format!("{} {n}", kind.label())}
                        </button>
                    }
                })
                .collect::<Vec<_>>()}
        </div>
        <table class=style::rows_table aria-label="parsed rows">
            <thead>
                <tr>
                    <th class=style::th>"Date"</th>
                    <th class=style::th>"Description"</th>
                    <th class=style::th>"Legs"</th>
                    <th class=style::th>"Fate"</th>
                    <th class=style::th>"Location"</th>
                </tr>
            </thead>
            <tbody>
                {move || {
                    visible
                        .with(|v| v.indices.clone())
                        .into_iter()
                        .filter_map(|i| rows.with_value(|r| r.get(i).cloned()))
                        .map(row_view)
                        .collect::<Vec<_>>()
                }}
            </tbody>
        </table>
        {move || {
            let remaining = visible.with(|v| v.remaining);
            (remaining > 0)
                .then(|| {
                    view! {
                        <button
                            type="button"
                            class=style::more
                            data-testid="import-rows-more"
                            on:click=move |_| limit.update(|l| *l = l.saturating_add(model::PAGE))
                        >
                            {format!("Show more ({remaining} hidden)")}
                        </button>
                    }
                })
        }}
    }
}

/// One row, plus a muted row of its diagnostics when it has any.
///
/// # Arguments
///
/// * `row` - The parsed row.
fn row_view(row: PreviewRow) -> impl IntoView {
    let kind = FateKind::of(&row.fate);
    let (label, tone) = model::fate_pill(&row.fate);
    let owner = model::fate_owner(&row.fate);
    let diagnostics = row.diagnostics;
    let diagnostic_row = (!diagnostics.is_empty()).then(|| {
        view! {
            <tr class=style::diag_row>
                <td colspan="5">
                    <ul class=style::diags>
                        {diagnostics
                            .iter()
                            .map(|d| view! { <li>{model::row_diagnostic_text(d)}</li> })
                            .collect::<Vec<_>>()}
                    </ul>
                </td>
            </tr>
        }
    });
    view! {
        <tr class=style::row data-fate=kind.slug()>
            <td class=style::td_date>{row.date}</td>
            <td class=style::td>{row.description}</td>
            <td class=style::td>
                <ul class=style::legs>{row.legs.into_iter().map(leg_view).collect::<Vec<_>>()}</ul>
            </td>
            <td class=style::td>
                <StatusPill label=label tone=tone />
                {owner.map(|o| view! { <OwnerLink owner=o /> })}
            </td>
            <td class=style::td_location>{row.location}</td>
        </tr>
        {diagnostic_row}
    }
}

/// One leg: the path as stated and its amount, struck with its cause when skipped.
///
/// # Arguments
///
/// * `leg` - The leg.
fn leg_view(leg: PreviewLeg) -> impl IntoView {
    let cause = model::leg_skip_cause(&leg.fate).map(str::to_owned);
    let class = if cause.is_some() {
        format!("{} {}", style::leg, style::leg_skipped)
    } else {
        style::leg.to_owned()
    };
    view! {
        <li class=class>
            <span class=style::leg_account>{leg.account}</span>
            {leg.amount.map(|a| view! { <Num money=a /> })}
            {cause.map(|c| view! { <span class=style::leg_cause>{format!("skipped: {c}")}</span> })}
        </li>
    }
}

/// Opens the owning transaction's first account register.
#[component]
fn OwnerLink(
    /// The owning transaction's ID.
    owner: String,
) -> impl IntoView {
    let navigate = use_navigate();
    let toasts = use_toasts();
    let failed = RwSignal::new(false);
    let title = format!("transaction {owner}");
    let open = move |_| {
        let id = owner.clone();
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            let reply = bc_ipc::client::get_transaction(&id)
                .await
                .map(|tx| tx.postings.first().map(|p| p.account.id.clone()));
            match model::owner_target(reply) {
                OwnerTarget::Register(href) => navigate(&href, NavigateOptions::default()),
                OwnerTarget::NotFound => failed.set(true),
                OwnerTarget::Failed(message) => {
                    toasts.push(ToastKind::Error, message, None);
                }
            }
        });
    };
    view! {
        <button type="button" class=style::owner title=title on:click=open>
            {move || if failed.get() { "owner not found" } else { "owner" }}
        </button>
    }
}
