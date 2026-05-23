//! QA page for [`super::Num`] and [`super::format_amount`].

use bc_ipc::Amount;
use leptos::prelude::*;

use super::Num;

/// Collapsed-border mono table spanning full container width.
const TABLE: &str =
    "border-collapse:collapse;font-family:var(--bc-font-mono);font-size:12px;width:100%";
/// Left-aligned muted table header cell.
const TH: &str = "padding:4px 16px 4px 0;text-align:left;color:var(--bc-ink-mute);";
/// Right-aligned muted table header cell.
const TH_R: &str = "padding:4px 0;text-align:right;color:var(--bc-ink-mute);";
/// Left-aligned muted label cell with right-padding.
const TD_LABEL: &str = "padding:4px 16px 4px 0;color:var(--bc-ink-mute);";
/// Right-aligned numeric data cell.
const TD_NUM: &str = "padding:4px 0;text-align:right;";

/// One table row: label on the left, [`Num`] right-aligned on the right.
#[component]
fn Row(
    /// Row label shown in the left column.
    label: &'static str,
    /// Monetary value to display.
    money: Amount,
) -> impl IntoView {
    view! {
        <tr>
            <td style=TD_LABEL>{label}</td>
            <td style=TD_NUM>
                <Num money=money />
            </td>
        </tr>
    }
}

/// A titled table section.
#[component]
fn Section(
    /// Bold heading shown above the table.
    title: &'static str,
    /// Subtitle shown next to the heading.
    subtitle: &'static str,
    /// Table rows rendered in the `<tbody>`.
    children: Children,
) -> impl IntoView {
    view! {
        <section>
            <p style="font-size:11px;color:var(--bc-ink-mute);margin:0 0 2px">
                <strong>{title}</strong>
                " — "
                {subtitle}
            </p>
            <table style=TABLE>
                <thead>
                    <tr>
                        <th style=TH>"label"</th>
                        <th style=TH_R>"rendered (right-aligned → decimal aligns)"</th>
                    </tr>
                </thead>
                <tbody>{children()}</tbody>
            </table>
        </section>
    }
}

/// Renders [`Num`] across all currencies, sign states, and edge cases.
///
/// Numbers in each table are right-aligned so the decimal point aligns within
/// a column — the recommended table layout for monetary values.
#[component]
pub fn NumQa() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px;max-width:640px">

            <Section title="USD $" subtitle="2 decimals · Western grouping">
                <Row label="positive" money=Amount::new(128_456, "USD") />
                <Row label="zero" money=Amount::new(0, "USD") />
                <Row label="negative" money=Amount::new(-128_456, "USD") />
                <Row label="one cent" money=Amount::new(1, "USD") />
                <Row label="minus one cent" money=Amount::new(-1, "USD") />
                <Row label="large" money=Amount::new(100_000_000, "USD") />
                <Row label="large negative" money=Amount::new(-100_000_000, "USD") />
            </Section>

            <Section
                title="AUD A$ · EUR € · GBP £"
                subtitle="2 decimals · Western grouping · different symbols"
            >
                <Row label="AUD positive" money=Amount::new(910_000, "AUD") />
                <Row label="AUD negative" money=Amount::new(-690_000, "AUD") />
                <Row label="EUR positive" money=Amount::new(910_000, "EUR") />
                <Row label="EUR negative" money=Amount::new(-690_000, "EUR") />
                <Row label="GBP positive" money=Amount::new(910_000, "GBP") />
                <Row label="GBP negative" money=Amount::new(-690_000, "GBP") />
            </Section>

            <Section
                title="JPY ¥ · KRW ₩"
                subtitle="0 decimal places — no fractional part rendered"
            >
                <Row label="JPY positive" money=Amount::new(9_100, "JPY") />
                <Row label="JPY zero" money=Amount::new(0, "JPY") />
                <Row label="JPY negative" money=Amount::new(-9_100, "JPY") />
                <Row label="JPY large" money=Amount::new(1_000_000, "JPY") />
                <Row label="KRW positive" money=Amount::new(910_000, "KRW") />
                <Row label="KRW negative" money=Amount::new(-910_000, "KRW") />
            </Section>

            <Section title="INR ₹" subtitle="2 decimals · South Asian grouping (1,23,456)">
                <Row label="hundreds" money=Amount::new(45_600, "INR") />
                <Row label="thousands" money=Amount::new(123_400, "INR") />
                <Row label="lakhs" money=Amount::new(12_345_600, "INR") />
                <Row label="crores" money=Amount::new(1_234_567_800, "INR") />
                <Row label="negative" money=Amount::new(-1_234_567_800, "INR") />
            </Section>

            <Section
                title="BTC ₿"
                subtitle="8 decimal places — minor unit is satoshi (10⁻⁸ BTC)"
            >
                <Row label="one satoshi" money=Amount::new(1, "BTC") />
                <Row label="one thousand sat" money=Amount::new(1_000, "BTC") />
                <Row label="one bitcoin" money=Amount::new(100_000_000, "BTC") />
                <Row label="mixed" money=Amount::new(123_456_789, "BTC") />
                <Row label="negative" money=Amount::new(-50_000_000, "BTC") />
            </Section>

            <Section
                title="ETH"
                subtitle="9 decimal places · symbol after · minor unit is nanoether"
            >
                <Row label="one nanoether" money=Amount::new(1, "ETH") />
                <Row label="one ETH" money=Amount::new(1_000_000_000, "ETH") />
                <Row label="mixed" money=Amount::new(1_234_567_891, "ETH") />
                <Row label="negative" money=Amount::new(-500_000_000, "ETH") />
            </Section>

        </div>
    }
}
