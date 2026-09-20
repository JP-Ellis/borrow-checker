//! Running-balance cell: the formatted amount over an in-cell bar on a shared
//! per-commodity axis.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

pub mod geometry;

#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use rust_decimal::Decimal;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
import_style!(style, "balance_cell.module.scss");

/// A balance plus the axis it is drawn on: `(lo, hi)` with `lo ≤ 0 ≤ hi`.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, PartialEq)]
pub struct BalanceCell {
    /// The balance.
    pub amount: bc_ipc::Amount,
    /// Shared per-commodity axis over the loaded rows.
    pub axis: (Decimal, Decimal),
}

/// Renders a [`BalanceCell`]: the formatted amount, right-aligned, over a bar
/// that fills from the zero line. `None` renders an em dash and no bar.
///
/// # Arguments
///
/// * `cell` - The balance and axis, or `None` when hidden or mixed-commodity.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn BalanceCellView(
    /// The balance and its axis; `None` renders `—`.
    cell: Signal<Option<BalanceCell>>,
) -> impl IntoView {
    let currencies = crate::currency_ctx::use_currency_store();
    move || match cell.get() {
        None => view! {
            <span class=style::cell data-testid="balance-cell" data-empty="true">
                <span class=format!("{} {}", style::text, style::neutral)>"\u{2014}"</span>
            </span>
        }
        .into_any(),
        Some(BalanceCell {
            amount,
            axis: (lo, hi),
        }) => {
            let meta = crate::components::num::meta::display_meta_for(
                &amount.currency_code,
                &currencies.get(),
            );
            let text = crate::components::num::format_amount(&amount.value, &meta);
            let g = geometry::bar_geometry(amount.value, lo, hi);
            let sign_class = match amount.value.cmp(&Decimal::ZERO) {
                core::cmp::Ordering::Greater => style::positive,
                core::cmp::Ordering::Less => style::negative,
                core::cmp::Ordering::Equal => style::neutral,
            };
            // The cell edge is the zero line when the axis touches zero.
            let show_zero = g.zero_pct > 0.0_f64 && g.zero_pct < 100.0_f64;
            let label = format!("balance {text} {}", amount.currency_code);
            view! {
                <span class=style::cell data-testid="balance-cell" aria-label=label>
                    <span class=style::layer aria-hidden="true">
                        {show_zero
                            .then(|| {
                                view! {
                                    <span class=style::zero style:left=format!("{}%", g.zero_pct) />
                                }
                            })}
                        <span
                            class=format!("{} {}", style::bar, sign_class)
                            style:left=format!("{}%", g.left_pct)
                            style:width=format!("{}%", g.width_pct)
                        />
                    </span>
                    <span class=format!("{} {}", style::text, sign_class)>{text}</span>
                </span>
            }
            .into_any()
        }
    }
}

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod qa;
