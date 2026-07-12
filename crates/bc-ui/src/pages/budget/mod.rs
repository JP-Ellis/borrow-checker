//! Budget page — allocation grid, period header, and accrual editor.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

#[cfg(target_arch = "wasm32")]
pub(crate) mod components;
pub(crate) mod query;

#[cfg(target_arch = "wasm32")]
use bc_ipc::BcError;
#[cfg(target_arch = "wasm32")]
use bc_ipc::BudgetSummary;
#[cfg(target_arch = "wasm32")]
use bc_ipc::BudgetTreeNode;
#[cfg(target_arch = "wasm32")]
use bc_ipc::Period;
#[cfg(target_arch = "wasm32")]
use components::budget_tree::BudgetTree;
#[cfg(target_arch = "wasm32")]
use components::header::BudgetHeader;
#[cfg(target_arch = "wasm32")]
use components::new_budget::NewBudget;
#[cfg(target_arch = "wasm32")]
use components::sticky_bar::StickyBar;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
import_style!(style, "budget.module.scss");

/// Shared page-level reactive state provided via context to all child components.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct BudgetPageCtx {
    /// The currently selected display period granularity.
    pub display_period: RwSignal<Period>,
    /// Start date of the selected display window.
    pub window_start: RwSignal<jiff::civil::Date>,
    /// When `true`, budget amounts are displayed as percentages of target.
    pub pct_mode: RwSignal<bool>,
    /// ID of the currently open detail panel, or `None` when closed.
    pub open_detail_id: RwSignal<Option<String>>,
    /// Bumped after any mutation to trigger data re-fetch across all subscribers.
    pub data_version: RwSignal<u32>,
}

#[cfg(target_arch = "wasm32")]
impl BudgetPageCtx {
    /// Creates a new [`BudgetPageCtx`] with sensible defaults.
    ///
    /// The display period defaults to [`Period::Monthly`] and the window start
    /// is set to the first day of the current calendar month.
    #[must_use]
    #[inline]
    #[expect(
        clippy::expect_used,
        reason = "Date::new with day=1 is always valid for any year/month from jiff::Zoned::now()"
    )]
    pub fn new() -> Self {
        let now = jiff::Zoned::now();
        let today = now.date();
        let window_start = jiff::civil::Date::new(today.year(), today.month(), 1)
            .expect("first of current month is always valid");

        Self {
            display_period: RwSignal::new(Period::Monthly),
            window_start: RwSignal::new(window_start),
            pct_mode: RwSignal::new(false),
            open_detail_id: RwSignal::new(None),
            data_version: RwSignal::new(0_u32),
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Default for BudgetPageCtx {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// Budget page — header KPIs, period controls, and the allocation tree.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn Budget() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    let filter_store = crate::filter_ctx::use_filter_store();

    let show_new = RwSignal::new(false);

    let overview: LocalResource<Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError>> =
        LocalResource::new(move || {
            ctx.data_version.get();
            let period = ctx.display_period.get();
            let start = ctx.window_start.get();
            let eff = filter_store.filter.with(query::budget_effective_filter);
            async move {
                let filter = (eff != bc_ipc::Filter::default()).then_some(eff);
                bc_ipc::client::get_budget_overview(period, start, filter.as_ref()).await
            }
        });

    let on_created = Callback::new(move |()| {
        ctx.data_version.update(|v| *v = v.saturating_add(1));
        show_new.set(false);
    });
    let on_cancel = Callback::new(move |()| show_new.set(false));

    view! {
        <div class=style::page>
            <BudgetHeader overview=overview />
            <StickyBar overview=overview />

            <Show when=move || show_new.get()>
                <div style="padding:var(--bc-space-3) var(--bc-space-6);max-width:480px">
                    <NewBudget on_created=on_created on_cancel=on_cancel />
                </div>
            </Show>

            <div class=style::tree_container>
                <Suspense fallback=move || {
                    view! { <div class=style::loading>"Loading budgets\u{2026}"</div> }
                }>
                    {move || {
                        overview
                            .get()
                            .map(|result| match result {
                                Err(e) => {
                                    view! {
                                        <div class=style::error_banner>
                                            {format!("Error loading budgets: {e}")}
                                        </div>
                                    }
                                        .into_any()
                                }
                                Ok((_, nodes)) if nodes.is_empty() => {
                                    view! {
                                        <div class=style::empty_state>
                                            <p>"// no budgets yet"</p>
                                            <button
                                                style="margin-top:var(--bc-space-3)"
                                                on:click=move |_| show_new.set(true)
                                            >
                                                "New budget"
                                            </button>
                                        </div>
                                    }
                                        .into_any()
                                }
                                Ok((_, nodes)) => {
                                    view! {
                                        <div>
                                            <div style="padding:var(--bc-space-2) var(--bc-space-6)">
                                                <button on:click=move |_| {
                                                    show_new.set(true);
                                                }>"New budget"</button>
                                            </div>
                                            <BudgetTree nodes=nodes />
                                        </div>
                                    }
                                        .into_any()
                                }
                            })
                    }}
                </Suspense>
            </div>
        </div>
    }
}
