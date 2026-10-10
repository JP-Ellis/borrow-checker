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
    /// The batch whose `discard_batch` call is in flight; held above the panel
    /// so a remounted panel still disables its controls.
    discarding: RwSignal<Option<String>>,
) -> impl IntoView {
    let state = RwSignal::new(DiscardState::Loading);
    let toasts = use_toasts();
    let tz = TimeZone::system();
    let confirming = Signal::derive({
        let id = batch_id.clone();
        move || discarding.with(|d| discard_model::is_discarding(d.as_deref(), &id))
    });

    load_preview(state, batch_id.clone());

    let confirm = Callback::new(move |()| {
        let Some(previewed) = state.with_untracked(|s| s.ready_counts().cloned()) else {
            return;
        };
        if discarding.with_untracked(Option::is_some) {
            return;
        }
        discarding.set(Some(batch_id.clone()));
        let id = batch_id.clone();
        // The panel may unmount while the call is in flight, so everything it
        // owns is touched through `try_*` and `is_disposed`; the rest lives above it.
        leptos::task::spawn_local(async move {
            match bc_ipc::client::discard_batch(&id).await {
                Ok(info) => {
                    last_discard.try_set(Some((id.clone(), previewed != info.counts)));
                    armed.try_set(None);
                    expanded.try_set(Some(id));
                    version.try_update(|v| *v = v.wrapping_add(1));
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
                        DiscardFailure::Blocked => {
                            if !state.is_disposed() {
                                load_preview(state, id);
                            }
                        }
                        DiscardFailure::AlreadyDiscarded => {
                            toasts.push(ToastKind::Info, ALREADY_DISCARDED_TEXT, None);
                            armed.try_set(None);
                            version.try_update(|v| *v = v.wrapping_add(1));
                            stores.refresh();
                        }
                        DiscardFailure::Other(message) => {
                            state.try_set(DiscardState::Error(message));
                        }
                    }
                }
            }
            discarding.try_set(None);
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
        DiscardState::Error(message) => view! {
            <div class=style::discard_panel>
                <ErrorBanner message=message />
                <Show when=move || !confirming.get()>
                    <div class=style::actions>
                        <button
                            type="button"
                            class=style::btn
                            data-testid="discard-close"
                            on:click=move |_| armed.set(None)
                        >
                            "Close"
                        </button>
                    </div>
                </Show>
            </div>
        }
        .into_any(),
    }
}

/// Fetches the discard preview into `state`; a disposed panel is left alone.
fn load_preview(state: RwSignal<DiscardState>, id: String) {
    state.set(DiscardState::Loading);
    leptos::task::spawn_local(async move {
        let reply = bc_ipc::client::preview_discard(&id).await;
        state.try_set(discard_model::after_preview_discard(reply));
    });
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
    confirming: Signal<bool>,
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
                    disabled=move || confirming.get()
                    on:click=move |_| armed.set(None)
                >
                    "Cancel"
                </button>
            </div>
        </div>
    }
    .into_any()
}
