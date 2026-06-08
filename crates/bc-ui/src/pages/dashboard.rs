//! Dashboard — net worth headline, account type summary, and account list.

use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use leptos::prelude::*;
use leptos_router::components::A;
use stylance::import_style;

use crate::components::num::format_amount;
use crate::components::stat_card::StatCard;
use crate::components::stat_card::StatCards;
use crate::components::stat_card::StatTone;

import_style!(style, "dashboard.module.scss");

// MARK: Net worth helpers

/// Summary of one account type: count and total balance in minor units.
struct TypeSummary {
    /// Number of accounts of this type.
    count: usize,
    /// Total balance in minor units (all accounts must share the same currency).
    total: i64,
    /// Currency code for the total; empty string means no accounts yet.
    currency_code: String,
    /// Whether the accounts in this group have mixed currencies.
    multi_currency: bool,
}

impl TypeSummary {
    /// Creates an empty [`TypeSummary`].
    fn empty() -> Self {
        Self {
            count: 0,
            total: 0,
            currency_code: String::new(),
            multi_currency: false,
        }
    }

    /// Accumulates one account's balance into this summary.
    ///
    /// If the account's currency differs from the first account seen, the
    /// summary is flagged as multi-currency and further summation stops.
    fn add(&mut self, node: &AccountNode) {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "count += 1 on a usize that will never reach usize::MAX in practice"
        )]
        {
            self.count += 1;
        }
        if self.currency_code.is_empty() {
            self.currency_code.clone_from(&node.balance.currency_code);
        } else if self.currency_code != node.balance.currency_code {
            self.multi_currency = true;
        }
        if !self.multi_currency {
            self.total = self.total.saturating_add(node.balance.minor_units);
        }
    }

    /// Returns the formatted total string, e.g. `"+$1,234.56"` or `"multi-currency"`.
    fn formatted(&self) -> String {
        if self.count == 0 {
            return "—".into();
        }
        if self.multi_currency {
            return "multi-currency".into();
        }
        let currency = bc_ipc::currency_from_code(&self.currency_code).unwrap_or(&bc_ipc::USD);
        format_amount(self.total, currency)
    }

    /// Returns a sub-line string like `"3 accounts"` or `"1 account"`.
    fn sub_line(&self) -> String {
        if self.count == 1 {
            "1 account".into()
        } else {
            format!("{} accounts", self.count)
        }
    }
}

/// Aggregated data computed from a flat account list.
struct AccountSummary {
    /// Net worth string: `"multi-currency"`, `"—"`, or a formatted amount.
    net_worth: String,
    /// Assets subtotal string.
    assets_total: String,
    /// Liabilities subtotal string.
    liabilities_total: String,
    /// Per-type data for stat cards, in display order.
    stat_rows: Vec<StatRow>,
    /// Accounts grouped by type, in display order.
    account_groups: Vec<(&'static str, Vec<AccountNode>)>,
}

/// Data for a single stat card.
struct StatRow {
    /// Eyebrow label.
    label: &'static str,
    /// Formatted total value.
    value: String,
    /// Sub-line text.
    sub: String,
    /// Colour tone.
    tone: StatTone,
}

/// Computes an [`AccountSummary`] from a flat list of account nodes.
#[expect(
    clippy::too_many_lines,
    reason = "routine grouping logic; no meaningful way to split without losing clarity"
)]
fn compute_summary(accounts: Vec<AccountNode>) -> AccountSummary {
    let mut assets = TypeSummary::empty();
    let mut liabilities = TypeSummary::empty();
    let mut equity = TypeSummary::empty();
    let mut income = TypeSummary::empty();
    let mut expense = TypeSummary::empty();

    let mut asset_accts: Vec<AccountNode> = vec![];
    let mut liability_accts: Vec<AccountNode> = vec![];
    let mut income_accts: Vec<AccountNode> = vec![];
    let mut expense_accts: Vec<AccountNode> = vec![];
    let mut equity_accts: Vec<AccountNode> = vec![];

    for node in accounts {
        match node.account_type {
            AccountType::Asset => {
                assets.add(&node);
                asset_accts.push(node);
            }
            AccountType::Liability => {
                liabilities.add(&node);
                liability_accts.push(node);
            }
            AccountType::Equity => {
                equity.add(&node);
                equity_accts.push(node);
            }
            AccountType::Income => {
                income.add(&node);
                income_accts.push(node);
            }
            AccountType::Expense => {
                expense.add(&node);
                expense_accts.push(node);
            }
            /* AccountType is #[non_exhaustive]; all current variants are handled above */
            _ => {}
        }
    }

    /* Net worth = assets − liabilities (single currency only) */
    let net_worth = {
        let multi = assets.multi_currency || liabilities.multi_currency;
        let both_empty = assets.currency_code.is_empty() && liabilities.currency_code.is_empty();
        let currencies_match = liabilities.currency_code.is_empty()
            || assets.currency_code.is_empty()
            || assets.currency_code == liabilities.currency_code;

        if multi || (!both_empty && !currencies_match) {
            "multi-currency".into()
        } else if both_empty {
            "—".into()
        } else {
            let code = if assets.currency_code.is_empty() {
                &liabilities.currency_code
            } else {
                &assets.currency_code
            };
            let net = assets.total.saturating_sub(liabilities.total);
            let currency = bc_ipc::currency_from_code(code).unwrap_or(&bc_ipc::USD);
            format_amount(net, currency)
        }
    };

    let assets_total = assets.formatted();
    let liabilities_total = liabilities.formatted();

    let stat_rows = vec![
        StatRow {
            label: "assets",
            sub: assets.sub_line(),
            value: assets.formatted(),
            tone: StatTone::Good,
        },
        StatRow {
            label: "liabilities",
            sub: liabilities.sub_line(),
            value: liabilities.formatted(),
            tone: StatTone::Bad,
        },
        StatRow {
            label: "income",
            sub: income.sub_line(),
            value: income.formatted(),
            tone: StatTone::Good,
        },
        StatRow {
            label: "expenses",
            sub: expense.sub_line(),
            value: expense.formatted(),
            tone: StatTone::Bad,
        },
        StatRow {
            label: "equity",
            sub: equity.sub_line(),
            value: equity.formatted(),
            tone: StatTone::Neutral,
        },
    ];

    let account_groups = vec![
        ("Assets", asset_accts),
        ("Liabilities", liability_accts),
        ("Income", income_accts),
        ("Expenses", expense_accts),
        ("Equity", equity_accts),
    ];

    AccountSummary {
        net_worth,
        assets_total,
        liabilities_total,
        stat_rows,
        account_groups,
    }
}

/// Returns a loading-state [`AccountSummary`] with placeholder strings.
fn loading_summary() -> AccountSummary {
    let stat_rows = vec![
        StatRow {
            label: "assets",
            value: "—".into(),
            sub: String::new(),
            tone: StatTone::Good,
        },
        StatRow {
            label: "liabilities",
            value: "—".into(),
            sub: String::new(),
            tone: StatTone::Bad,
        },
        StatRow {
            label: "income",
            value: "—".into(),
            sub: String::new(),
            tone: StatTone::Good,
        },
        StatRow {
            label: "expenses",
            value: "—".into(),
            sub: String::new(),
            tone: StatTone::Bad,
        },
        StatRow {
            label: "equity",
            value: "—".into(),
            sub: String::new(),
            tone: StatTone::Neutral,
        },
    ];
    AccountSummary {
        net_worth: "—".into(),
        assets_total: "—".into(),
        liabilities_total: "—".into(),
        stat_rows,
        account_groups: vec![],
    }
}

// MARK: Dashboard component

/// Dashboard page — net worth headline, account type summary, and account list.
#[component]
pub fn Dashboard() -> impl IntoView {
    /* LocalResource required: bc_ipc::client futures are not Send (JsFuture) */
    let accounts_resource = LocalResource::new(bc_ipc::client::list_accounts);

    view! {
        <div class=style::page>
            {move || {
                let summary = match accounts_resource.get() {
                    None => loading_summary(),
                    Some(result) => compute_summary(result.unwrap_or_default()),
                };
                let net_worth = summary.net_worth.clone();
                let assets_total = summary.assets_total.clone();
                let liabilities_total = summary.liabilities_total.clone();

                view! {
                    <section class=style::headline_section>
                        <div class=style::headline_label>"net worth"</div>
                        <div class=style::headline>{net_worth}</div>
                        <div class=style::subtotals>
                            <div class=style::subtotal_item>
                                <span class=style::subtotal_label>"assets"</span>
                                <span class=style::subtotal_value>{assets_total}</span>
                            </div>
                            <div class=style::subtotal_item>
                                <span class=style::subtotal_label>"liabilities"</span>
                                <span class=style::subtotal_value>{liabilities_total}</span>
                            </div>
                        </div>
                    </section>

                    <section class=style::stat_section>
                        <StatCards count=5>
                            {summary
                                .stat_rows
                                .into_iter()
                                .map(|row| {
                                    view! {
                                        <StatCard
                                            label=row.label.to_owned()
                                            value=row.value
                                            sub=row.sub
                                            tone=row.tone
                                        />
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </StatCards>
                    </section>

                    <section class=style::account_section>
                        <div class=style::account_section_title>"accounts"</div>
                        {summary
                            .account_groups
                            .into_iter()
                            .filter(|(_, nodes)| !nodes.is_empty())
                            .map(|(group_label, nodes)| {
                                let rows = nodes
                                    .into_iter()
                                    .map(|node| {
                                        let href = format!("/accounts/{}", node.id);
                                        let balance_str = {
                                            let currency = bc_ipc::currency_from_code(
                                                    &node.balance.currency_code,
                                                )
                                                .unwrap_or(&bc_ipc::USD);
                                            format_amount(node.balance.minor_units, currency)
                                        };
                                        view! {
                                            <div class=style::account_row>
                                                <A href=href>
                                                    <span class=style::account_name>{node.name}</span>
                                                </A>
                                                <span class=style::account_balance>{balance_str}</span>
                                            </div>
                                        }
                                    })
                                    .collect::<Vec<_>>();
                                view! {
                                    <div class=style::account_group>
                                        <div class=style::account_section_title>{group_label}</div>
                                        {rows}
                                    </div>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </section>
                }
            }}
        </div>
    }
}
