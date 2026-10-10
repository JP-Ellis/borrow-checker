//! The History section: every import batch, newest first.

use bc_ipc::ImportBatchInfo;
use jiff::tz::TimeZone;
use leptos::html::Tbody;
use leptos::prelude::*;
use leptos_router::hooks::use_query_map;

use crate::components::error_banner::ErrorBanner;
use crate::components::status_pill::StatusPill;
use crate::pages::import::discard::COUNTS_CHANGED_TEXT;
use crate::pages::import::discard::DiscardPanel;
use crate::pages::import::discard_model;
use crate::pages::import::discard_model::Tense;
use crate::pages::import::history_model;
use crate::pages::import::links;
use crate::pages::import::refresh::SharedStores;
use crate::pages::import::style;

/// Every batch, newest first, refetched on each bump of `version`.
/// `?batch=<id>` highlights and scrolls to that batch.
#[component]
pub fn HistorySection(
    /// Bumped after a write.
    version: RwSignal<u32>,
    /// Stores to refresh after a discard.
    stores: SharedStores,
) -> impl IntoView {
    let query = use_query_map();
    let highlight = Memo::new(move |_| query.with(|q| q.get("batch")));
    let batches = LocalResource::new(move || {
        version.track();
        bc_ipc::client::list_import_batches()
    });
    let expanded = RwSignal::new(Option::<String>::None);
    let armed = RwSignal::new(Option::<String>::None);
    let last_discard = RwSignal::new(Option::<(String, bool)>::None);
    // `?batch=<id>&discard=1` arms that batch once it is listed and not discarded.
    let armed_from_query = StoredValue::new(Option::<String>::None);
    Effect::new(move |_| {
        let (target, arm) = query.with(|q| {
            (
                q.get("batch"),
                history_model::arm_requested(q.get("discard").as_deref()),
            )
        });
        let Some(target) = target.filter(|_| arm) else {
            return;
        };
        let Some(Ok(list)) = batches.get() else {
            return;
        };
        if armed_from_query.with_value(|a| a.as_deref() == Some(target.as_str())) {
            return;
        }
        if list
            .iter()
            .any(|b| b.id == target && !history_model::is_discarded(&b.state))
        {
            armed_from_query.set_value(Some(target.clone()));
            armed.set(Some(target));
        }
    });
    view! {
        <section class=style::section aria-labelledby="import-history-title">
            <h2 id="import-history-title" class=style::section_title>
                "History"
            </h2>
            {move || match batches.get() {
                None => {
                    view! {
                        <p role="status">
                            <span class=style::skeleton_bar style="width:20rem" />
                        </p>
                    }
                        .into_any()
                }
                Some(Err(e)) => {
                    view! { <ErrorBanner message=format!("Failed to load import history: {e}") /> }
                        .into_any()
                }
                Some(Ok(list)) if list.is_empty() => {
                    view! {
                        <p class=style::empty_state>
                            "No imports yet. Preview a profile above to start one."
                        </p>
                    }
                        .into_any()
                }
                Some(Ok(list)) => {
                    view! {
                        <table class=style::table aria-label="import history">
                            <thead>
                                <tr>
                                    <th class=style::th>"Started"</th>
                                    <th class=style::th>"Profile"</th>
                                    <th class=style::th>"Importer"</th>
                                    <th class=style::th>"State"</th>
                                    <th class=style::th>"Outcome"</th>
                                    <th class=style::th>"Actions"</th>
                                </tr>
                            </thead>
                            {list
                                .into_iter()
                                .map(|info| {
                                    view! {
                                        <BatchRow
                                            info=info
                                            highlight=highlight
                                            expanded=expanded
                                            armed=armed
                                            last_discard=last_discard
                                            version=version
                                            stores=stores
                                        />
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </table>
                    }
                        .into_any()
                }
            }}
        </section>
    }
}

/// One batch as a `<tbody>`: its row and, when expanded, its details.
#[component]
fn BatchRow(
    /// The batch.
    info: ImportBatchInfo,
    /// The `?batch=` value.
    highlight: Memo<Option<String>>,
    /// The expanded batch's ID.
    expanded: RwSignal<Option<String>>,
    /// The armed batch's ID.
    armed: RwSignal<Option<String>>,
    /// The last confirmed discard and whether its counts moved.
    last_discard: RwSignal<Option<(String, bool)>>,
    /// History's version.
    version: RwSignal<u32>,
    /// Stores to refresh after a discard.
    stores: SharedStores,
) -> impl IntoView {
    let tz = TimeZone::system();
    let id = info.id.clone();
    let (pill, tone) = history_model::state_pill(&info.state);
    let started = history_model::short_timestamp(&info.started_at, &tz);
    let name = history_model::batch_name(&info).to_owned();
    let outcome = history_model::outcome_text(&info);
    let details = StoredValue::new(history_model::detail_lines(&info, &tz));
    let discarded = StoredValue::new(
        info.discard
            .as_ref()
            .filter(|_| history_model::is_discarded(&info.state))
            .map(|d| {
                (
                    history_model::short_timestamp(&d.discarded_at, &tz),
                    d.snapshot.clone(),
                )
            }),
    );

    let body_ref = NodeRef::<Tbody>::new();
    let hl_id = id.clone();
    let highlighted =
        Signal::derive(move || highlight.with(|h| h.as_deref() == Some(hl_id.as_str())));
    // Scrolls only; writing a signal here would re-render the node it reads.
    Effect::new(move |_| {
        if highlighted.get()
            && let Some(el) = body_ref.get()
        {
            el.scroll_into_view();
        }
    });
    let open_id = id.clone();
    let is_open = Signal::derive(move || expanded.with(|e| e.as_deref() == Some(open_id.as_str())));
    let toggle_id = id.clone();
    let toggle = move |_| {
        expanded.update(|e| {
            *e = if e.as_deref() == Some(toggle_id.as_str()) {
                None
            } else {
                Some(toggle_id.clone())
            };
        });
    };
    let can_discard = !history_model::is_discarded(&info.state);
    let outcome_lines = StoredValue::new(
        info.discard
            .as_ref()
            .map(|d| discard_model::consequences(&d.counts, Tense::Past))
            .unwrap_or_default(),
    );
    let arm_id = id.clone();
    let armed_id = id.clone();
    let is_armed = Signal::derive(move || armed.with(|a| a.as_deref() == Some(armed_id.as_str())));
    let changed_id = id.clone();
    let counts_changed = Signal::derive(move || {
        last_discard.with(|l| matches!(l, Some((lid, true)) if *lid == changed_id))
    });
    let panel_id = id.clone();

    view! {
        <tbody
            node_ref=body_ref
            data-testid=format!("import-batch-row-{id}")
            data-highlighted=move || highlighted.get().to_string()
            class=move || { if highlighted.get() { style::highlight } else { "" } }
        >
            <tr class=style::row>
                <td class=style::td_date>{started}</td>
                <td class=style::td>{name}</td>
                <td class=style::td>
                    <code class=style::mono>{info.importer}</code>
                </td>
                <td class=style::td>
                    <StatusPill label=pill.to_owned() tone=tone />
                </td>
                <td class=style::td>{outcome}</td>
                <td class=style::td>
                    <button
                        type="button"
                        class=style::btn
                        aria-expanded=move || is_open.get().to_string()
                        on:click=toggle
                    >
                        "Details"
                    </button>
                    {can_discard
                        .then(|| {
                            view! {
                                <button
                                    type="button"
                                    class=style::btn
                                    data-testid="discard-arm"
                                    on:click=move |_| armed.set(Some(arm_id.clone()))
                                >
                                    "Discard\u{2026}"
                                </button>
                            }
                        })}
                </td>
            </tr>
            <Show when=move || is_open.get()>
                <tr>
                    <td class=style::panel_cell colspan="6">
                        <ul class=style::detail_list>
                            {details
                                .get_value()
                                .into_iter()
                                .map(|l| view! { <li>{l}</li> })
                                .collect::<Vec<_>>()}
                        </ul>
                        <Show when=move || counts_changed.get()>
                            <p class=style::warn data-testid="discard-counts-changed">
                                {COUNTS_CHANGED_TEXT}
                            </p>
                        </Show>
                        <ul class=style::consequences>
                            {outcome_lines
                                .get_value()
                                .into_iter()
                                .map(|c| {
                                    let class = if c.warn { style::warn } else { "" };
                                    view! { <li class=class>{c.text}</li> }
                                })
                                .collect::<Vec<_>>()}
                        </ul>
                        {discarded
                            .get_value()
                            .map(|(at, snapshot)| {
                                view! {
                                    <p class=style::muted>{format!("Discarded {at}.")}</p>
                                    {match snapshot {
                                        Some(path) => {
                                            view! {
                                                <a
                                                    class=style::link
                                                    href=links::backup_href(&path)
                                                    title=path.clone()
                                                    data-testid="restore-snapshot-link"
                                                >
                                                    "Restore snapshot"
                                                </a>
                                            }
                                                .into_any()
                                        }
                                        None => {
                                            view! { <p class=style::muted>"No snapshot was taken."</p> }
                                                .into_any()
                                        }
                                    }}
                                }
                            })}
                    </td>
                </tr>
            </Show>
            <Show when=move || is_armed.get()>
                <tr>
                    <td class=style::panel_cell colspan="6">
                        <DiscardPanel
                            batch_id=panel_id.clone()
                            armed=armed
                            expanded=expanded
                            version=version
                            last_discard=last_discard
                            stores=stores
                        />
                    </td>
                </tr>
            </Show>
        </tbody>
    }
}
