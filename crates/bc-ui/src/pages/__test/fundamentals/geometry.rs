//! Route entry for `/__test/fundamentals/geometry`.

use leptos::prelude::*;

/// Display name shown in the QA index.
pub const TITLE: &str = "Geometry";
/// Route path.
pub const PATH: &str = "/__test/fundamentals/geometry";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Space scale, radius, motion timing, and z-index layers.";

/// Section label style.
const LABEL: &str = "font-size:11px;color:var(--bc-ink-mute);margin-bottom:16px;";

/// Space scale: (token-name, px-value).
const SPACE_STEPS: &[(&str, &str)] = &[
    ("--bc-space-1", "4px"),
    ("--bc-space-2", "6px"),
    ("--bc-space-3", "8px"),
    ("--bc-space-4", "10px"),
    ("--bc-space-5", "12px"),
    ("--bc-space-6", "14px"),
    ("--bc-space-7", "18px"),
    ("--bc-space-8", "22px"),
    ("--bc-space-9", "28px"),
];

/// Radius scale: (token-name, px-value, label).
const RADIUS_STEPS: &[(&str, &str, &str)] = &[
    ("--bc-radius-tag", "3px", "tag token"),
    ("--bc-radius-kbd", "4px", "kbd / chip"),
    ("--bc-radius-tab", "5px", "tab"),
    ("--bc-radius-control", "6px", "control / input"),
    ("--bc-radius-card", "8px", "card"),
    ("--bc-radius-pill", "99px", "pill / status"),
];

/// Motion tokens: (label, CSS duration value, CSS ease name, token names).
const MOTION_ROWS: &[(&str, &str, &str, &str)] = &[
    (
        "fast — micro feedback (hover, focus ring)",
        "var(--bc-duration-fast)",
        "var(--bc-ease-out)",
        "--bc-duration-fast 120ms  +  --bc-ease-out",
    ),
    (
        "default — panel open, colour transition",
        "var(--bc-duration)",
        "var(--bc-ease-default)",
        "--bc-duration 200ms  +  --bc-ease-default",
    ),
    (
        "slow — full-page or content-shift transitions",
        "var(--bc-duration-slow)",
        "var(--bc-ease-out)",
        "--bc-duration-slow 350ms  +  --bc-ease-out",
    ),
];

/// Z-index layers: (token-name, value, usage note).
const Z_LAYERS: &[(&str, &str, &str)] = &[
    ("--bc-z-below", "−1", "inert backdrop — behind everything"),
    ("--bc-z-base", "0", "normal document flow"),
    ("--bc-z-raised", "10", "sticky bars, floating cards"),
    ("--bc-z-overlay", "20", "drawers, side panels"),
    ("--bc-z-modal", "30", "modal dialogs"),
    ("--bc-z-toast", "40", "toast notifications"),
    ("--bc-z-top", "50", "command palette, global shortcuts"),
];

/// One row in the space-scale section.
///
/// Shows the token name, pixel value, an actual-size height indicator, and a
/// padding-in-practice demo so the space can be felt in context.
#[component]
fn SpaceRow(
    /// Token name, e.g. `--bc-space-3`.
    name: &'static str,
    /// Pixel value string, e.g. `"8px"`.
    px: &'static str,
) -> impl IntoView {
    view! {
        <div style="display:flex;align-items:center;gap:16px;padding:8px 0;\
        border-bottom:1px solid var(--bc-border)">
            <span style="font-family:var(--bc-font-mono);font-size:10.5px;\
            color:var(--bc-ink-mute);width:96px;flex-shrink:0;">{name}</span>
            <span style="font-family:var(--bc-font-mono);font-size:10.5px;\
            color:var(--bc-ink-dim);width:28px;flex-shrink:0;">{px}</span>

            <div style="width:40px;height:32px;flex-shrink:0;\
            display:flex;align-items:center;justify-content:center;">
                <div style=format!(
                    "width:var({name});height:var({name});\
                    background:var(--bc-ink-soft);border-radius:1px;",
                ) />
            </div>

            <div style=format!(
                "flex:1;background:var(--bc-surface-alt);border:1px solid var(--bc-border);\
                border-radius:3px;padding:var({name});",
            )>
                <div style="background:var(--bc-surface-accent);\
                border-radius:2px;height:16px;" />
            </div>
        </div>
    }
}

/// One radius swatch.
#[component]
fn RadiusSwatch(
    /// Token name, e.g. `--bc-radius-card`.
    name: &'static str,
    /// Pixel value string, e.g. `"8px"`.
    px: &'static str,
    /// Usage label, e.g. `"card"`.
    label: &'static str,
) -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;align-items:center;gap:6px">
            <div style=format!(
                "width:56px;height:56px;background:var(--bc-surface-accent);\
                border:1px solid var(--bc-border-strong);\
                border-radius:var({name})",
            ) />
            <div style="font-family:var(--bc-font-mono);font-size:10px;\
            color:var(--bc-ink-soft);text-align:center">{px}</div>
            <div style="font-family:var(--bc-font-mono);font-size:9.5px;\
            color:var(--bc-ink-mute);text-align:center">{label}</div>
        </div>
    }
}

/// One animated motion demo row. Hover to trigger the transition.
#[component]
fn MotionRow(
    /// Human-readable label shown to the right of the track.
    label: &'static str,
    /// CSS `transition-duration` value (may reference a custom property).
    duration_css: &'static str,
    /// CSS `transition-timing-function` value.
    ease_css: &'static str,
    /// Token name string shown beneath the demo.
    tokens: &'static str,
) -> impl IntoView {
    let (active, set_active) = signal(false);

    view! {
        <div
            on:mouseenter=move |_| set_active.set(true)
            on:mouseleave=move |_| set_active.set(false)
            style="cursor:default;padding:12px 0;border-bottom:1px solid var(--bc-border)"
        >
            <div style="display:flex;align-items:center;gap:16px;margin-bottom:6px">

                <div style="position:relative;width:180px;height:24px;\
                background:var(--bc-surface-alt);border:1px solid var(--bc-border);\
                border-radius:4px;overflow:hidden;flex-shrink:0">
                    <div style=move || {
                        format!(
                            "position:absolute;top:4px;left:4px;width:16px;height:16px;\
                        border-radius:3px;background:var(--bc-accent);\
                        transform:translateX({x}px);\
                        transition:transform {dur} {ease}",
                            x = if active.get() { 156_i32 } else { 0_i32 },
                            dur = duration_css,
                            ease = ease_css,
                        )
                    } />
                </div>
                <span style="font-family:var(--bc-font-sans);font-size:13px;\
                color:var(--bc-ink-soft)">{label}</span>
            </div>
            <div style="font-family:var(--bc-font-mono);font-size:10px;\
            color:var(--bc-ink-dim)">{tokens}</div>
        </div>
    }
}

/// Renders the geometry (spacing, radius, motion, z-index) fundamentals reference.
#[component]
pub fn GeometryFundamentals() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:48px;padding:24px;max-width:800px">

            <section>
                <p style=LABEL>"Space scale"</p>
                <div style="border:1px solid var(--bc-border);border-radius:4px;\
                padding:0 20px">

                    <div style="display:flex;align-items:center;gap:16px;\
                    padding:8px 0;border-bottom:1px solid var(--bc-border)">
                        <span style="font-family:var(--bc-font-mono);font-size:9.5px;\
                        color:var(--bc-ink-dim);width:96px;flex-shrink:0;">"token"</span>
                        <span style="font-family:var(--bc-font-mono);font-size:9.5px;\
                        color:var(--bc-ink-dim);width:28px;flex-shrink:0;">"value"</span>
                        <span style="font-family:var(--bc-font-mono);font-size:9.5px;\
                        color:var(--bc-ink-dim);width:40px;flex-shrink:0;">"size"</span>
                        <span style="font-family:var(--bc-font-mono);font-size:9.5px;\
                        color:var(--bc-ink-dim);flex:1;">"padding demo"</span>
                    </div>
                    {SPACE_STEPS
                        .iter()
                        .map(|(name, px)| {
                            view! { <SpaceRow name=name px=px /> }
                        })
                        .collect::<Vec<_>>()}
                </div>
            </section>

            <section>
                <p style=LABEL>"Radius scale"</p>
                <div style="border:1px solid var(--bc-border);border-radius:4px;\
                padding:20px 24px;display:flex;gap:24px;flex-wrap:wrap;align-items:flex-end">
                    {RADIUS_STEPS
                        .iter()
                        .map(|(name, px, label)| {
                            view! { <RadiusSwatch name=name px=px label=label /> }
                        })
                        .collect::<Vec<_>>()}
                </div>
            </section>

            <section>
                <p style=LABEL>"Motion — hover each row to preview"</p>
                <div style="border:1px solid var(--bc-border);border-radius:4px;\
                padding:0 20px">
                    {MOTION_ROWS
                        .iter()
                        .map(|(label, dur, ease, tokens)| {
                            view! {
                                <MotionRow
                                    label=label
                                    duration_css=dur
                                    ease_css=ease
                                    tokens=tokens
                                />
                            }
                        })
                        .collect::<Vec<_>>()}
                </div>
                <div style="margin-top:12px;font-family:var(--bc-font-mono);\
                font-size:10px;color:var(--bc-ink-dim)">
                    "ease curves — "
                    <span style="color:var(--bc-ink-mute)">
                        "ease-default: cubic-bezier(0.2, 0.7, 0.3, 1)"
                    </span> " · "
                    <span style="color:var(--bc-ink-mute)">
                        "ease-in: cubic-bezier(0.4, 0, 1, 1)"
                    </span> " · "
                    <span style="color:var(--bc-ink-mute)">
                        "ease-out: cubic-bezier(0, 0, 0.2, 1)"
                    </span>
                </div>
            </section>

            <section>
                <p style=LABEL>"Z-index layers"</p>
                <div style="border:1px solid var(--bc-border);border-radius:4px;overflow:hidden">
                    {Z_LAYERS
                        .iter()
                        .enumerate()
                        .map(|(i, (name, value, note))| {
                            view! {
                                <div style=format!(
                                    "display:grid;grid-template-columns:140px 40px 1fr;\
                                align-items:center;padding:8px 14px;{}",
                                    if i > 0 { "border-top:1px solid var(--bc-border)" } else { "" },
                                )>
                                    <span style="font-family:var(--bc-font-mono);font-size:11px;\
                                    color:var(--bc-ink-soft)">{*name}</span>
                                    <span style="font-family:var(--bc-font-mono);font-size:11px;\
                                    color:var(--bc-ink);font-weight:500;text-align:right;\
                                    padding-right:16px">{*value}</span>
                                    <span style="font-family:var(--bc-font-sans);font-size:12px;\
                                    color:var(--bc-ink-mute)">{*note}</span>
                                </div>
                            }
                        })
                        .collect::<Vec<_>>()}
                </div>
            </section>

        </div>
    }
}
