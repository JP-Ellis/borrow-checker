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
/// * `on_add_tx` - Optional callback fired when the user clicks "+ transaction".
/// * `period_window` - Page-level period granularity (read-only; the register's `PeriodNav` writes it).
/// * `window_start` - Page-level display-window start (read-only).
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
    /// Optional callback fired when the user clicks the "+ transaction" action button.
    #[prop(optional)]
    on_add_tx: Option<Callback<()>>,
    /// Page-level period granularity (read-only; the register's `PeriodNav` writes it).
    period_window: Signal<bc_ipc::Period>,
    /// Page-level display-window start (read-only).
    window_start: Signal<jiff::civil::Date>,
) -> impl IntoView {
    let currencies = crate::currency_ctx::use_currency_store();
    let account_id = node.id.clone();
    let sparkline_account_id = node.id.clone();

    let stats_resource = LocalResource::new(move || {
        let id = account_id.clone();
        let start = window_start.get();
        let until = crate::components::period_nav::period_end(&period_window.get(), start);
        if let Some(v) = data_version {
            v.get();
        }
        async move { bc_ipc::client::get_account_stats(&id, start, until).await }
    });

    // Sparkline granularity/count are independent of the page-level
    // `period_window` (see #255); named distinctly to avoid confusion.
    let sparkline_period = RwSignal::new(bc_ipc::Period::Monthly);
    let count = RwSignal::new(bc_ipc::Period::Monthly.default_sparkline_count());

    let sparkline_resource = LocalResource::new(move || {
        let id = sparkline_account_id.clone();
        let p = sparkline_period.get();
        let n = count.get();
        if let Some(v) = data_version {
            v.get();
        }
        async move { bc_ipc::client::get_account_sparkline(&id, p, n).await }
    });

    let sparkline_currency_code = node
        .balance
        .as_ref()
        .map_or_else(String::new, |b| b.currency_code.clone());

    // Closing / opening / net (formatted) plus a net-is-negative flag, computed
    // once per dependency change. A `Memo` avoids re-reading the resource and
    // re-formatting three amounts in each of the four view closures below.
    let balance_line = Memo::new(move |_| {
        let stats = stats_resource.get().and_then(Result::ok);
        let cur = currencies.get();
        let fmt = |a: &bc_ipc::Amount| {
            let meta = crate::components::num::meta::display_meta_for(&a.currency_code, &cur);
            crate::components::num::format_amount(&a.value, &meta)
        };
        match stats {
            None => (
                "\u{2014}".to_owned(),
                "\u{2014}".to_owned(),
                "\u{2014}".to_owned(),
                false,
            ),
            Some(s) => {
                let net_neg = s.net.value < rust_decimal::Decimal::ZERO;
                (
                    fmt(&s.closing_balance),
                    fmt(&s.opening_balance),
                    fmt(&s.net),
                    net_neg,
                )
            }
        }
    });

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

    // Single handler for both the inline action-bar button and the popover menu
    // button, avoiding duplicate closure definitions.
    let fire_add_tx = move |_: leptos::ev::MouseEvent| {
        if let Some(cb) = on_add_tx {
            cb.run(());
        }
    };

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
                        <button
                            class=format!("{} {}", style::action_btn, style::action_primary)
                            on:click=fire_add_tx
                        >
                            "+ transaction "
                            <kbd class=style::kbd>"↵"</kbd>
                        </button>
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
                            on:click=fire_add_tx
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
                <span class=style::balance>{move || balance_line.with(|b| b.0.clone())}</span>
                <span class=style::balance_meta>"// closing"</span>
            </div>
            <div class=style::balance_sub>
                <span class=style::opening>
                    "opening " {move || balance_line.with(|b| b.1.clone())}
                </span>
                <span class=move || {
                    if balance_line.with(|b| b.3) { style::net_bad } else { style::net_good }
                }>"net " {move || balance_line.with(|b| b.2.clone())}</span>
            </div>

            <div class=style::stat_row>
                <StatCards count=4>
                    {move || {
                        let stats = stats_resource.get().and_then(Result::ok);
                        let window_label = crate::components::period_nav::window_label(
                            &period_window.get(),
                            window_start.get(),
                        );
                        let (income_str, expense_str, tx_count_str) = stats
                            .as_ref()
                            .map_or_else(
                                || ("—".into(), "—".into(), "—".into()),
                                |s| {
                                    let meta = crate::components::num::meta::display_meta_for(
                                        &s.income.currency_code,
                                        &currencies.get(),
                                    );
                                    let inc = crate::components::num::format_amount(
                                        &s.income.value,
                                        &meta,
                                    );
                                    let exp = crate::components::num::format_amount(
                                        &s.expenses.value,
                                        &meta,
                                    );
                                    (inc, exp, s.tx_count.to_string())
                                },
                            );

                        view! {
                            <StatCard
                                label="income".into()
                                value=income_str
                                sub=window_label.clone()
                                tone=StatTone::Good
                            />
                            <StatCard
                                label="expenses".into()
                                value=expense_str
                                sub=window_label.clone()
                                tone=StatTone::Bad
                            />
                            <StatCard
                                label="transactions".into()
                                value=tx_count_str
                                sub=window_label
                                tone=StatTone::Neutral
                            />
                        }
                    }}
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
                            "financial_quarter" => {
                                bc_ipc::Period::FinancialQuarter {
                                    start_month: 7,
                                    start_day: 1,
                                }
                            }
                            _ => bc_ipc::Period::Monthly,
                        };
                        count.set(new_period.default_sparkline_count());
                        sparkline_period.set(new_period);
                    }
                >
                    <option
                        value="daily"
                        selected=move || sparkline_period.get() == bc_ipc::Period::Daily
                    >
                        "daily"
                    </option>
                    <option
                        value="weekly"
                        selected=move || sparkline_period.get() == bc_ipc::Period::Weekly
                    >
                        "weekly"
                    </option>
                    <option
                        value="fortnightly"
                        selected=move || sparkline_period.get() == bc_ipc::Period::Fortnightly
                    >
                        "fortnightly"
                    </option>
                    <option
                        value="monthly"
                        selected=move || sparkline_period.get() == bc_ipc::Period::Monthly
                    >
                        "monthly"
                    </option>
                    <option
                        value="quarterly"
                        selected=move || sparkline_period.get() == bc_ipc::Period::Quarterly
                    >
                        "quarterly"
                    </option>
                    <option
                        value="calendar_year"
                        selected=move || sparkline_period.get() == bc_ipc::Period::CalendarYear
                    >
                        "calendar year"
                    </option>
                    <option
                        value="financial_year"
                        selected=move || {
                            matches!(sparkline_period.get(), bc_ipc::Period::FinancialYear { .. })
                        }
                    >
                        "financial year"
                    </option>
                    <option
                        value="financial_quarter"
                        selected=move || {
                            matches!(
                                sparkline_period.get(),
                                bc_ipc::Period::FinancialQuarter { .. }
                            )
                        }
                    >
                        "financial quarter"
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
                let p = sparkline_period.get();
                let n = count.get();
                let points = match sparkline_resource.get() {
                    Some(Ok(pts)) => pts,
                    Some(Err(ref e)) => {
                        leptos::logging::warn!("sparkline fetch failed: {e:?}");
                        vec![]
                    }
                    None => vec![],
                };
                let title = match p {
                    bc_ipc::Period::Daily => format!("Cash Flow (Last {n} Days)"),
                    bc_ipc::Period::Weekly => format!("Cash Flow (Last {n} Weeks)"),
                    bc_ipc::Period::Fortnightly => format!("Cash Flow (Last {n} Fortnights)"),
                    bc_ipc::Period::Quarterly | bc_ipc::Period::FinancialQuarter { .. } => {
                        format!("Cash Flow (Last {n} Quarters)")
                    }
                    bc_ipc::Period::CalendarYear | bc_ipc::Period::FinancialYear { .. } => {
                        format!("Cash Flow (Last {n} Years)")
                    }
                    _ => format!("Cash Flow (Last {n} Months)"),
                };
                view! {
                    <Sparkline points=points currency_code=sparkline_currency_code.clone()>
                        <Title slot>{title}</Title>
                    </Sparkline>
                }
            }}
        </div>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
