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
                <Row label="positive" money=Amount::from_minor(128_456, "USD", 2) />
                <Row label="zero" money=Amount::from_minor(0, "USD", 2) />
                <Row label="negative" money=Amount::from_minor(-128_456, "USD", 2) />
                <Row label="one cent" money=Amount::from_minor(1, "USD", 2) />
                <Row label="minus one cent" money=Amount::from_minor(-1, "USD", 2) />
                <Row label="large" money=Amount::from_minor(100_000_000, "USD", 2) />
                <Row label="large negative" money=Amount::from_minor(-100_000_000, "USD", 2) />
            </Section>

            <Section
                title="AUD A$ · EUR € · GBP £"
                subtitle="2 decimals · Western grouping · different symbols"
            >
                <Row label="AUD positive" money=Amount::from_minor(910_000, "AUD", 2) />
                <Row label="AUD negative" money=Amount::from_minor(-690_000, "AUD", 2) />
                <Row label="EUR positive" money=Amount::from_minor(910_000, "EUR", 2) />
                <Row label="EUR negative" money=Amount::from_minor(-690_000, "EUR", 2) />
                <Row label="GBP positive" money=Amount::from_minor(910_000, "GBP", 2) />
                <Row label="GBP negative" money=Amount::from_minor(-690_000, "GBP", 2) />
            </Section>

            <Section
                title="JPY ¥ · KRW ₩"
                subtitle="0 decimal places — no fractional part rendered"
            >
                <Row label="JPY positive" money=Amount::from_minor(9_100, "JPY", 0) />
                <Row label="JPY zero" money=Amount::from_minor(0, "JPY", 0) />
                <Row label="JPY negative" money=Amount::from_minor(-9_100, "JPY", 0) />
                <Row label="JPY large" money=Amount::from_minor(1_000_000, "JPY", 0) />
                <Row label="KRW positive" money=Amount::from_minor(910_000, "KRW", 0) />
                <Row label="KRW negative" money=Amount::from_minor(-910_000, "KRW", 0) />
            </Section>

            <Section title="INR ₹" subtitle="2 decimals · South Asian grouping (1,23,456)">
                <Row label="hundreds" money=Amount::from_minor(45_600, "INR", 2) />
                <Row label="thousands" money=Amount::from_minor(123_400, "INR", 2) />
                <Row label="lakhs" money=Amount::from_minor(12_345_600, "INR", 2) />
                <Row label="crores" money=Amount::from_minor(1_234_567_800, "INR", 2) />
                <Row label="negative" money=Amount::from_minor(-1_234_567_800, "INR", 2) />
            </Section>

            <Section
                title="BTC ₿"
                subtitle="8 decimal places — minor unit is satoshi (10⁻⁸ BTC)"
            >
                <Row label="one satoshi" money=Amount::from_minor(1, "BTC", 8) />
                <Row label="one thousand sat" money=Amount::from_minor(1_000, "BTC", 8) />
                <Row label="one bitcoin" money=Amount::from_minor(100_000_000, "BTC", 8) />
                <Row label="mixed" money=Amount::from_minor(123_456_789, "BTC", 8) />
                <Row label="negative" money=Amount::from_minor(-50_000_000, "BTC", 8) />
            </Section>

            <Section
                title="ETH"
                subtitle="9 decimal places · symbol after · minor unit is nanoether"
            >
                <Row label="one nanoether" money=Amount::from_minor(1, "ETH", 9) />
                <Row label="one ETH" money=Amount::from_minor(1_000_000_000, "ETH", 9) />
                <Row label="mixed" money=Amount::from_minor(1_234_567_891, "ETH", 9) />
                <Row label="negative" money=Amount::from_minor(-500_000_000, "ETH", 9) />
            </Section>

        </div>
    }
}
