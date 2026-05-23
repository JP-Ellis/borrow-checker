//! Route entry for `/__test/fundamentals/colour`.

use leptos::prelude::*;

/// Display name shown in the QA index.
pub const TITLE: &str = "Colour";
/// Route path.
pub const PATH: &str = "/__test/fundamentals/colour";
/// One-line description for the index card.
pub const DESCRIPTION: &str =
    "Ink scale, surfaces, semantic palette, syntax tokens, and contrast matrix.";

/// Section label style.
const LABEL: &str = "font-size:11px;color:var(--bc-ink-mute);margin-bottom:16px;";

/// Ink tiers: (short-name, CSS variable).
const INK_TIERS: &[(&str, &str)] = &[
    ("ink", "--bc-ink"),
    ("ink-soft", "--bc-ink-soft"),
    ("ink-mute", "--bc-ink-mute"),
    ("ink-dim", "--bc-ink-dim"),
];

/// Surface tiers: (short-name, CSS variable).
const SURFACE_TIERS: &[(&str, &str)] = &[
    ("bg", "--bc-bg"),
    ("surface", "--bc-surface"),
    ("surface-alt", "--bc-surface-alt"),
    ("surface-accent", "--bc-surface-accent"),
];

/// Semantic tones: (name, fg-var, soft-var, example-text).
const SEMANTIC_TONES: &[(&str, &str, &str, &str)] = &[
    (
        "accent",
        "--bc-accent",
        "--bc-accent-soft",
        "allocate · import · primary CTA",
    ),
    (
        "good",
        "--bc-good",
        "--bc-good-soft",
        "reconciled · cleared · +delta",
    ),
    (
        "bad",
        "--bc-bad",
        "--bc-bad-soft",
        "overspent · error · −delta",
    ),
    (
        "warn",
        "--bc-warn",
        "--bc-warn-soft",
        "pending · unallocated · attention",
    ),
];

/// Syntax tokens: (name, CSS variable).
const SYNTAX_TOKENS: &[(&str, &str)] = &[
    ("keyword", "--bc-keyword"),
    ("string", "--bc-string"),
    ("number", "--bc-number"),
    ("type", "--bc-type"),
    ("fn", "--bc-fn"),
    ("comment", "--bc-comment"),
];

/// One ink-tier row in the ink-scale section.
#[component]
fn InkRow(
    /// Short name, e.g. `"ink-soft"`.
    name: &'static str,
    /// CSS variable, e.g. `"--bc-ink-soft"`.
    var: &'static str,
) -> impl IntoView {
    view! {
        <div style="display:flex;align-items:baseline;gap:16px;padding:10px 16px;\
        border-bottom:1px solid var(--bc-border)">
            <span style="font-family:var(--bc-font-mono);font-size:10.5px;\
            color:var(--bc-ink-mute);width:72px;flex-shrink:0">{name}</span>
            <span style=format!(
                "font-family:var(--bc-font-sans);font-size:15px;color:var({var});flex:1",
            )>"Smart Access · $487,320.42 · imported 2m ago"</span>
            <span style=format!(
                "font-family:var(--bc-font-mono);font-size:11px;color:var({var})",
            )>"fn cashflow() → +$2,253"</span>
        </div>
    }
}

/// One surface swatch.
#[component]
fn SurfaceSwatch(
    /// Short name, e.g. `"surface-alt"`.
    name: &'static str,
    /// CSS variable, e.g. `"--bc-surface-alt"`.
    var: &'static str,
) -> impl IntoView {
    view! {
        <div style=format!(
            "flex:1;min-width:120px;border:1px solid var(--bc-border);\
            border-radius:4px;overflow:hidden;background:var({var})",
        )>
            <div style="padding:14px 12px">
                <div style="font-family:var(--bc-font-sans);font-size:13px;\
                color:var(--bc-ink);margin-bottom:2px">"Smart Access"</div>
                <div style="font-family:var(--bc-font-mono);font-size:11px;\
                color:var(--bc-ink-soft)">"$487,320.42"</div>
            </div>
            <div style="padding:6px 12px;border-top:1px solid var(--bc-border);\
            font-family:var(--bc-font-mono);font-size:10px;color:var(--bc-ink-mute)">{name}</div>
        </div>
    }
}

/// One nested surface specimen — three concentric boxes, each with a label.
///
/// `layers` is an ordered list of `(token-name, CSS-var)` from outermost to innermost.
#[component]
fn SurfaceNest(
    /// Human-readable label for the whole specimen, e.g. `"card → section → highlight"`.
    label: &'static str,
    /// Layers from outermost to innermost: `(short-name, CSS-var)`.
    layers: &'static [(&'static str, &'static str)],
) -> impl IntoView {
    fn nest(layers: &[(&'static str, &'static str)]) -> impl IntoView {
        match layers {
            [] => view! { <div /> }.into_any(),
            [(name, var), rest @ ..] => {
                let rest = rest.to_vec();
                view! {
                    <div style=format!(
                        "background:var({var});border:1px solid var(--bc-border);\
                        border-radius:4px;padding:10px",
                    )>
                        <div style="font-family:var(--bc-font-mono);font-size:9.5px;\
                        color:var(--bc-ink-dim);margin-bottom:8px">{*name}</div>
                        {if rest.is_empty() {
                            view! {
                                <div style="font-family:var(--bc-font-sans);font-size:12px;\
                                color:var(--bc-ink-soft)">"Smart Access · $487,320.42"</div>
                            }
                                .into_any()
                        } else {
                            nest(&rest).into_any()
                        }}
                    </div>
                }
                .into_any()
            }
        }
    }

    view! {
        <div>
            <div style="font-family:var(--bc-font-mono);font-size:10px;\
            color:var(--bc-ink-mute);margin-bottom:8px">{label}</div>
            {nest(layers)}
        </div>
    }
}

/// One semantic tone card.
#[component]
fn ToneCard(
    /// Token short-name, e.g. `"good"`.
    name: &'static str,
    /// CSS variable for the tone colour, e.g. `"--bc-good"`.
    fg_var: &'static str,
    /// CSS variable for the soft background, e.g. `"--bc-good-soft"`.
    soft_var: &'static str,
    /// One-line usage note.
    note: &'static str,
) -> impl IntoView {
    view! {
        <div style="border:1px solid var(--bc-border);border-radius:4px;overflow:hidden">

            <div style=format!("height:6px;background:var({fg_var})") />
            <div style="padding:12px 14px">
                <div style="display:flex;align-items:center;gap:8px;margin-bottom:6px">
                    <span style=format!(
                        "font-family:var(--bc-font-mono);font-size:12px;\
                        font-weight:600;color:var({fg_var})",
                    )>{name}</span>
                    <span style=format!(
                        "font-family:var(--bc-font-mono);font-size:9.5px;\
                        padding:1px 6px;border-radius:99px;\
                        background:var({soft_var});color:var({fg_var})",
                    )>"soft"</span>
                </div>
                <div style="font-family:var(--bc-font-sans);font-size:11.5px;\
                color:var(--bc-ink-mute)">{note}</div>
            </div>
        </div>
    }
}

/// The ink × surface contrast matrix.
#[component]
fn ContrastMatrix() -> impl IntoView {
    Effect::new(move |_| {
        let js = r#"
            if (!document.getElementById('apca-w3-script')) {
                let logic = document.createElement('script');
                logic.id = 'apca-w3-script';
                logic.type = 'module';
                logic.innerHTML = `
                    import { calcAPCA } from 'https://esm.sh/apca-w3';
                    import { colorParsley } from 'https://esm.sh/colorparsley';
                    import { parse, formatHex } from 'https://esm.sh/culori';

                    function cssToHex(cssColor) {
                        let parsed = parse(cssColor);
                        if (!parsed) return cssColor;
                        return formatHex(parsed);
                    }

                    function updateAPCA() {
                        document.querySelectorAll('.apca-badge').forEach(badge => {
                            let cell = badge.closest('.contrast-cell');
                            if (!cell) return;

                            let style = getComputedStyle(cell);

                            let fgHex = cssToHex(style.color);
                            let bgHex = cssToHex(style.backgroundColor);

                            let txt = colorParsley(fgHex);
                            let bg = colorParsley(bgHex);

                            let lc = calcAPCA(txt, bg);
                            if (isNaN(lc)) {
                                console.error("APCA failed:", style.color, style.backgroundColor, "->", fgHex, bgHex);
                                return;
                            }

                            let absLc = Math.round(Math.abs(lc));
                            badge.textContent = "Lc " + absLc;

                            let colorVar = "--bc-bad";
                            if (absLc >= 75) {
                                colorVar = "--bc-good";
                            } else if (absLc >= 60) {
                                colorVar = "--bc-good";
                            } else if (absLc >= 45) {
                                colorVar = "--bc-warn";
                            } else if (absLc >= 30) {
                                colorVar = "--bc-warn";
                            }

                            badge.style.color = "var(" + colorVar + ")";
                            badge.style.backgroundColor = "var(" + colorVar + "-soft)";
                        });
                    }

                    updateAPCA();
                    const observer = new MutationObserver(updateAPCA);
                    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
                `;
                document.head.appendChild(logic);
            }
        "#;
        drop(js_sys::eval(js));
    });

    view! {
        <div style="border:1px solid var(--bc-border);border-radius:4px;overflow:hidden;position:relative">
            <div style="display:grid;grid-template-columns:80px repeat(4, 1fr);\
            background:var(--bc-surface);border-bottom:1px solid var(--bc-border)">
                <div style="padding:8px 10px" />
                {SURFACE_TIERS
                    .iter()
                    .map(|(surf_name, _)| {
                        view! {
                            <div style="padding:8px 10px;font-family:var(--bc-font-mono);\
                            font-size:9.5px;color:var(--bc-ink-mute);letter-spacing:0.06em;\
                            text-transform:uppercase;border-left:1px solid var(--bc-border)">
                                {*surf_name}
                            </div>
                        }
                    })
                    .collect::<Vec<_>>()}
            </div>

            {INK_TIERS
                .iter()
                .map(|(ink_name, ink_var)| {
                    view! {
                        <div style="display:grid;grid-template-columns:80px repeat(4, 1fr);\
                        border-bottom:1px solid var(--bc-border)">

                            <div style="padding:10px;font-family:var(--bc-font-mono);\
                            font-size:9.5px;color:var(--bc-ink-mute);display:flex;\
                            align-items:center;background:var(--bc-surface);\
                            border-right:1px solid var(--bc-border);letter-spacing:0.06em;\
                            text-transform:uppercase">{*ink_name}</div>

                            {SURFACE_TIERS
                                .iter()
                                .map(|(_, surf_var)| {
                                    let badge = view! {
                                        <span
                                            class="apca-badge"
                                            style="float:right;font-family:var(--bc-font-mono);font-size:9px;\
                                            color:var(--bc-ink-mute);background:var(--bc-surface-alt);\
                                            padding:0 4px;border-radius:99px"
                                        >
                                            "Lc --"
                                        </span>
                                    }
                                        .into_any();

                                    view! {
                                        <div
                                            class="contrast-cell"
                                            style=format!(
                                                "padding:10px 12px;\
                                    background:var({surf_var});color:var({ink_var});\
                                    border-left:1px solid var(--bc-border)",
                                            )
                                        >
                                            <div style="font-family:var(--bc-font-sans);\
                                            font-size:13px;margin-bottom:2px">
                                                {badge} "Smart Access"
                                            </div>
                                            <div style="font-family:var(--bc-font-mono);\
                                            font-size:10.5px;font-variant-numeric:tabular-nums;clear:both">
                                                "$487,320 · 0123"
                                            </div>
                                            <div style="font-family:var(--bc-font-mono);\
                                            font-size:9.5px;margin-top:2px">"caption · eyebrow"</div>
                                        </div>
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </div>
                    }
                })
                .collect::<Vec<_>>()}
        </div>
    }
}

/// Renders the colour fundamentals reference.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "colour reference catalogues all token groups and the contrast matrix in one page"
)]
pub fn ColourFundamentals() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:48px;padding:24px;max-width:960px">

            <section>
                <p style=LABEL>"Ink scale — text on bc-bg"</p>
                <div style="border:1px solid var(--bc-border);border-radius:4px;\
                overflow:hidden;background:var(--bc-bg)">
                    {INK_TIERS
                        .iter()
                        .map(|(name, var)| {
                            view! { <InkRow name=name var=var /> }
                        })
                        .collect::<Vec<_>>()}
                </div>
            </section>

            <section>
                <p style=LABEL>"Surface scale — ink on each surface"</p>
                <div style="display:flex;gap:10px;flex-wrap:wrap">
                    {SURFACE_TIERS
                        .iter()
                        .map(|(name, var)| {
                            view! { <SurfaceSwatch name=name var=var /> }
                        })
                        .collect::<Vec<_>>()}
                </div>
                <div style="margin-top:10px;display:flex;gap:20px">
                    {[("--bc-border", "border"), ("--bc-border-strong", "border-strong")]
                        .iter()
                        .map(|(var, name)| {
                            view! {
                                <div style="display:flex;align-items:center;gap:8px">
                                    <div style=format!(
                                        "width:80px;height:0;border-bottom:1px solid var({var})",
                                    ) />
                                    <span style="font-family:var(--bc-font-mono);font-size:10px;\
                                    color:var(--bc-ink-mute)">{*name}</span>
                                </div>
                            }
                        })
                        .collect::<Vec<_>>()} <div style="display:flex;align-items:center;gap:8px">
                        <div style="width:80px;height:0;border-bottom:2px solid var(--bc-border-strong)" />
                        <span style="font-family:var(--bc-font-mono);font-size:10px;\
                        color:var(--bc-ink-mute)">"border-strong (2px)"</span>
                    </div>
                </div>
            </section>

            <section>
                <p style=LABEL>"Surface nesting — concentric elevation"</p>
                <div style="display:grid;grid-template-columns:1fr 1fr;gap:12px">
                    <SurfaceNest
                        label="bg → surface → surface-alt → surface-accent"
                        layers=&[
                            ("bg", "--bc-bg"),
                            ("surface", "--bc-surface"),
                            ("surface-alt", "--bc-surface-alt"),
                            ("surface-accent", "--bc-surface-accent"),
                        ]
                    />
                    <SurfaceNest
                        label="bg → surface-alt → surface → surface-accent"
                        layers=&[
                            ("bg", "--bc-bg"),
                            ("surface-alt", "--bc-surface-alt"),
                            ("surface", "--bc-surface"),
                            ("surface-accent", "--bc-surface-accent"),
                        ]
                    />
                </div>
            </section>

            <section>
                <p style=LABEL>"Semantic tones — with soft background tint"</p>
                <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(200px,1fr));\
                gap:10px">
                    {SEMANTIC_TONES
                        .iter()
                        .map(|(name, fg, soft, note)| {
                            view! { <ToneCard name=name fg_var=fg soft_var=soft note=note /> }
                        })
                        .collect::<Vec<_>>()}
                </div>
            </section>

            <section>
                <p style=LABEL>"Syntax palette — metadata, code, and console chrome"</p>
                <div style="border:1px solid var(--bc-border);border-radius:4px;overflow:hidden">

                    <div style="display:flex;border-bottom:1px solid var(--bc-border)">
                        {SYNTAX_TOKENS
                            .iter()
                            .map(|(name, var)| {
                                view! {
                                    <div style="flex:1;padding:10px 12px;border-right:1px solid var(--bc-border)">
                                        <div style=format!(
                                            "font-family:var(--bc-font-mono);font-size:18px;\
                                        font-weight:600;color:var({var});margin-bottom:4px",
                                        )>"Aa"</div>
                                        <div style="font-family:var(--bc-font-mono);font-size:9.5px;\
                                        color:var(--bc-ink-mute)">{*name}</div>
                                    </div>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </div>

                    <div style="padding:16px;background:var(--bc-surface-alt);\
                    font-family:var(--bc-font-mono);font-size:12.5px;line-height:1.9">
                        <div>
                            <span style="color:var(--bc-accent)">"> "</span>
                            <span style="color:var(--bc-keyword)">"find "</span>
                            <span style="color:var(--bc-string)">"\"coles\""</span>
                            <span style="color:var(--bc-ink-dim)">" --last "</span>
                            <span style="color:var(--bc-number)">"30d"</span>
                            <span style="color:var(--bc-ink-dim)">" --in "</span>
                            <span style="color:var(--bc-type)">"Groceries"</span>
                        </div>
                        <div style="color:var(--bc-comment)">
                            "// 14 matches · sum −$1,284.55"
                        </div>
                        <div>
                            <span style="color:var(--bc-fn)">"autocat"</span>
                            <span style="color:var(--bc-ink-dim)">"(tx_4421) → "</span>
                            <span style="color:var(--bc-type)">"Groceries:Coles"</span>
                        </div>
                        <div style="margin-top:6px">
                            <span style="color:var(--bc-keyword)">"budget"</span>
                            <span style="color:var(--bc-fn)">" allocate"</span>
                            <span style="color:var(--bc-string)">" \"Savings:Japan 2026\""</span>
                            <span style="color:var(--bc-ink-dim)">" --amount "</span>
                            <span style="color:var(--bc-number)">"500"</span>
                        </div>
                    </div>
                </div>
            </section>

            <section>
                <p style=LABEL>"Contrast matrix — ink tier × surface tier"</p>
                <ContrastMatrix />
                <p style="margin-top:10px;font-family:var(--bc-font-mono);font-size:10px;\
                color:var(--bc-ink-dim)">
                    "Each cell renders at three sizes (sans 13px body, mono 10.5px data, mono 9.5px caption). \
                    Toggle dark/light via the shell toggle to verify both themes."
                </p>
            </section>

        </div>
    }
}
