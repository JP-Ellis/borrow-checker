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

/// A committed run, top to bottom: files that did not parse, warnings, the
/// counts line, skips by cause, rejected legs, unresolved accounts,
/// unregistered commodities, created tags with the tag hint, created
/// accounts, the opens and closes the declarations applied, the snapshot,
/// then a link to its History row and Dismiss. An empty list renders nothing.
#[component]
pub fn OutcomePanel(
    /// The run's outcome.
    result: ImportResult,
) -> impl IntoView {
    let ctx = expect_context::<ImportCtx>();
    let counts = model::counts_line(
        result.new_transactions,
        result.attached_postings,
        u64::try_from(result.declared.len()).unwrap_or(u64::MAX),
        result.skipped_postings,
    );
    let rejected = model::rejected_line(result.skipped_rejected).map(|line| {
        view! {
            <p class=style::muted data-testid="import-outcome-rejected">
                {line}
            </p>
        }
    });
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
    let skips = (!result.skips_by_cause.is_empty()).then(|| {
        view! {
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
        }
    });
    let declared = (!result.declared.is_empty()).then(|| {
        view! {
            <ul class=style::block_list data-testid="import-outcome-declared">
                {result
                    .declared
                    .iter()
                    .map(|d| view! { <li>{model::declared_line(d)}</li> })
                    .collect::<Vec<_>>()}
            </ul>
        }
    });
    let failed = model::failed_sources(&result.sources);
    let tag_hint =
        (!result.created_tags.is_empty()).then(|| view! { <p class=style::muted>{TAG_HINT}</p> });
    view! {
        <div class=style::panel data-testid="import-outcome" role="status" aria-live="polite">
            <FailedFiles failed=failed />
            {result
                .warnings
                .into_iter()
                .map(|w| view! { <p class=style::warn>{w}</p> })
                .collect::<Vec<_>>()}
            <p class=style::summary>{counts}</p>
            {skips}
            {rejected}
            {list(result.unresolved_accounts, "Unresolved account")}
            {list(result.unresolved_commodities, "Unregistered commodity")}
            {list(result.created_tags, "Created tag")}
            {tag_hint}
            {list(result.created_accounts, "Created account")}
            {declared}
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

/// The files a committed run could not read or parse, so did not import.
/// Renders nothing when every file parsed.
#[component]
fn FailedFiles(
    /// `(source, reason)` per failed file.
    failed: Vec<(String, String)>,
) -> impl IntoView {
    (!failed.is_empty()).then(|| {
        view! {
            <ul class=style::block_list data-testid="import-outcome-failed-files">
                {failed
                    .into_iter()
                    .map(|(source, reason)| {
                        view! {
                            <li class=style::warn>
                                "File " <code class=style::mono>{source}</code>
                                " was not imported: " {reason} ". Fix it, then import again."
                            </li>
                        }
                    })
                    .collect::<Vec<_>>()}
            </ul>
        }
    })
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
