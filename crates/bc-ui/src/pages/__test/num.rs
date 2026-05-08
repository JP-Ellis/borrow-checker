//! [`Num`] component QA page — renders the component across all sign states.

use leptos::prelude::*;

use crate::components::num::Num;

/// Tests the [`Num`] component: positive, negative, zero, large, small.
#[component]
pub fn NumTest() -> impl IntoView {
    view! {
        <div class="page">
            <h1 style="font-size: 20px; margin-bottom: 12px;">"Num component"</h1>

            <table style="border-collapse: collapse; font-family: var(--bc-font-mono); font-size: 12px;">
                <thead>
                    <tr>
                        <th style="padding: 4px 12px 4px 0; text-align: left; color: var(--bc-ink-mute);">
                            "Input"
                        </th>
                        <th style="padding: 4px 0; text-align: left; color: var(--bc-ink-mute);">
                            "Rendered"
                        </th>
                    </tr>
                </thead>
                <tbody>
                    {[
                        ("positive", 128_456_i64),
                        ("zero", 0),
                        ("negative", -128_456),
                        ("one cent", 1),
                        ("minus one cent", -1),
                        ("large", 100_000_000),
                        ("large negative", -100_000_000),
                    ]
                        .into_iter()
                        .map(|(label, cents)| {
                            view! {
                                <tr>
                                    <td style="padding: 4px 12px 4px 0; color: var(--bc-ink-mute);">
                                        {label}
                                    </td>
                                    <td style="padding: 4px 0;">
                                        <Num cents=cents />
                                    </td>
                                </tr>
                            }
                        })
                        .collect::<Vec<_>>()}
                </tbody>
            </table>
        </div>
    }
}
