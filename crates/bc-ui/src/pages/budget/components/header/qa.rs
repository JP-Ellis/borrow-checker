//! QA page for [`super::BudgetHeader`].

use bc_ipc::Amount;
use bc_ipc::BcError;
use bc_ipc::BudgetOverview;
use bc_ipc::BudgetSummary;
use bc_ipc::Period;
use jiff::civil::Date;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::BudgetHeader;
use crate::pages::budget::BudgetPageCtx;

/// Creates a fixture [`BudgetSummary`] with realistic values.
fn loaded_summary() -> BudgetSummary {
    BudgetSummary::builder()
        .red(1)
        .warn(2)
        .green(5)
        .unbudgeted(vec![(
            "Expenses".to_owned(),
            Amount::new(Decimal::new(18_755, 2), "AUD"),
        )])
        .build()
}

/// Creates a fixture [`BudgetSummary`] with no budgets.
fn empty_summary() -> BudgetSummary {
    BudgetSummary::default()
}

/// A loaded summary with at least one row that has unvalued spend.
fn unvalued_summary() -> BudgetSummary {
    BudgetSummary::builder()
        .red(1)
        .warn(2)
        .green(5)
        .has_unvalued(true)
        .build()
}

/// Wraps a scenario in a labelled box with a fresh context.
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

    let overview: LocalResource<Result<BudgetOverview, BcError>> =
        LocalResource::new(move || async move {
            /* never resolves in QA — simulates loading skeleton */
            core::future::pending::<Result<BudgetOverview, BcError>>().await
        });

    view! { <BudgetHeader overview=overview /> }
}

/// Loaded state with full summary data and 1 overspent line.
#[component]
fn LoadedCase() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    let summary = loaded_summary();
    let overview: LocalResource<Result<BudgetOverview, BcError>> = LocalResource::new(move || {
        let s = summary.clone();
        async move { Ok::<_, BcError>(BudgetOverview::builder().summary(s).build()) }
    });

    view! { <BudgetHeader overview=overview /> }
}

/// Loaded state with a leaf that has unvalued spend.
#[component]
fn UnvaluedCase() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    let summary = unvalued_summary();
    let overview: LocalResource<Result<BudgetOverview, BcError>> = LocalResource::new(move || {
        let s = summary.clone();
        async move { Ok::<_, BcError>(BudgetOverview::builder().summary(s).build()) }
    });

    view! { <BudgetHeader overview=overview /> }
}

/// Loaded state with no budget data.
#[component]
fn EmptyCase() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    let summary = empty_summary();
    let overview: LocalResource<Result<BudgetOverview, BcError>> = LocalResource::new(move || {
        let s = summary.clone();
        async move { Ok::<_, BcError>(BudgetOverview::builder().summary(s).build()) }
    });

    view! { <BudgetHeader overview=overview /> }
}

/// Loaded state with context starting on a non-monthly period (weekly).
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
    let overview: LocalResource<Result<BudgetOverview, BcError>> = LocalResource::new(move || {
        let s = summary.clone();
        async move { Ok::<_, BcError>(BudgetOverview::builder().summary(s).build()) }
    });

    view! { <BudgetHeader overview=overview /> }
}

/// QA showcase for [`BudgetHeader`].
#[component]
pub fn BudgetHeaderQa() -> impl IntoView {
    view! {
        <div style="padding:24px;max-width:900px">
            <Scenario title="Loading (pending resource)">
                <LoadingCase />
            </Scenario>
            <Scenario title="Loaded — with data (1 overspent line)">
                <LoadedCase />
            </Scenario>
            <Scenario title="unvalued spend">
                <UnvaluedCase />
            </Scenario>
            <Scenario title="Loaded — no budgets">
                <EmptyCase />
            </Scenario>
            <Scenario title="Weekly period window">
                <WeeklyCase />
            </Scenario>
        </div>
    }
}
