//! Budget page — allocation grid, period header, and accrual editor.

pub(crate) mod components;
pub mod period_nav;

use bc_ipc::BcError;
use bc_ipc::BudgetSummary;
use bc_ipc::BudgetTreeNode;
use bc_ipc::Period;
use components::budget_tree::BudgetTree;
use components::header::BudgetHeader;
use components::sticky_bar::StickyBar;
use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "budget.module.scss");

/// Shared page-level reactive state provided via context to all child components.
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

impl Default for BudgetPageCtx {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// Budget page — header KPIs, period controls, and the allocation tree.
#[component]
pub fn Budget() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    let overview: LocalResource<Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError>> =
        LocalResource::new(move || {
            ctx.data_version.get();
            let period = ctx.display_period.get();
            let start = ctx.window_start.get();
            async move { bc_ipc::client::get_budget_overview(period, start).await }
        });

    view! {
        <div class=style::page>
            <BudgetHeader overview=overview />
            <StickyBar overview=overview />

            <div class=style::tree_container>
                <Suspense fallback=move || {
                    view! { <div class=style::loading>"Loading budgets…"</div> }
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
                                            "// no budgets yet — create one to get started"
                                        </div>
                                    }
                                        .into_any()
                                }
                                Ok((_, nodes)) => view! { <BudgetTree nodes=nodes /> }.into_any(),
                            })
                    }}
                </Suspense>
            </div>
        </div>
    }
}
