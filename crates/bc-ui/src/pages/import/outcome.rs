//! What a commit did, and how a failed run is reported.

use bc_ipc::ImportFailure;
use bc_ipc::ImportResult;
use leptos::prelude::*;

use crate::components::error_banner::ErrorBanner;
use crate::pages::import::ImportCtx;
use crate::pages::import::links;
use crate::pages::import::model;
use crate::pages::import::preview::TAG_HINT;
use crate::pages::import::style;

/// A committed run, in the CLI's order: warnings, counts, skips by cause,
/// created tags, created accounts, the snapshot, then a link to its History row.
#[component]
pub fn OutcomePanel(
    /// The run's outcome.
    result: ImportResult,
) -> impl IntoView {
    let ctx = expect_context::<ImportCtx>();
    let counts = model::counts_line(
        result.new_transactions,
        result.attached_postings,
        result.skipped_postings,
    );
    let history = links::history_href(&result.batch_id, false);
    let list = |items: Vec<String>, noun: &'static str| {
        (!items.is_empty()).then(|| {
            view! {
                <ul class=style::block_list>
                    {items
                        .into_iter()
                        .map(|i| view! { <li>{noun} " " <code class=style::mono>{i}</code></li> })
                        .collect::<Vec<_>>()}
                </ul>
            }
        })
    };
    let tag_hint =
        (!result.created_tags.is_empty()).then(|| view! { <p class=style::muted>{TAG_HINT}</p> });
    view! {
        <div class=style::panel data-testid="import-outcome" role="status" aria-live="polite">
            {result
                .warnings
                .into_iter()
                .map(|w| view! { <p class=style::warn>{w}</p> })
                .collect::<Vec<_>>()}
            <p class=style::summary>{counts}</p>
            <ul class=style::block_list>
                {result
                    .skips_by_cause
                    .into_iter()
                    .map(|c| {
                        view! {
                            <li>{format!("{}: {}", c.cause, model::legs_skipped(c.postings))}</li>
                        }
                    })
                    .collect::<Vec<_>>()}
            </ul>
            {list(result.unresolved_accounts, "Unresolved account")}
            {list(result.unresolved_commodities, "Unregistered commodity")}
            {list(result.created_tags, "Created tag")}
            {tag_hint}
            {list(result.created_accounts, "Created account")}
            {match result.snapshot {
                Some(path) => {
                    view! {
                        <p class=style::muted>
                            "Pre-import snapshot: " <code class=style::mono>{path}</code>
                        </p>
                    }
                        .into_any()
                }
                None => view! { <p class=style::muted>"No snapshot was taken."</p> }.into_any(),
            }}
            <div class=style::actions>
                <a class=style::link href=history data-testid="import-view-history">
                    "View in history"
                </a>
                <button
                    type="button"
                    class=style::btn
                    data-testid="import-outcome-dismiss"
                    on:click=move |_| ctx.dismiss.run(())
                >
                    "Dismiss"
                </button>
            </div>
        </div>
    }
}

/// A failed run. One that wrote a partial batch links to its History row
/// with discard armed.
#[component]
pub fn FailureBanner(
    /// The failure.
    failure: ImportFailure,
) -> impl IntoView {
    let text = model::failure_text(&failure);
    match failure.batch_id {
        None => view! { <ErrorBanner message=text /> }.into_any(),
        Some(id) => view! {
            <div class=style::failure role="alert" data-testid="import-failure">
                <p>{text}</p>
                <p>
                    "The run stopped after writing; its rows stay until the batch is discarded. "
                    <a
                        class=style::link
                        href=links::history_href(&id, true)
                        data-testid="import-failure-history"
                    >
                        "Review and discard the batch"
                    </a>
                </p>
            </div>
        }
        .into_any(),
    }
}
