//! QA page for [`super::StickyBar`].

use bc_ipc::BcError;
use bc_ipc::BudgetOverview;
use bc_ipc::BudgetSummary;
use bc_ipc::Period;
use jiff::civil::Date;
use leptos::prelude::*;

use super::StickyBar;
use crate::pages::budget::BudgetPageCtx;

/// Creates a fixture [`BudgetSummary`] with realistic loaded values.
fn loaded_summary() -> BudgetSummary {
    BudgetSummary::builder().red(2).warn(1).green(4).build()
}

/// Creates a fixture [`BudgetSummary`] with every row on track.
fn on_track_summary() -> BudgetSummary {
    BudgetSummary::builder().green(6).build()
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

    let overview: LocalResource<Result<BudgetOverview, BcError>> =
        LocalResource::new(move || async move {
            /* never resolves — simulates loading skeleton */
            core::future::pending::<Result<BudgetOverview, BcError>>().await
        });

    view! { <StickyBar overview=overview /> }
}

/// Loaded state with full summary data.
#[component]
fn LoadedCase() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    let summary = loaded_summary();
    let overview: LocalResource<Result<BudgetOverview, BcError>> = LocalResource::new(move || {
        let s = summary.clone();
        async move { Ok::<_, BcError>(BudgetOverview::builder().summary(s).build()) }
    });

    view! { <StickyBar overview=overview /> }
}

/// Loaded state with every row on track.
#[component]
fn OnTrackCase() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    let summary = on_track_summary();
    let overview: LocalResource<Result<BudgetOverview, BcError>> = LocalResource::new(move || {
        let s = summary.clone();
        async move { Ok::<_, BcError>(BudgetOverview::builder().summary(s).build()) }
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
    let overview: LocalResource<Result<BudgetOverview, BcError>> = LocalResource::new(move || {
        let s = summary.clone();
        async move { Ok::<_, BcError>(BudgetOverview::builder().summary(s).build()) }
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
            <Scenario title="Loaded — with data (2 red, 1 warn)">
                <LoadedCase />
            </Scenario>
            <Scenario title="Loaded — nothing over">
                <OnTrackCase />
            </Scenario>
            <Scenario title="Loaded — weekly period window">
                <WeeklyCase />
            </Scenario>
        </div>
    }
}
