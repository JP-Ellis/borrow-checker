//! QA page for [`super::Sparkline`].

use bc_ipc::Amount;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::SparkPoint;
use super::Sparkline;
use super::Title;

/// Constructs a [`SparkPoint`] from a static string label and cent values.
fn pt(label: &'static str, income: i64, expenses: i64) -> SparkPoint {
    SparkPoint::new(
        label,
        Amount::new(Decimal::new(income, 2), "AUD"),
        Amount::new(Decimal::new(expenses, 2), "AUD"),
    )
}

/// Renders [`Sparkline`] in several states for visual inspection.
#[component]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division_remainder_used,
    clippy::modulo_arithmetic,
    reason = "pseudo-random QA data generation; arithmetic safe for small i64 day-index values"
)]
pub fn SparklineQa() -> impl IntoView {
    // 30 daily points: weekends (d%7 < 2) have no income; expenses occur every day.
    let dense_pts: Vec<SparkPoint> = (1_i64..=30)
        .map(|d| {
            SparkPoint::new(
                format!("{d:02}"),
                Amount::new(
                    Decimal::new(
                        if d % 7 < 2 {
                            0
                        } else {
                            11_000 + d * 317 % 4_000
                        },
                        2,
                    ),
                    "AUD",
                ),
                Amount::new(Decimal::new(8_000 + d * 53 % 3_000, 2), "AUD"),
            )
        })
        .collect();
    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "typical — 6 months of income vs expenses"
                </p>
                <Sparkline points=vec![
                    pt("nov", 900_000, 680_000),
                    pt("dec", 915_000, 710_000),
                    pt("jan", 905_000, 695_000),
                    pt("feb", 910_000, 700_000),
                    pt("mar", 908_000, 688_000),
                    pt("apr", 910_000, 690_000),
                ]>
                    <Title slot>"Cash Flow (Last 6 Months)"</Title>
                </Sparkline>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "single point — degenerate case"
                </p>
                <Sparkline points=vec![pt("apr", 500_000, 500_000)]>
                    <Title slot>"Cash Flow (April)"</Title>
                </Sparkline>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "empty — no data points"
                </p>
                <Sparkline points=vec![]>
                    <Title slot>"Cash Flow (Last 6 Months)"</Title>
                </Sparkline>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "flat — all equal values (midpoint rendering)"
                </p>
                <Sparkline points=vec![
                    pt("jan", 800_000, 800_000),
                    pt("feb", 800_000, 800_000),
                    pt("mar", 800_000, 800_000),
                ]>
                    <Title slot>"Cash Flow (Q1)"</Title>
                </Sparkline>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "dense — 30 daily points, weekends have no income"
                </p>
                <Sparkline points=dense_pts>
                    <Title slot>"Daily Flow (August)"</Title>
                </Sparkline>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "show_fill=false — lines only, no fill"
                </p>
                <Sparkline
                    points=vec![
                        pt("nov", 900_000, 680_000),
                        pt("dec", 915_000, 710_000),
                        pt("jan", 905_000, 695_000),
                        pt("feb", 910_000, 700_000),
                        pt("mar", 908_000, 688_000),
                        pt("apr", 910_000, 690_000),
                    ]
                    show_fill=false
                >
                    <Title slot>"Cash Flow (Last 6 Months)"</Title>
                </Sparkline>
            </section>

        </div>
    }
}
