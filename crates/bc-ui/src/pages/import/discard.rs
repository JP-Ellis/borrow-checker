//! An armed discard: its rolled-back consequences, then confirm.

use bc_ipc::DiscardCounts;
use bc_ipc::DiscardDependant;
use jiff::tz::TimeZone;
use leptos::prelude::*;

use crate::components::error_banner::ErrorBanner;
use crate::components::toast::ToastKind;
use crate::components::toast::use_toasts;
use crate::pages::import::discard_model;
use crate::pages::import::discard_model::DiscardFailure;
use crate::pages::import::discard_model::DiscardState;
use crate::pages::import::discard_model::Tense;
use crate::pages::import::history_model;
use crate::pages::import::links;
use crate::pages::import::refresh::SharedStores;
use crate::pages::import::style;

/// The Blocked panel's explanation.
const BLOCKED_TEXT: &str =
    "Later batches own legs on this batch's transactions. Discard them first.";

/// The notice when a confirm finds the batch already discarded.
const ALREADY_DISCARDED_TEXT: &str = "That batch was already discarded. History is up to date.";

/// The note above a discard's real figures when they differ from its preview.
pub(crate) const COUNTS_CHANGED_TEXT: &str = "Counts changed since the preview.";

/// Previews the discard of `batch_id` on mount, then offers confirm, or
/// lists the batches blocking it.
#[component]
pub fn DiscardPanel(
    /// The batch to discard.
    batch_id: String,
    /// The armed batch's ID; cleared to collapse this panel.
    armed: RwSignal<Option<String>>,
    /// The expanded batch's ID; set to show the discarded row's details.
    expanded: RwSignal<Option<String>>,
    /// History's version; bumped after a discard.
    version: RwSignal<u32>,
    /// The last confirmed discard and whether its counts moved since the preview.
    last_discard: RwSignal<Option<(String, bool)>>,
    /// Stores to refresh after a discard.
    stores: SharedStores,
) -> impl IntoView {
    let state = RwSignal::new(DiscardState::Loading);
    let confirming = RwSignal::new(false);
    let toasts = use_toasts();
    let tz = TimeZone::system();

    let load_id = batch_id.clone();
    let reload = Callback::new(move |()| {
        let id = load_id.clone();
        state.set(DiscardState::Loading);
        leptos::task::spawn_local(async move {
            state.set(discard_model::after_preview_discard(
                bc_ipc::client::preview_discard(&id).await,
            ));
        });
    });
    reload.run(());

    let confirm = Callback::new(move |()| {
        let Some(previewed) = state.with_untracked(|s| s.ready_counts().cloned()) else {
            return;
        };
        if confirming.get_untracked() {
            return;
        }
        confirming.set(true);
        let id = batch_id.clone();
        leptos::task::spawn_local(async move {
            match bc_ipc::client::discard_batch(&id).await {
                Ok(info) => {
                    last_discard.set(Some((id.clone(), previewed != info.counts)));
                    armed.set(None);
                    expanded.set(Some(id));
                    version.update(|v| *v = v.wrapping_add(1));
                    stores.refresh();
                }
                Err(err) => {
                    let now_discarded =
                        bc_ipc::client::list_import_batches()
                            .await
                            .is_ok_and(|list| {
                                list.iter()
                                    .any(|b| b.id == id && history_model::is_discarded(&b.state))
                            });
                    match discard_model::classify(&err, now_discarded) {
                        DiscardFailure::Blocked => reload.run(()),
                        DiscardFailure::AlreadyDiscarded => {
                            toasts.push(ToastKind::Info, ALREADY_DISCARDED_TEXT, None);
                            armed.set(None);
                            version.update(|v| *v = v.wrapping_add(1));
                        }
                        DiscardFailure::Other(message) => state.set(DiscardState::Error(message)),
                    }
                }
            }
            confirming.set(false);
        });
    });

    move || match state.get() {
        DiscardState::Loading => view! {
            <p class=style::spinner role="status" aria-live="polite">
                "Working out what discard would change\u{2026}"
            </p>
        }
        .into_any(),
        DiscardState::Blocked(dependants) => blocked_view(&dependants, &tz, armed),
        DiscardState::Ready {
            counts,
            snapshot_planned,
        } => ready_view(&counts, snapshot_planned, confirming, confirm, armed),
        DiscardState::Error(message) => view! { <ErrorBanner message=message /> }.into_any(),
    }
}

/// The Blocked panel: the later batches to discard first.
fn blocked_view(
    dependants: &[DiscardDependant],
    tz: &TimeZone,
    armed: RwSignal<Option<String>>,
) -> AnyView {
    view! {
        <div class=style::discard_panel role="alert" data-testid="discard-blocked">
            <p class=style::warn>{BLOCKED_TEXT}</p>
            <ul class=style::detail_list>
                {dependants
                    .iter()
                    .map(|d| {
                        view! {
                            <li>
                                <a class=style::link href=links::history_href(&d.batch_id, false)>
                                    {discard_model::dependant_text(d, tz)}
                                </a>
                            </li>
                        }
                    })
                    .collect::<Vec<_>>()}
            </ul>
            <button type="button" class=style::btn on:click=move |_| armed.set(None)>
                "Close"
            </button>
        </div>
    }
    .into_any()
}

/// The Ready panel: the consequences, the safety line, confirm and cancel.
fn ready_view(
    counts: &DiscardCounts,
    snapshot_planned: bool,
    confirming: RwSignal<bool>,
    confirm: Callback<()>,
    armed: RwSignal<Option<String>>,
) -> AnyView {
    let (safety, safety_warn) = discard_model::safety_line(snapshot_planned);
    let safety_class = if safety_warn {
        style::warn
    } else {
        style::muted
    };
    view! {
        <div class=style::discard_panel data-testid="discard-consequences">
            <ul class=style::consequences>
                {discard_model::consequences(counts, Tense::Future)
                    .into_iter()
                    .map(|c| {
                        let class = if c.warn { style::warn } else { "" };
                        view! { <li class=class>{c.text}</li> }
                    })
                    .collect::<Vec<_>>()}
            </ul>
            <p class=safety_class>{safety}</p>
            <div class=style::actions>
                <button
                    type="button"
                    class=style::danger
                    data-testid="discard-confirm"
                    disabled=move || confirming.get()
                    on:click=move |_| confirm.run(())
                >
                    {discard_model::confirm_label(counts)}
                </button>
                <button
                    type="button"
                    class=style::btn
                    data-testid="discard-cancel"
                    on:click=move |_| armed.set(None)
                >
                    "Cancel"
                </button>
            </div>
        </div>
    }
    .into_any()
}
