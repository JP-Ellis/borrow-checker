//! The preview panel: what a commit would do, broken parts first.

use bc_ipc::AccountTotal;
use bc_ipc::CauseCount;
use bc_ipc::ImportPreview;
use bc_ipc::UnresolvedItem;
use leptos::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::components::num::Num;
use crate::components::toast::ToastKind;
use crate::components::toast::use_toasts;
use crate::pages::import::ImportCtx;
use crate::pages::import::links::CURRENCIES_HREF;
use crate::pages::import::model;
use crate::pages::import::model::RunState;
use crate::pages::import::rows::RowsTable;
use crate::pages::import::style;

/// The hint under tags a run would create, as the CLI words it.
pub(crate) const TAG_HINT: &str =
    "Tags named by the source are created automatically; rename or delete any that are typos.";

/// One preview, top to bottom: summary, blockers, warnings, would create,
/// would post, rows, other diagnostics, footer.
#[component]
pub fn PreviewPanel(
    /// The preview.
    preview: ImportPreview,
    /// Whether a commit found the source changed and returned this preview.
    changed: bool,
) -> impl IntoView {
    let ctx = expect_context::<ImportCtx>();
    let summary = model::summary_line(&preview);
    let label = model::commit_label(&preview);
    let skip_line = model::skip_warning(&preview);
    let others = model::other_causes(&preview.skips_by_cause);
    let profile = preview.profile;
    let repreview = Callback::new(move |()| ctx.preview.run(profile.clone()));
    let accounts = preview.unresolved_accounts;
    let commodities = preview.unresolved_commodities;
    let blockers =
        (!accounts.is_empty() || !commodities.is_empty() || !others.is_empty()).then(|| {
            view! {
                <Blockers
                    accounts=accounts
                    commodities=commodities
                    others=others
                    on_repreview=repreview
                />
            }
        });
    let warnings = preview.warnings;
    let warning_view = (!warnings.is_empty()).then(|| {
        view! {
            <div class=style::warnings data-testid="import-warnings">
                {warnings
                    .into_iter()
                    .map(|w| view! { <p class=style::warn>{w}</p> })
                    .collect::<Vec<_>>()}
            </div>
        }
    });
    let diagnostics = preview.other_diagnostics;
    let diagnostic_view = (!diagnostics.is_empty()).then(|| {
        view! {
            <section class=style::block aria-label="other diagnostics">
                <h3 class=style::block_title>"Other diagnostics"</h3>
                <ul class=style::diags>
                    {diagnostics
                        .iter()
                        .map(|d| view! { <li>{model::diagnostic_text(d)}</li> })
                        .collect::<Vec<_>>()}
                </ul>
            </section>
        }
    });
    view! {
        <div class=style::panel data-testid="import-preview">
            {changed
                .then(|| {
                    view! {
                        <p class=style::warn role="status">
                            "The source changed since you previewed. Review again."
                        </p>
                    }
                })}
            <p class=style::summary>{summary}</p>
            <p class=style::muted>"Nothing has been written."</p>
            {blockers}
            {warning_view}
            <WouldCreate accounts=preview.would_create_accounts tags=preview.would_create_tags />
            <WouldPost totals=preview.account_totals />
            <RowsTable rows=preview.rows />
            {diagnostic_view}
            <Footer label=label skip_line=skip_line on_repreview=repreview />
        </div>
    }
}

/// The warn line about skipped legs, Commit labelled with its effect, and Re-preview.
#[component]
fn Footer(
    /// The Commit label, or `None` when nothing would write.
    label: Option<String>,
    /// The skipped-legs warning, if any leg would be skipped.
    skip_line: Option<String>,
    /// Runs the preview again.
    on_repreview: Callback<()>,
) -> impl IntoView {
    let ctx = expect_context::<ImportCtx>();
    let can_commit = label.is_some();
    let text = label.unwrap_or_else(|| "Nothing to import".to_owned());
    let committing = move || ctx.run.with(|r| matches!(r, RunState::Committing { .. }));
    view! {
        <div class=style::footer>
            {skip_line.map(|w| view! { <p class=style::warn>{w}</p> })} <div class=style::actions>
                <button
                    type="button"
                    class=style::btn_primary
                    data-testid="import-commit"
                    disabled=move || !can_commit || ctx.run.with(RunState::busy)
                    on:click=move |_| ctx.commit.run(())
                >
                    {move || {
                        if committing() { "Committing\u{2026}".to_owned() } else { text.clone() }
                    }}
                </button>
                <button
                    type="button"
                    class=style::btn
                    data-testid="import-repreview"
                    disabled=move || ctx.run.with(RunState::busy)
                    on:click=move |_| on_repreview.run(())
                >
                    "Re-preview"
                </button>
            </div>
        </div>
    }
}

/// Unresolved accounts and commodities with how to fix each, other skip
/// causes, and Re-preview.
#[component]
fn Blockers(
    /// Accounts the document names that do not exist.
    accounts: Vec<UnresolvedItem>,
    /// Commodity codes that are not registered.
    commodities: Vec<UnresolvedItem>,
    /// Every other skip cause.
    others: Vec<CauseCount>,
    /// Runs the preview again.
    on_repreview: Callback<()>,
) -> impl IntoView {
    let ctx = expect_context::<ImportCtx>();
    view! {
        <section class=style::block aria-label="blockers" data-testid="import-blockers">
            <h3 class=style::block_title>"Blockers"</h3>
            <ul class=style::block_list>
                {accounts
                    .into_iter()
                    .map(|item| {
                        let command = model::account_create_command(&item.name);
                        view! {
                            <li>
                                "Account " <code class=style::mono>{item.name}</code> ": "
                                {model::legs_skipped(item.postings)} ". Create it: "
                                <CopyCommand command=command />
                            </li>
                        }
                    })
                    .collect::<Vec<_>>()}
                {commodities
                    .into_iter()
                    .map(|item| {
                        view! {
                            <li>
                                "Commodity " <code class=style::mono>{item.name}</code> ": "
                                {model::legs_skipped(item.postings)} ". "
                                <a class=style::link href=CURRENCIES_HREF>
                                    "Register it in Settings \u{2192} Currencies"
                                </a>
                            </li>
                        }
                    })
                    .collect::<Vec<_>>()}
                {others
                    .into_iter()
                    .map(|c| {
                        view! {
                            <li>{format!("{}: {}", c.cause, model::legs_skipped(c.postings))}</li>
                        }
                    })
                    .collect::<Vec<_>>()}
            </ul>
            <button
                type="button"
                class=style::btn
                disabled=move || ctx.run.with(RunState::busy)
                on:click=move |_| on_repreview.run(())
            >
                "Re-preview"
            </button>
        </section>
    }
}

/// A shell command with a button that copies it.
#[component]
fn CopyCommand(
    /// The command text.
    command: String,
) -> impl IntoView {
    let toasts = use_toasts();
    let text = command.clone();
    let copy = move |_| {
        let text = text.clone();
        leptos::task::spawn_local(async move {
            let copied = match web_sys::window() {
                Some(w) => JsFuture::from(w.navigator().clipboard().write_text(&text))
                    .await
                    .is_ok(),
                None => false,
            };
            if copied {
                toasts.push(ToastKind::Success, "Command copied.", None);
            } else {
                toasts.push(
                    ToastKind::Error,
                    "Could not copy. Select the command and copy it by hand.",
                    None,
                );
            }
        });
    };
    view! {
        <code class=style::command>{command}</code>
        <button type="button" class=style::copy aria-label="copy command" on:click=copy>
            "copy"
        </button>
    }
}

/// Accounts and tags the commit would create.
#[component]
fn WouldCreate(
    /// Accounts from the document's declarations.
    accounts: Vec<String>,
    /// Tags not yet in the tag tree.
    tags: Vec<String>,
) -> impl IntoView {
    (!accounts.is_empty() || !tags.is_empty()).then(|| {
        let tag_view = (!tags.is_empty()).then(|| {
            view! {
                <ul class=style::block_list>
                    {tags
                        .into_iter()
                        .map(|t| view! { <li>"Tag " <code class=style::mono>{t}</code></li> })
                        .collect::<Vec<_>>()}
                </ul>
                <p class=style::muted>{TAG_HINT}</p>
            }
        });
        view! {
            <section class=style::block aria-label="would create">
                <h3 class=style::block_title>"Would create"</h3>
                <ul class=style::block_list>
                    {accounts
                        .into_iter()
                        .map(|a| view! { <li>"Account " <code class=style::mono>{a}</code></li> })
                        .collect::<Vec<_>>()}
                </ul>
                {tag_view}
            </section>
        }
    })
}

/// Per-account net change by commodity.
#[component]
fn WouldPost(
    /// One entry per account the commit touches.
    totals: Vec<AccountTotal>,
) -> impl IntoView {
    (!totals.is_empty()).then(|| {
        view! {
            <section class=style::block aria-label="would post">
                <h3 class=style::block_title>"Would post"</h3>
                <table class=style::totals>
                    <tbody>
                        {totals
                            .into_iter()
                            .map(|t| {
                                let amounts = if t.amounts.is_empty() {
                                    view! { <span class=style::muted>"no net change"</span> }
                                        .into_any()
                                } else {
                                    t.amounts
                                        .into_iter()
                                        .map(|a| view! { <Num money=a /> })
                                        .collect::<Vec<_>>()
                                        .into_any()
                                };
                                view! {
                                    <tr>
                                        <td class=style::td>
                                            <code class=style::mono>{t.account}</code>
                                        </td>
                                        <td class=style::td>
                                            <span class=style::amounts>{amounts}</span>
                                        </td>
                                    </tr>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </tbody>
                </table>
            </section>
        }
    })
}
