//! Per-account dashboard — breadcrumb, balance, stat cards, and sparkline.

use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use leptos::prelude::*;
use stylance::import_style;

use crate::components::sparkline::Sparkline;
use crate::components::sparkline::Title;
use crate::components::stat_card::StatCard;
use crate::components::stat_card::StatCards;
use crate::components::stat_card::StatTone;
use crate::components::tag_token::TagToken;

import_style!(style, "dashboard.module.scss");

/// Monotonic counter for generating unique per-instance anchor names and popover IDs.
static DASHBOARD_INSTANCE: AtomicUsize = AtomicUsize::new(0);

/// Full per-account dashboard: breadcrumb, balance headline, stat tiles, sparkline.
///
/// Scrolls away with the page — the [`StickyAccountBar`] takes over once this
/// component has left the viewport.
///
/// # Arguments
///
/// * `node` - The account to display.
/// * `data_version` - Optional monotonic counter; when it changes, stats and sparkline re-fetch.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "Period is #[non_exhaustive]; wildcard covers CalendarYear and FinancialYear in the title — TODO: dedicate title strings once the settings screen exposes financial-year configuration"
)]
pub fn AccountDashboard(
    /// Account to display.
    node: AccountNode,
    /// Monotonic counter bumped by the parent after any successful mutation.
    /// Stats and sparkline LocalResources re-fetch whenever this changes.
    #[prop(optional)]
    data_version: Option<ReadSignal<u32>>,
) -> impl IntoView {
    let account_id = node.id.clone();
    let sparkline_account_id = node.id.clone();

    let stats_resource = LocalResource::new(move || {
        let id = account_id.clone();
        if let Some(v) = data_version {
            v.get();
        }
        async move { bc_ipc::client::get_account_stats(&id).await }
    });

    let period = RwSignal::new(bc_ipc::Period::Monthly);
    let count = RwSignal::new(bc_ipc::Period::Monthly.default_sparkline_count());

    let sparkline_resource = LocalResource::new(move || {
        let id = sparkline_account_id.clone();
        let p = period.get();
        let n = count.get();
        if let Some(v) = data_version {
            v.get();
        }
        async move { bc_ipc::client::get_account_sparkline(&id, p, n).await }
    });

    let sparkline_currency_code = node.balance.currency_code.clone();

    let balance_str = if node.balance.currency_code.is_empty() {
        "—".into()
    } else {
        let currency =
            bc_ipc::currency_from_code(&node.balance.currency_code).unwrap_or(&bc_ipc::USD);
        crate::components::num::format_amount(node.balance.minor_units, currency)
    };

    let breadcrumb = if node.parent_id.is_some() {
        let section = match node.account_type {
            AccountType::Asset => "Assets",
            AccountType::Liability => "Liabilities",
            AccountType::Equity => "Equity",
            AccountType::Income => "Income",
            AccountType::Expense => "Expenses",
            _ => "Accounts",
        };
        format!("{section} :: {}", node.name)
    } else {
        node.name.clone()
    };

    let tags: Vec<_> = node.tags.clone();

    let instance = DASHBOARD_INSTANCE.fetch_add(1, Ordering::Relaxed);
    let anchor_name = format!("--bc-dash-actions-{instance}");
    let popover_id = format!("bc-dashboard-actions-{instance}");
    let toggle_style = format!("anchor-name: {anchor_name}");
    let menu_style = format!("position-anchor: {anchor_name}");

    view! {
        <div class=style::dashboard>
            <div class=style::header_row>
                <div class=style::breadcrumb>{breadcrumb}</div>

                <div class=style::actions>
                    <div class=style::actions_inline>
                        <button class=style::action_btn>
                            "reconcile " <kbd class=style::kbd>"r"</kbd>
                        </button>
                        <button class=style::action_btn>
                            "import " <kbd class=style::kbd>"i"</kbd>
                        </button>
                        <button class=format!(
                            "{} {}",
                            style::action_btn,
                            style::action_primary,
                        )>"+ transaction " <kbd class=style::kbd>"↵"</kbd></button>
                    </div>

                    <button
                        class=style::actions_toggle
                        style=toggle_style
                        popovertarget=popover_id.clone()
                    >
                        "actions ▾"
                    </button>
                    <div
                        popover=""
                        id=popover_id.clone()
                        class=style::actions_menu
                        style=menu_style
                    >
                        <button
                            class=style::action_btn
                            popovertarget=popover_id.clone()
                            popovertargetaction="hide"
                        >
                            "reconcile "
                            <kbd class=style::kbd>"r"</kbd>
                        </button>
                        <button
                            class=style::action_btn
                            popovertarget=popover_id.clone()
                            popovertargetaction="hide"
                        >
                            "import "
                            <kbd class=style::kbd>"i"</kbd>
                        </button>
                        <button
                            class=format!("{} {}", style::action_btn, style::action_primary)
                            popovertarget=popover_id.clone()
                            popovertargetaction="hide"
                        >
                            "+ transaction "
                            <kbd class=style::kbd>"↵"</kbd>
                        </button>
                    </div>
                </div>
            </div>

            <div class=style::title_row>
                <div class=style::name_group>
                    <span class=style::acct_name>{node.name}</span>
                    {node.mask.map(|m| view! { <span class=style::mask>"···· "{m}</span> })}
                </div>
                <div class=style::meta_group>
                    <span class=style::reconciled>"• reconciled"</span>
                    {tags.into_iter().map(|t| view! { <TagToken label=t /> }).collect::<Vec<_>>()}
                </div>
            </div>

            <div class=style::balance_row>
                <span class=style::balance>{balance_str}</span>
                <span class=style::balance_meta>"// available"</span>
            </div>

            <div class=style::stat_row>
                <StatCards count=4>
                    {move || {
                        let stats = stats_resource.get().and_then(Result::ok);
                        let (income_str, expense_str) = stats
                            .as_ref()
                            .map_or_else(
                                || ("—".into(), "—".into()),
                                |s| {
                                    let currency = bc_ipc::currency_from_code(
                                            &s.income.currency_code,
                                        )
                                        .unwrap_or(&bc_ipc::USD);
                                    let inc = crate::components::num::format_amount(
                                        s.income.minor_units,
                                        currency,
                                    );
                                    let exp = crate::components::num::format_amount(
                                        s.expenses.minor_units.saturating_neg(),
                                        currency,
                                    );
                                    (inc, exp)
                                },
                            );

                        view! {
                            <StatCard
                                label="income (30d)".into()
                                value=income_str
                                sub="last 30 days"
                                tone=StatTone::Good
                            />
                            <StatCard
                                label="expenses (30d)".into()
                                value=expense_str
                                sub="last 30 days"
                                tone=StatTone::Bad
                            />
                        }
                    }}
                    <StatCard
                        label="uncategorised".into()
                        value="3".into()
                        sub="need envelope"
                        tone=StatTone::Warn
                    />
                    <StatCard
                        label="last import".into()
                        value="2h ago".into()
                        sub="commbank-au.wasm"
                        tone=StatTone::Neutral
                    />
                </StatCards>
            </div>

            <div class=style::sparkline_controls>
                <select
                    class=style::sparkline_select
                    on:change=move |ev| {
                        let new_period = match leptos::prelude::event_target_value(&ev).as_str() {
                            "daily" => bc_ipc::Period::Daily,
                            "weekly" => bc_ipc::Period::Weekly,
                            "fortnightly" => bc_ipc::Period::Fortnightly,
                            "quarterly" => bc_ipc::Period::Quarterly,
                            "calendar_year" => bc_ipc::Period::CalendarYear,
                            "financial_year" => {
                                bc_ipc::Period::FinancialYear {
                                    start_month: 7,
                                    start_day: 1,
                                }
                            }
                            _ => bc_ipc::Period::Monthly,
                        };
                        count.set(new_period.default_sparkline_count());
                        period.set(new_period);
                    }
                >
                    <option value="daily" selected=move || period.get() == bc_ipc::Period::Daily>
                        "daily"
                    </option>
                    <option value="weekly" selected=move || period.get() == bc_ipc::Period::Weekly>
                        "weekly"
                    </option>
                    <option
                        value="fortnightly"
                        selected=move || period.get() == bc_ipc::Period::Fortnightly
                    >
                        "fortnightly"
                    </option>
                    <option
                        value="monthly"
                        selected=move || period.get() == bc_ipc::Period::Monthly
                    >
                        "monthly"
                    </option>
                    <option
                        value="quarterly"
                        selected=move || period.get() == bc_ipc::Period::Quarterly
                    >
                        "quarterly"
                    </option>
                    <option
                        value="calendar_year"
                        selected=move || period.get() == bc_ipc::Period::CalendarYear
                    >
                        "calendar year"
                    </option>
                    <option
                        value="financial_year"
                        selected=move || {
                            matches!(period.get(), bc_ipc::Period::FinancialYear { .. })
                        }
                    >
                        "financial year"
                    </option>
                </select>

                <select
                    class=style::sparkline_select
                    on:change=move |ev| {
                        if let Ok(n) = leptos::prelude::event_target_value(&ev).parse::<u32>() {
                            count.set(n);
                        }
                    }
                >
                    {[4_u32, 6, 8, 12, 24]
                        .map(|n| {
                            view! {
                                <option value=n.to_string() selected=move || count.get() == n>
                                    {n.to_string()}
                                </option>
                            }
                        })}
                </select>
            </div>

            {move || {
                let p = period.get();
                let n = count.get();
                let points = match sparkline_resource.get() {
                    Some(Ok(pts)) => pts,
                    Some(Err(ref e)) => {
                        leptos::logging::warn!("sparkline fetch failed: {e:?}");
                        vec![]
                    }
                    None => vec![],
                };
                let currency = bc_ipc::currency_from_code(&sparkline_currency_code)
                    .unwrap_or(&bc_ipc::USD);
                let title = match p {
                    bc_ipc::Period::Daily => format!("Cash Flow (Last {n} Days)"),
                    bc_ipc::Period::Weekly => format!("Cash Flow (Last {n} Weeks)"),
                    bc_ipc::Period::Fortnightly => format!("Cash Flow (Last {n} Fortnights)"),
                    bc_ipc::Period::Quarterly => format!("Cash Flow (Last {n} Quarters)"),
                    bc_ipc::Period::CalendarYear | bc_ipc::Period::FinancialYear { .. } => {
                        format!("Cash Flow (Last {n} Years)")
                    }
                    _ => format!("Cash Flow (Last {n} Months)"),
                };
                view! {
                    <Sparkline points=points currency=currency>
                        <Title slot>{title}</Title>
                    </Sparkline>
                }
            }}
        </div>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
