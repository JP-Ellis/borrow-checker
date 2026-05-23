//! QA index page — lists all registered component QA pages.

use leptos::prelude::*;

/// A single card in the QA index grid.
#[component]
pub fn QaCard(
    /// Component display name.
    #[prop(into)]
    title: String,
    /// Route path to navigate to.
    #[prop(into)]
    path: String,
    /// One-line description.
    #[prop(into)]
    description: String,
) -> impl IntoView {
    view! {
        <a
            href=path
            style="display:block;padding:12px 16px;background:var(--bc-surface);\
            border:1px solid var(--bc-border);border-radius:4px;\
            text-decoration:none;color:inherit;"
        >
            <div style="font-family:var(--bc-font-mono);font-size:13px;color:var(--bc-ink);margin-bottom:4px;">
                {title}
            </div>
            <div style="font-size:11px;color:var(--bc-ink-mute);">{description}</div>
        </a>
    }
}

/// Index of all registered QA test pages.
#[component]
pub fn QaIndex() -> impl IntoView {
    view! {
        <div style="padding:24px;max-width:960px">
            <h1 style="font-family:var(--bc-font-mono);font-size:14px;color:var(--bc-ink-mute);\
            margin-bottom:24px;">"// QA component index"</h1>

            <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));\
            gap:8px;">
                <QaCard
                    title="components"
                    path="/__test/component"
                    description="Shared UI components: sparkline, stat card, num, status pill, tag token, TOML view."
                />
                <QaCard
                    title="pages"
                    path="/__test/page"
                    description="Full page QA: accounts dashboard, register, sidebar, and sticky bar."
                />
                <QaCard
                    title="fundamentals"
                    path="/__test/fundamentals"
                    description="Design tokens: type scale, colour system, spacing and geometry."
                />
            </div>
        </div>
    }
}
