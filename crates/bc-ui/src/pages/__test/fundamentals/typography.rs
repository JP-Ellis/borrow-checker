//! Route entry for `/__test/fundamentals/typography`.

use leptos::prelude::*;

/// Display name shown in the QA index.
pub const TITLE: &str = "Typography";
/// Route path.
pub const PATH: &str = "/__test/fundamentals/typography";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Type scale, font families, document flow, and mono specimens.";

/// Section label style.
const LABEL: &str = "font-size:11px;color:var(--bc-ink-mute);margin-bottom:16px;";

/// Type scale rows: (token, clamp range, role label).
const SCALE_ROWS: &[(&str, &str, &str)] = &[
    ("--bc-text-page-title", "24–36px", "page title"),
    ("--bc-text-section", "16–22px", "section heading"),
    ("--bc-text-label", "12–14px", "label / ui text"),
    ("--bc-text-body", "12–14px", "body copy"),
    ("--bc-text-data", "10.5–12px", "data / tabular"),
    ("--bc-text-caption", "9.5–11px", "caption / footnote"),
    ("--bc-text-eyebrow", "9–10.5px", "eyebrow / tag"),
];

/// Fira Code weight specimens: (weight-value, weight-name, specimen).
const MONO_WEIGHTS: &[(u32, &str, &str)] = &[
    (
        400,
        "regular",
        "fn cashflow(acct: &Account) → Result<Delta>",
    ),
    (500, "medium", "$487,320.42  +$2,253.00  −$1,284.55"),
    (600, "semibold", "CLEARED  PENDING  ERROR  0123456789"),
];

/// Inter Tight weight specimens: (weight-value, weight-name, specimen).
const SANS_WEIGHTS: &[(u32, &str, &str)] = &[
    (
        400,
        "regular",
        "Smart Access · savings buffer for irregular income",
    ),
    (
        500,
        "medium",
        "Reconciled · imported 2 minutes ago · 14 matches",
    ),
    (
        600,
        "semibold",
        "Smart Access  Everyday  Joint  Savings:Japan 2026",
    ),
    (700, "bold", "Net Worth  +$47,234  −$1,284  Allocated"),
];

/// One row in the type scale reference table.
#[component]
fn ScaleRow(
    /// CSS token, e.g. `"--bc-text-section"`.
    token: &'static str,
    /// Human-readable clamp range, e.g. `"16–22px"`.
    range: &'static str,
    /// Role label, e.g. `"section heading"`.
    role: &'static str,
) -> impl IntoView {
    view! {
        <div style="display:grid;grid-template-columns:160px 80px 1fr 1fr;\
        align-items:baseline;gap:16px;padding:10px 16px;\
        border-bottom:1px solid var(--bc-border)">
            <span style="font-family:var(--bc-font-mono);font-size:10.5px;\
            color:var(--bc-ink-mute)">{token}</span>
            <span style="font-family:var(--bc-font-mono);font-size:10px;\
            color:var(--bc-ink-dim)">{range}</span>
            <span style=format!(
                "font-family:var(--bc-font-sans);font-size:var({token});\
                color:var(--bc-ink);line-height:1.15",
            )>"Smart Access"</span>
            <span style=format!(
                "font-family:var(--bc-font-mono);font-size:var({token});\
                color:var(--bc-ink);line-height:1.15;font-variant-numeric:tabular-nums",
            )>"$487,320.42"</span>
        </div>
        <div style="padding:2px 16px 8px;border-bottom:1px solid var(--bc-border)">
            <span style="font-family:var(--bc-font-mono);font-size:9.5px;\
            color:var(--bc-ink-dim)">{role}</span>
        </div>
    }
}

/// Renders the typography fundamentals reference.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "typography reference catalogues all token groups, weight specimens, and flow patterns in one page"
)]
pub fn TypographyFundamentals() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:48px;padding:24px;max-width:960px">

            <section>
                <p style=LABEL>"Type scale — fluid ramp (sans body · mono data)"</p>
                <div style="border:1px solid var(--bc-border);border-radius:4px;\
                overflow:hidden;background:var(--bc-bg)">

                    <div style="display:grid;grid-template-columns:160px 80px 1fr 1fr;\
                    gap:16px;padding:8px 16px;background:var(--bc-surface);\
                    border-bottom:1px solid var(--bc-border)">
                        <span style="font-family:var(--bc-font-mono);font-size:9.5px;\
                        color:var(--bc-ink-dim);letter-spacing:0.06em;text-transform:uppercase">
                            "token"
                        </span>
                        <span style="font-family:var(--bc-font-mono);font-size:9.5px;\
                        color:var(--bc-ink-dim);letter-spacing:0.06em;text-transform:uppercase">
                            "range"
                        </span>
                        <span style="font-family:var(--bc-font-mono);font-size:9.5px;\
                        color:var(--bc-ink-dim);letter-spacing:0.06em;text-transform:uppercase">
                            "sans"
                        </span>
                        <span style="font-family:var(--bc-font-mono);font-size:9.5px;\
                        color:var(--bc-ink-dim);letter-spacing:0.06em;text-transform:uppercase">
                            "mono"
                        </span>
                    </div>
                    {SCALE_ROWS
                        .iter()
                        .map(|(token, range, role)| {
                            view! { <ScaleRow token=token range=range role=role /> }
                        })
                        .collect::<Vec<_>>()}
                </div>
            </section>

            <section>
                <p style=LABEL>"Font families — weight specimens"</p>
                <div style="display:grid;grid-template-columns:1fr 1fr;gap:12px">

                    <div style="border:1px solid var(--bc-border);border-radius:4px;overflow:hidden">
                        <div style="padding:10px 14px;background:var(--bc-surface);\
                        border-bottom:1px solid var(--bc-border)">
                            <span style="font-family:var(--bc-font-mono);font-size:10.5px;\
                            color:var(--bc-ink-mute)">"Fira Code — bc-font-mono"</span>
                        </div>
                        {MONO_WEIGHTS
                            .iter()
                            .map(|(weight, name, specimen)| {
                                view! {
                                    <div style="padding:12px 14px;border-bottom:1px solid var(--bc-border)">
                                        <div style="font-family:var(--bc-font-mono);font-size:10px;\
                                        color:var(--bc-ink-dim);margin-bottom:4px">
                                            {format!("{weight} {name}")}
                                        </div>
                                        <div style=format!(
                                            "font-family:var(--bc-font-mono);font-size:13.5px;\
                                        color:var(--bc-ink);font-weight:{weight};\
                                        font-variant-numeric:tabular-nums",
                                        )>{*specimen}</div>
                                    </div>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </div>

                    <div style="border:1px solid var(--bc-border);border-radius:4px;overflow:hidden">
                        <div style="padding:10px 14px;background:var(--bc-surface);\
                        border-bottom:1px solid var(--bc-border)">
                            <span style="font-family:var(--bc-font-mono);font-size:10.5px;\
                            color:var(--bc-ink-mute)">"Inter Tight — bc-font-sans"</span>
                        </div>
                        {SANS_WEIGHTS
                            .iter()
                            .map(|(weight, name, specimen)| {
                                view! {
                                    <div style="padding:12px 14px;border-bottom:1px solid var(--bc-border)">
                                        <div style="font-family:var(--bc-font-mono);font-size:10px;\
                                        color:var(--bc-ink-dim);margin-bottom:4px">
                                            {format!("{weight} {name}")}
                                        </div>
                                        <div style=format!(
                                            "font-family:var(--bc-font-sans);font-size:14px;\
                                        color:var(--bc-ink);font-weight:{weight}",
                                        )>{*specimen}</div>
                                    </div>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </div>

                </div>
            </section>

            <section>
                <p style=LABEL>"Document flow — heading and body rhythm"</p>
                <div style="display:flex;flex-direction:column;gap:12px">

                    <div style="border:1px solid var(--bc-border);border-radius:4px;\
                    padding:20px 24px;background:var(--bc-bg)">
                        <div style="font-family:var(--bc-font-mono);font-size:9px;\
                        color:var(--bc-ink-dim);margin-bottom:12px;\
                        text-transform:uppercase;letter-spacing:0.08em">"Full hierarchy"</div>
                        <h1 style="font-family:var(--bc-font-sans);\
                        font-size:var(--bc-text-page-title);\
                        font-weight:700;color:var(--bc-ink);margin:0 0 6px">"Accounts Overview"</h1>
                        <h2 style="font-family:var(--bc-font-sans);\
                        font-size:var(--bc-text-section);\
                        font-weight:600;color:var(--bc-ink);margin:0 0 8px">
                            "Transaction History"
                        </h2>
                        <p style="font-family:var(--bc-font-sans);\
                        font-size:var(--bc-text-label);\
                        font-weight:500;color:var(--bc-ink-soft);margin:0 0 10px">
                            "Smart Access · BSB 062-000 · ****4821"
                        </p>
                        <p style="font-family:var(--bc-font-sans);\
                        font-size:var(--bc-text-body);\
                        color:var(--bc-ink-soft);line-height:1.6;margin:0 0 8px">
                            "Your spending buffer sits $1,284 below the recommended six-week runway. \
                            The shortfall is driven by three irregular grocery runs and an unallocated \
                            transfer from Joint that has not been matched to a category."
                        </p>
                        <p style="font-family:var(--bc-font-mono);\
                        font-size:var(--bc-text-caption);\
                        color:var(--bc-ink-dim);margin:0">
                            "Last imported 2 minutes ago · 847 transactions · reconciled through 11 May 2026"
                        </p>
                    </div>

                    <div style="border:1px solid var(--bc-border);border-radius:4px;\
                    padding:20px 24px;background:var(--bc-bg)">
                        <div style="font-family:var(--bc-font-mono);font-size:9px;\
                        color:var(--bc-ink-dim);margin-bottom:12px;\
                        text-transform:uppercase;letter-spacing:0.08em">
                            "Section → body → caption"
                        </div>
                        <h2 style="font-family:var(--bc-font-sans);\
                        font-size:var(--bc-text-section);\
                        font-weight:600;color:var(--bc-ink);margin:0 0 8px">
                            "Unallocated This Month"
                        </h2>
                        <p style="font-family:var(--bc-font-sans);\
                        font-size:var(--bc-text-body);\
                        color:var(--bc-ink-soft);line-height:1.6;margin:0 0 6px">
                            "You have $3,420 in income that has not been assigned to a category. \
                            Borrow Checker holds it in a holding buffer until you decide how to allocate it."
                        </p>
                        <p style="font-family:var(--bc-font-mono);\
                        font-size:var(--bc-text-caption);\
                        color:var(--bc-ink-dim);margin:0 0 20px">
                            "3 transactions contributing · oldest 4 May 2026"
                        </p>
                        <h2 style="font-family:var(--bc-font-sans);\
                        font-size:var(--bc-text-section);\
                        font-weight:600;color:var(--bc-ink);margin:0 0 8px">"Savings Goals"</h2>
                        <p style="font-family:var(--bc-font-sans);\
                        font-size:var(--bc-text-body);\
                        color:var(--bc-ink-soft);line-height:1.6;margin:0 0 6px">
                            "Japan 2026 is on track: you've saved $4,800 of the $6,000 target with \
                            eight weeks remaining. Emergency fund is overfunded by $340."
                        </p>
                        <p style="font-family:var(--bc-font-mono);\
                        font-size:var(--bc-text-caption);\
                        color:var(--bc-ink-dim);margin:0">
                            "2 active goals · updated automatically on import"
                        </p>
                    </div>

                    <div style="border:1px solid var(--bc-border);border-radius:4px;\
                    padding:20px 24px;background:var(--bc-bg)">
                        <div style="font-family:var(--bc-font-mono);font-size:9px;\
                        color:var(--bc-ink-dim);margin-bottom:12px;\
                        text-transform:uppercase;letter-spacing:0.08em">
                            "Consecutive section headings"
                        </div>
                        {["Net Worth", "Cash Flow", "Spending by Category", "Savings Progress"]
                            .iter()
                            .enumerate()
                            .map(|(i, heading)| {
                                view! {
                                    <div style=format!(
                                        "display:flex;align-items:baseline;gap:12px;\
                                    padding:10px 0;{}",
                                        if i > 0 {
                                            "border-top:1px solid var(--bc-border)"
                                        } else {
                                            ""
                                        },
                                    )>
                                        <h2 style="font-family:var(--bc-font-sans);\
                                        font-size:var(--bc-text-section);\
                                        font-weight:600;color:var(--bc-ink);margin:0;flex:1">
                                            {*heading}
                                        </h2>
                                        <span style="font-family:var(--bc-font-mono);\
                                        font-size:var(--bc-text-data);\
                                        color:var(--bc-ink-dim);font-variant-numeric:tabular-nums">
                                            "→"
                                        </span>
                                    </div>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </div>

                    <div style="border:1px solid var(--bc-border);border-radius:4px;\
                    padding:20px 24px;background:var(--bc-bg)">
                        <div style="font-family:var(--bc-font-mono);font-size:9px;\
                        color:var(--bc-ink-dim);margin-bottom:16px;\
                        text-transform:uppercase;letter-spacing:0.08em">
                            "Eyebrow + value — KPI tiles"
                        </div>
                        <div style="display:flex;gap:32px;flex-wrap:wrap">
                            {[
                                ("Net Worth", "+$47,234.18", "--bc-good"),
                                ("Unallocated", "$3,420.00", "--bc-warn"),
                                ("Overspent", "−$284.55", "--bc-bad"),
                                ("Cleared Today", "$1,128.00", "--bc-ink"),
                            ]
                                .iter()
                                .map(|(label, value, color_var)| {
                                    view! {
                                        <div>
                                            <div style="font-family:var(--bc-font-mono);\
                                            font-size:var(--bc-text-eyebrow);\
                                            color:var(--bc-ink-dim);\
                                            text-transform:uppercase;letter-spacing:0.08em;\
                                            margin-bottom:4px">{*label}</div>
                                            <div style=format!(
                                                "font-family:var(--bc-font-mono);\
                                            font-size:var(--bc-text-section);\
                                            font-weight:600;color:var({color_var});\
                                            font-variant-numeric:tabular-nums",
                                            )>{*value}</div>
                                        </div>
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </div>
                    </div>

                </div>
            </section>

            <section>
                <p style=LABEL>"Mono specimens — CLI, data table, config"</p>
                <div style="display:flex;flex-direction:column;gap:12px">

                    <div style="border:1px solid var(--bc-border);border-radius:4px;\
                    overflow:hidden">
                        <div style="padding:8px 14px;background:var(--bc-surface);\
                        border-bottom:1px solid var(--bc-border)">
                            <span style="font-family:var(--bc-font-mono);font-size:10px;\
                            color:var(--bc-ink-mute)">"CLI — command and output"</span>
                        </div>
                        <div style="padding:16px;background:var(--bc-surface-alt);\
                        font-family:var(--bc-font-mono);font-size:12.5px;line-height:1.9">

                            <div>
                                <span style="color:var(--bc-accent)">"› "</span>
                                <span style="color:var(--bc-keyword)">"find "</span>
                                <span style="color:var(--bc-string)">"\"coles\""</span>
                                <span style="color:var(--bc-ink-dim)">" --last "</span>
                                <span style="color:var(--bc-number)">"30d"</span>
                                <span style="color:var(--bc-ink-dim)">" --in "</span>
                                <span style="color:var(--bc-type)">"Groceries"</span>
                                <span style="color:var(--bc-ink-dim)">" --show-uncleared"</span>
                            </div>

                            <div style="color:var(--bc-comment)">
                                "// 14 matches · sum −$1,284.55 · 2 uncleared"
                            </div>

                            {[
                                (
                                    "2026-05-09",
                                    "Coles Eastland",
                                    "−$184.30",
                                    "Groceries:Coles",
                                    "--bc-ink-soft",
                                ),
                                (
                                    "2026-05-04",
                                    "Coles Express",
                                    "−$47.20",
                                    "Groceries:Coles",
                                    "--bc-ink-soft",
                                ),
                                (
                                    "2026-04-28",
                                    "COLES 3148",
                                    "−$212.05",
                                    "Groceries:Coles",
                                    "--bc-ink-dim",
                                ),
                            ]
                                .iter()
                                .map(|(date, payee, amount, cat, ink)| {
                                    view! {
                                        <div style="display:grid;grid-template-columns:100px 1fr 90px 1fr;\
                                        gap:12px">
                                            <span style=format!(
                                                "color:var({ink});font-variant-numeric:tabular-nums",
                                            )>{*date}</span>
                                            <span style=format!("color:var({ink})")>{*payee}</span>
                                            <span style=format!(
                                                "color:var(--bc-bad);text-align:right;font-variant-numeric:tabular-nums",
                                            )>{*amount}</span>
                                            <span style="color:var(--bc-type)">{*cat}</span>
                                        </div>
                                    }
                                })
                                .collect::<Vec<_>>()}

                            <div style="margin-top:6px">
                                <span style="color:var(--bc-fn)">"autocat"</span>
                                <span style="color:var(--bc-ink-dim)">"(tx_4421) → "</span>
                                <span style="color:var(--bc-type)">"Groceries:Coles"</span>
                                <span style="color:var(--bc-comment)">" // confidence 0.94"</span>
                            </div>
                            <div>
                                <span style="color:var(--bc-keyword)">"budget"</span>
                                <span style="color:var(--bc-fn)">" allocate"</span>
                                <span style="color:var(--bc-string)">
                                    " \"Savings:Japan 2026\""
                                </span>
                                <span style="color:var(--bc-ink-dim)">" --amount "</span>
                                <span style="color:var(--bc-number)">"500"</span>
                            </div>
                        </div>
                    </div>

                    <div style="border:1px solid var(--bc-border);border-radius:4px;\
                    overflow:hidden">
                        <div style="padding:8px 14px;background:var(--bc-surface);\
                        border-bottom:1px solid var(--bc-border)">
                            <span style="font-family:var(--bc-font-mono);font-size:10px;\
                            color:var(--bc-ink-mute)">"Data table — tabular-nums alignment"</span>
                        </div>
                        <div style="padding:0;font-family:var(--bc-font-mono);\
                        font-size:12px;font-variant-numeric:tabular-nums">

                            <div style="display:grid;grid-template-columns:1fr 110px 110px 110px;\
                            padding:8px 16px;background:var(--bc-surface);\
                            border-bottom:1px solid var(--bc-border);\
                            font-size:9.5px;color:var(--bc-ink-dim);\
                            text-transform:uppercase;letter-spacing:0.06em">
                                <span>"Account"</span>
                                <span style="text-align:right">"Balance"</span>
                                <span style="text-align:right">"Allocated"</span>
                                <span style="text-align:right">"Available"</span>
                            </div>
                            {[
                                (
                                    "Smart Access",
                                    "$12,847.33",
                                    "$9,200.00",
                                    "$3,647.33",
                                    "--bc-good",
                                ),
                                ("Everyday", "$2,430.18", "$2,100.00", "$330.18", "--bc-good"),
                                ("Joint", "$5,021.44", "$5,800.00", "−$778.56", "--bc-bad"),
                                (
                                    "Savings · Japan 2026",
                                    "$4,800.00",
                                    "$4,800.00",
                                    "$0.00",
                                    "--bc-ink-soft",
                                ),
                            ]
                                .iter()
                                .enumerate()
                                .map(|(i, (name, balance, alloc, avail, avail_color))| {
                                    view! {
                                        <div style=format!(
                                            "display:grid;grid-template-columns:1fr 110px 110px 110px;\
                                        padding:9px 16px;color:var(--bc-ink-soft);{}",
                                            if i > 0 {
                                                "border-top:1px solid var(--bc-border)"
                                            } else {
                                                ""
                                            },
                                        )>
                                            <span style="color:var(--bc-ink)">{*name}</span>
                                            <span style="text-align:right">{*balance}</span>
                                            <span style="text-align:right">{*alloc}</span>
                                            <span style=format!(
                                                "text-align:right;color:var({avail_color})",
                                            )>{*avail}</span>
                                        </div>
                                    }
                                })
                                .collect::<Vec<_>>()}

                            <div style="display:grid;grid-template-columns:1fr 110px 110px 110px;\
                            padding:9px 16px;border-top:2px solid var(--bc-border-strong);\
                            font-weight:600;color:var(--bc-ink)">
                                <span>"Total"</span>
                                <span style="text-align:right">"$25,098.95"</span>
                                <span style="text-align:right">"$21,900.00"</span>
                                <span style="text-align:right;color:var(--bc-good)">
                                    "$3,198.95"
                                </span>
                            </div>
                        </div>
                    </div>

                    <div style="border:1px solid var(--bc-border);border-radius:4px;\
                    overflow:hidden">
                        <div style="padding:8px 14px;background:var(--bc-surface);\
                        border-bottom:1px solid var(--bc-border)">
                            <span style="font-family:var(--bc-font-mono);font-size:10px;\
                            color:var(--bc-ink-mute)">"TOML config — syntax specimen"</span>
                        </div>
                        <div style="padding:16px;background:var(--bc-surface-alt);\
                        font-family:var(--bc-font-mono);font-size:12.5px;line-height:1.9">
                            <div style="color:var(--bc-comment)">
                                "# borrow-checker config · v0.4"
                            </div>
                            <div style="margin-top:4px">
                                <span style="color:var(--bc-keyword)">
                                    "[accounts.smart_access]"
                                </span>
                            </div>
                            <div style="padding-left:16px">
                                <span style="color:var(--bc-fn)">"name"</span>
                                <span style="color:var(--bc-ink-dim)">"         = "</span>
                                <span style="color:var(--bc-string)">"\"Smart Access\""</span>
                            </div>
                            <div style="padding-left:16px">
                                <span style="color:var(--bc-fn)">"institution"</span>
                                <span style="color:var(--bc-ink-dim)">"    = "</span>
                                <span style="color:var(--bc-string)">"\"Example Bank\""</span>
                            </div>
                            <div style="padding-left:16px">
                                <span style="color:var(--bc-fn)">"currency"</span>
                                <span style="color:var(--bc-ink-dim)">"       = "</span>
                                <span style="color:var(--bc-string)">"\"AUD\""</span>
                            </div>
                            <div style="padding-left:16px">
                                <span style="color:var(--bc-fn)">"opening_balance"</span>
                                <span style="color:var(--bc-ink-dim)">" = "</span>
                                <span style="color:var(--bc-number)">"10_000.00"</span>
                            </div>
                            <div style="margin-top:4px">
                                <span style="color:var(--bc-keyword)">"[budget.categories]"</span>
                            </div>
                            <div style="padding-left:16px">
                                <span style="color:var(--bc-fn)">"\"Savings:Japan 2026\""</span>
                                <span style="color:var(--bc-ink-dim)">" = "</span>
                                <span style="color:var(--bc-type)">"{ target = "</span>
                                <span style="color:var(--bc-number)">"6_000.00"</span>
                                ", due = "
                                <span style="color:var(--bc-string)">"\"2026-10-01\""</span>
                                <span style="color:var(--bc-type)">"}"</span>
                            </div>
                            <div style="padding-left:16px">
                                <span style="color:var(--bc-fn)">"\"Groceries\""</span>
                                <span style="color:var(--bc-ink-dim)">"             = "</span>
                                <span style="color:var(--bc-type)">"{ monthly = "</span>
                                <span style="color:var(--bc-number)">"1_400.00"</span>
                                <span style="color:var(--bc-type)">"}"</span>
                            </div>
                            <div style="margin-top:4px;color:var(--bc-comment)">
                                "# autocat rules are in ~/.config/borrow-checker/autocat.toml"
                            </div>
                        </div>
                    </div>

                </div>
            </section>

        </div>
    }
}
