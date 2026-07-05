//! Transfer-suggestion review panel — merge or dismiss proposed transfer pairs.

#[cfg(debug_assertions)]
pub mod qa;

use bc_ipc::TransferSuggestion;
use leptos::prelude::*;
use leptos::task::spawn_local;
use stylance::import_style;

use crate::components::error_banner::ErrorBanner;
use crate::components::num::Num;
use crate::components::toast::ToastAction;
use crate::components::toast::ToastKind;
use crate::components::toast::use_toasts;

import_style!(style, "transfers.module.scss");

/// Stable key for a suggestion: the ordered `(debit, credit)` id pair.
fn key(s: &TransferSuggestion) -> (String, String) {
    (s.debit.clone(), s.credit.clone())
}

/// Transfer-suggestion review panel.
///
/// Fetches suggestions once via `suggest_transfers`, then lets the user merge or
/// dismiss each pair. Dismissal is session-only; Refresh re-queries the backend.
#[component]
#[expect(clippy::too_many_lines, reason = "large view! block")]
pub fn TransfersPanel() -> impl IntoView {
    let resource =
        LocalResource::new(move || async move { bc_ipc::client::suggest_transfers().await });

    // The working list the UI mutates. `None` until the first load resolves.
    let working = RwSignal::new(None::<Vec<TransferSuggestion>>);

    // Mirror each successful load into the working list.
    Effect::new(move |_| {
        if let Some(Ok(list)) = resource.get() {
            working.set(Some(list));
        }
    });

    let toasts = use_toasts();

    let dismiss: Callback<TransferSuggestion> = Callback::new(move |s: TransferSuggestion| {
        working.update(|opt| {
            if let Some(list) = opt.as_mut() {
                list.retain(|x| key(x) != key(&s));
            }
        });
    });

    let merge: Callback<TransferSuggestion> = Callback::new(move |s: TransferSuggestion| {
        let survivor = s.debit.clone();
        let absorbed = s.credit.clone();
        spawn_local(async move {
            match bc_ipc::client::merge_transactions(&survivor, &absorbed).await {
                Ok(()) => {
                    working.update(|opt| {
                        if let Some(list) = opt.as_mut() {
                            list.retain(|x| key(x) != key(&s));
                        }
                    });
                    let restore = s.clone();
                    let undo_survivor = survivor.clone();
                    toasts.push(
                        ToastKind::Success,
                        "Transactions merged.",
                        Some(ToastAction {
                            label: "Undo".to_owned(),
                            on_activate: Callback::new(move |()| {
                                let restore = restore.clone();
                                let undo_survivor = undo_survivor.clone();
                                spawn_local(async move {
                                    match bc_ipc::client::unmerge_transaction(&undo_survivor).await
                                    {
                                        Ok(_) => {
                                            working.update(|opt| {
                                                if let Some(list) = opt.as_mut() {
                                                    list.push(restore.clone());
                                                }
                                            });
                                        }
                                        Err(e) => {
                                            use_toasts().push(
                                                ToastKind::Error,
                                                format!("Undo failed: {e}"),
                                                None,
                                            );
                                        }
                                    }
                                });
                            }),
                        }),
                    );
                }
                Err(e) => {
                    toasts.push(ToastKind::Error, format!("Merge failed: {e}"), None);
                }
            }
        });
    });

    let refresh = move |_| resource.refetch();

    view! {
        <div class=style::panel>
            <div class=style::header>
                <span class=style::title>"Transfer suggestions"</span>
                <button type="button" class=style::btn on:click=refresh>
                    "Refresh"
                </button>
            </div>
            {move || match (resource.get(), working.get()) {
                (Some(Err(e)), _) => {
                    view! {
                        <div class=style::error>
                            <ErrorBanner message=format!(
                                "Failed to load transfer suggestions: {e}",
                            ) />
                        </div>
                    }
                        .into_any()
                }
                (_, Some(list)) if list.is_empty() => {
                    view! { <p class=style::empty>"No transfer suggestions."</p> }.into_any()
                }
                (_, Some(list)) => {
                    view! {
                        <div class=style::list>
                            {list
                                .into_iter()
                                .map(|s| {
                                    view! {
                                        <SuggestionCard
                                            suggestion=s.clone()
                                            on_merge=merge
                                            on_dismiss=dismiss
                                        />
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </div>
                    }
                        .into_any()
                }
                _ => {
                    // Covers `(None, None)` (initial load) and `(Some(Ok(_)), None)` (the
                    // resource resolved but the syncing `Effect` has not yet run).
                    view! {
                        <div class=style::list>
                            <div class=style::skeleton_card />
                            <div class=style::skeleton_card />
                        </div>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

/// A single suggestion card with Dismiss / Merge actions.
#[component]
fn SuggestionCard(
    /// The pair to display.
    suggestion: TransferSuggestion,
    /// Invoked with the suggestion when Merge is clicked.
    #[prop(into)]
    on_merge: Callback<TransferSuggestion>,
    /// Invoked with the suggestion when Dismiss is clicked.
    #[prop(into)]
    on_dismiss: Callback<TransferSuggestion>,
) -> impl IntoView {
    let merge_s = suggestion.clone();
    let dismiss_s = suggestion.clone();
    view! {
        <div class=style::card data-testid="transfer-suggestion">
            <div class=style::card_top>
                <span class=style::amount>
                    <Num money=suggestion.amount.clone() />
                </span>
                <span class=style::date>{suggestion.date_debit.clone()}</span>
            </div>
            <div class=style::leg>
                <span class=style::leg_dir>"debit"</span>
                <span class=style::leg_account>{suggestion.debit_account.clone()}</span>
                <span class=style::leg_narration>{suggestion.debit_narration.clone()}</span>
            </div>
            <div class=style::leg>
                <span class=style::leg_dir>"credit"</span>
                <span class=style::leg_account>{suggestion.credit_account.clone()}</span>
                <span class=style::leg_narration>{suggestion.credit_narration.clone()}</span>
            </div>
            <div class=style::actions>
                <button
                    type="button"
                    class=style::btn
                    on:click=move |_| on_dismiss.run(dismiss_s.clone())
                >
                    "Dismiss"
                </button>
                <button
                    type="button"
                    class=format!("{} {}", style::btn, style::btn_primary)
                    data-testid="transfer-merge"
                    on:click=move |_| on_merge.run(merge_s.clone())
                >
                    "Merge"
                </button>
            </div>
        </div>
    }
}
