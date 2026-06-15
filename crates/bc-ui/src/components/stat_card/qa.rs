//! QA page for [`super::StatCard`] and [`super::StatCards`].

use leptos::prelude::*;

use super::StatCard;
use super::StatCards;
use super::StatTone;

/// Renders [`StatCard`] and [`StatCards`] across all tones, counts, and edge cases.
#[component]
pub fn StatCardQa() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "all four tones (reflows as container narrows)"
                </p>
                <StatCards count=4>
                    <StatCard
                        label="income (30d)".into()
                        value="+$9,100".into()
                        sub="avg · commbank"
                        tone=StatTone::Good
                    />
                    <StatCard
                        label="expenses (30d)".into()
                        value="−$6,900".into()
                        sub="avg · 47 tx"
                        tone=StatTone::Bad
                    />
                    <StatCard
                        label="uncategorised".into()
                        value="3".into()
                        sub="not categorised"
                        tone=StatTone::Warn
                    />
                    <StatCard
                        label="last import".into()
                        value="2h ago".into()
                        sub="commbank-au.wasm"
                        tone=StatTone::Neutral
                    />
                </StatCards>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "five cards — reflows 5 → 3+2 → 2+2+1 → 1 (last row always fills)"
                </p>
                <StatCards count=5>
                    <StatCard label="income".into() value="+$9,100".into() tone=StatTone::Good />
                    <StatCard label="expenses".into() value="−$6,900".into() tone=StatTone::Bad />
                    <StatCard label="pending".into() value="0".into() tone=StatTone::Warn />
                    <StatCard label="synced".into() value="now".into() tone=StatTone::Neutral />
                    <StatCard label="savings".into() value="+$2,200".into() tone=StatTone::Good />
                </StatCards>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "long text — overflow behaviour (2 cards always 50/50)"
                </p>
                <StatCards count=2>
                    <StatCard
                        label="a very long eyebrow label that might wrap".into()
                        value="+$1,234,567.89".into()
                        sub="an equally long sub-line with lots of detail"
                        tone=StatTone::Good
                    />
                    <StatCard label="zero".into() value="$0.00".into() tone=StatTone::Neutral />
                </StatCards>
            </section>

        </div>
    }
}
