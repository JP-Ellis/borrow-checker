//! QA page for [`super::StickyBar`].

use bc_ipc::Amount;
use bc_ipc::BcError;
use bc_ipc::BudgetSummary;
use bc_ipc::BudgetTreeNode;
use bc_ipc::Period;
use jiff::civil::Date;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::StickyBar;
use crate::pages::budget::BudgetPageCtx;

/// Creates a fixture [`BudgetSummary`] with realistic loaded values.
fn loaded_summary() -> BudgetSummary {
    BudgetSummary::new(
        Some(Amount::new(Decimal::new(500_000, 2), "AUD")),
        Some(Amount::new(Decimal::new(312_450, 2), "AUD")),
        Some(Amount::new(Decimal::new(187_550, 2), "AUD")),
        false,
        2,
    )
}

/// Creates a fixture [`BudgetSummary`] flagged with mixed commodities.
fn mixed_summary() -> BudgetSummary {
    BudgetSummary::new(None, None, None, true, 0)
}

/// Wraps a scenario in a labelled box.
#[component]
fn Scenario(
    /// Title displayed above the component instance.
    title: &'static str,
    /// Child view rendered inside the scenario box.
    children: Children,
) -> impl IntoView {
    view! {
        <div style="margin-bottom:32px">
            <p style="font-family:var(--bc-font-mono);font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;text-transform:uppercase;letter-spacing:0.05em">
                {title}
            </p>
            {children()}
        </div>
    }
}

/// Loading state — the resource is pending.
#[component]
fn LoadingCase() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    let overview: LocalResource<Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError>> =
        LocalResource::new(move || async move {
            /* never resolves — simulates loading skeleton */
            core::future::pending::<Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError>>().await
        });

    view! { <StickyBar overview=overview /> }
}

/// Loaded state with full summary data.
#[component]
fn LoadedCase() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    let summary = loaded_summary();
    let overview: LocalResource<Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError>> =
        LocalResource::new(move || {
            let s = summary.clone();
            async move { Ok::<_, BcError>((s, vec![])) }
        });

    view! { <StickyBar overview=overview /> }
}

/// Loaded state with mixed-currency summary.
#[component]
fn MixedCurrenciesCase() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    let summary = mixed_summary();
    let overview: LocalResource<Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError>> =
        LocalResource::new(move || {
            let s = summary.clone();
            async move { Ok::<_, BcError>((s, vec![])) }
        });

    view! { <StickyBar overview=overview /> }
}

/// Loaded state with a weekly period window.
#[component]
fn WeeklyCase() -> impl IntoView {
    #[expect(
        clippy::expect_used,
        reason = "hardcoded QA date constant — cannot fail"
    )]
    let window = Date::new(2026, 6, 15).expect("valid date");
    let ctx = BudgetPageCtx {
        display_period: RwSignal::new(Period::Weekly),
        window_start: RwSignal::new(window),
        ..BudgetPageCtx::new()
    };
    provide_context(ctx);

    let summary = loaded_summary();
    let overview: LocalResource<Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError>> =
        LocalResource::new(move || {
            let s = summary.clone();
            async move { Ok::<_, BcError>((s, vec![])) }
        });

    view! { <StickyBar overview=overview /> }
}

/// QA showcase for [`StickyBar`].
#[component]
pub fn StickyBarQa() -> impl IntoView {
    view! {
        <div style="padding:24px;max-width:900px">
            <Scenario title="Loading (pending resource)">
                <LoadingCase />
            </Scenario>
            <Scenario title="Loaded — with data (2 overspent lines)">
                <LoadedCase />
            </Scenario>
            <Scenario title="Loaded — mixed currencies">
                <MixedCurrenciesCase />
            </Scenario>
            <Scenario title="Loaded — weekly period window">
                <WeeklyCase />
            </Scenario>
        </div>
    }
}
