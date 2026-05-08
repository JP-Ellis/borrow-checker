//! Root styling test page — exercises design tokens and base layout.

use leptos::prelude::*;

/// Tests root/base styling: colour tokens, typography scale, spacing, radius.
#[component]
pub fn Root() -> impl IntoView {
    view! {
        <div class="page">
            <h1 style="font-size: 20px; margin-bottom: 12px;">"Root styling"</h1>

            <section style="margin-bottom: 16px;">
                <p style="margin-bottom: 4px; color: var(--bc-ink-mute); font-size: 11px;">
                    "Ink scale"
                </p>
                <p style="color: var(--bc-ink);">"--bc-ink (primary text)"</p>
                <p style="color: var(--bc-ink-soft);">"--bc-ink-soft"</p>
                <p style="color: var(--bc-ink-mute);">"--bc-ink-mute"</p>
                <p style="color: var(--bc-ink-dim);">"--bc-ink-dim"</p>
            </section>

            <section style="margin-bottom: 16px;">
                <p style="margin-bottom: 4px; color: var(--bc-ink-mute); font-size: 11px;">
                    "Surface scale"
                </p>
                <div style="display: flex; gap: 8px;">
                    <div style="padding: 8px 12px; background: var(--bc-bg); border: 1px solid var(--bc-border);">
                        "bg"
                    </div>
                    <div style="padding: 8px 12px; background: var(--bc-surface); border: 1px solid var(--bc-border);">
                        "surface"
                    </div>
                    <div style="padding: 8px 12px; background: var(--bc-surface-alt); border: 1px solid var(--bc-border);">
                        "surface-alt"
                    </div>
                    <div style="padding: 8px 12px; background: var(--bc-surface-hi); border: 1px solid var(--bc-border);">
                        "surface-hi"
                    </div>
                </div>
            </section>

            <section style="margin-bottom: 16px;">
                <p style="margin-bottom: 4px; color: var(--bc-ink-mute); font-size: 11px;">
                    "Semantic colours"
                </p>
                <div style="display: flex; gap: 8px;">
                    <span style="color: var(--bc-good);">"good"</span>
                    <span style="color: var(--bc-warn);">"warn"</span>
                    <span style="color: var(--bc-bad);">"bad"</span>
                    <span style="color: var(--bc-accent);">"accent"</span>
                </div>
            </section>

            <section>
                <p style="margin-bottom: 4px; color: var(--bc-ink-mute); font-size: 11px;">
                    "Typography"
                </p>
                <p style="font-family: var(--bc-font-sans);">
                    "Sans: Inter Tight — the quick brown fox"
                </p>
                <p style="font-family: var(--bc-font-mono);">
                    "Mono: Fira Code — 0123 +$1,234.56 −$0.01"
                </p>
            </section>
        </div>
    }
}
