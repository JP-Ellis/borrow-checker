//! SVG sparkline — income and expense lines over a time axis.
use core::cmp::Ordering;
use core::sync::atomic::AtomicUsize;

use bc_ipc::Currency;
use bc_ipc::USD;
use leptos::prelude::*;
use leptos::web_sys;
use rust_decimal::prelude::ToPrimitive as _;
use stylance::import_style;

use crate::components::num::format_amount;

import_style!(style, "sparkline.module.scss");

/// Per-instance counter for generating unique SVG gradient IDs.
static NEXT_SPARKLINE_ID: AtomicUsize = AtomicUsize::new(0);

// MARK: Scaling

/// Scales `values` (i64 cents) to SVG y-coordinates within `[y_top, y_bottom]`.
///
/// X coordinates are evenly distributed across `[0, width]`.
/// When all values are equal, maps to the vertical midpoint.
///
/// # Arguments
///
/// * `values` - Time-series values in the currency's minor unit.
/// * `y_top` - SVG y coordinate for the maximum value (top of chart).
/// * `y_bottom` - SVG y coordinate for the minimum value (bottom of chart).
/// * `width` - Total SVG width for x distribution.
///
/// # Returns
///
/// A `Vec<(f32, f32)>` of `(x, y)` SVG coordinates.
#[must_use]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::float_arithmetic,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::expect_used,
    dead_code,
    reason = "SVG coordinate math on non-empty slices; precision loss, expect, and dead_code are safe here; used in later tasks"
)]
pub fn scale_to_svg(values: &[i64], y_top: f32, y_bottom: f32, width: f32) -> Vec<(f32, f32)> {
    if values.is_empty() {
        return vec![];
    }
    let min = *values.iter().min().expect("non-empty slice");
    let max = *values.iter().max().expect("non-empty slice");
    let range = max - min;
    let mid_y = f32::midpoint(y_top, y_bottom);
    let n = values.len();

    values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = if n == 1 {
                width / 2.0
            } else {
                (i as f32 / (n - 1) as f32) * width
            };
            let y = if range == 0 {
                mid_y
            } else {
                let t = (v - min) as f32 / range as f32;
                y_bottom - t * (y_bottom - y_top)
            };
            (x, y)
        })
        .collect()
}

/// Scales `values` to SVG y-coordinates using explicit `[min_val, max_val]` bounds.
///
/// Use this to plot multiple series on a shared y-axis.
///
/// # Arguments
///
/// * `values` - Time-series values in the currency's minor unit.
/// * `min_val` - Explicit minimum (maps to `y_bottom`).
/// * `max_val` - Explicit maximum (maps to `y_top`).
/// * `y_top` - SVG y coordinate for the maximum value.
/// * `y_bottom` - SVG y coordinate for the minimum value.
/// * `width` - Chart area width for x distribution.
///
/// # Returns
///
/// A `Vec<(f32, f32)>` of `(x, y)` SVG coordinates.
#[must_use]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::float_arithmetic,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "SVG coordinate math; precision loss acceptable for screen coordinates"
)]
fn scale_to_svg_with_bounds(
    values: &[i64],
    min_val: i64,
    max_val: i64,
    y_top: f32,
    y_bottom: f32,
    width: f32,
) -> Vec<(f32, f32)> {
    if values.is_empty() {
        return vec![];
    }
    let range = max_val - min_val;
    let mid_y = f32::midpoint(y_top, y_bottom);
    let n = values.len();
    values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = if n == 1 {
                width / 2.0
            } else {
                (i as f32 / (n - 1) as f32) * width
            };
            let y = if range == 0 {
                mid_y
            } else {
                let t = (v - min_val) as f32 / range as f32;
                y_bottom - t * (y_bottom - y_top)
            };
            (x, y)
        })
        .collect()
}

// MARK: Utilities

/// Converts `(f32, f32)` point pairs to an SVG `points` attribute string.
#[must_use]
#[inline]
fn points_attr(pts: &[(f32, f32)]) -> String {
    pts.iter()
        .map(|(x, y)| format!("{x:.1},{y:.1}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Converts data points to an SVG `points` string closed at `chart_bot`.
///
/// Appends `(last_x, chart_bot)` and `(first_x, chart_bot)` so the resulting
/// polygon fills the area between the polyline and the chart floor.
#[must_use]
#[inline]
fn fill_points_attr(pts: &[(f32, f32)], chart_bot: f32) -> String {
    let (Some(&(first_x, _)), Some(&(last_x, _))) = (pts.first(), pts.last()) else {
        return String::new();
    };
    let mut all = pts.to_vec();
    all.push((last_x, chart_bot));
    all.push((first_x, chart_bot));
    points_attr(&all)
}

// MARK: Data Types

/// Re-exported from `bc_ipc` so callers can `use crate::components::sparkline::SparkPoint`
/// without adding a direct `bc_ipc` import.
pub use bc_ipc::SparkPoint;

// MARK: Component

/// Title slot for [`Sparkline`] — accepts arbitrary HTML children.
#[slot]
pub struct Title {
    /// Title content — plain text or rich markup.
    children: Children,
}

/// SVG cash-flow sparkline — income (solid) and expenses (dashed) over time.
///
/// Both lines share a single y-axis so their magnitudes can be compared directly.
/// Hovering over the chart shows a crosshair and the exact values for the nearest point.
///
/// # Arguments
///
/// * `points` - Time-series data points, oldest first.
/// * `title` - Title slot — use `<Title slot>` to pass text or rich markup.
/// * `currency` - Currency for formatting y-axis and hover labels. Defaults to [`USD`].
#[component]
#[expect(
    clippy::too_many_lines,
    clippy::float_arithmetic,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "Leptos component; SVG chart and mouse coordinate arithmetic; f64→f32 casts safe for screen coordinates"
)]
pub fn Sparkline(
    /// Time-series data, oldest first.
    points: Vec<SparkPoint>,
    /// Title slot — plain text or rich markup via `<Title slot>`.
    title: Title,
    /// Currency for formatting values. Defaults to [`USD`].
    #[prop(default = &USD)]
    currency: &'static Currency,
    /// Show gradient fill under each series. Defaults to `true`.
    #[prop(default = true)]
    show_fill: bool,
) -> impl IntoView {
    const W: f32 = 300.0;
    const H: f32 = 52.0;
    const PAD: f32 = 4.0;
    const CHART_TOP: f32 = PAD;
    const CHART_BOT: f32 = H - PAD;

    // Gather series values.
    let to_plot = |amt: &bc_ipc::Amount| -> i64 {
        let mut scaled = amt.value;
        scaled.rescale(u32::from(currency.decimals));
        scaled.mantissa().to_i64().unwrap_or(0)
    };
    let income_vals: Vec<i64> = points.iter().map(|p| to_plot(&p.income)).collect();
    let expense_vals: Vec<i64> = points.iter().map(|p| to_plot(&p.expenses)).collect();

    // Shared y-scale: find the global min/max across both series.
    let global_min = income_vals
        .iter()
        .chain(expense_vals.iter())
        .copied()
        .min()
        .unwrap_or(0);
    let global_max = income_vals
        .iter()
        .chain(expense_vals.iter())
        .copied()
        .max()
        .unwrap_or(0);

    // Scale both series with shared bounds across the full SVG width.
    let income_pts: Vec<(f32, f32)> = scale_to_svg_with_bounds(
        &income_vals,
        global_min,
        global_max,
        CHART_TOP,
        CHART_BOT,
        W,
    );
    let expense_pts: Vec<(f32, f32)> = scale_to_svg_with_bounds(
        &expense_vals,
        global_min,
        global_max,
        CHART_TOP,
        CHART_BOT,
        W,
    );

    let income_str = points_attr(&income_pts);
    let expense_str = points_attr(&expense_pts);

    // Fill areas — computed before pts are moved into StoredValue.
    let spark_id = NEXT_SPARKLINE_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let grad_income_id = format!("spark-fill-income-{spark_id}");
    let grad_expense_id = format!("spark-fill-expense-{spark_id}");
    let income_fill_str = fill_points_attr(&income_pts, CHART_BOT);
    let expense_fill_str = fill_points_attr(&expense_pts, CHART_BOT);
    let income_fill_url = format!("url(#{grad_income_id})");
    let expense_fill_url = format!("url(#{grad_expense_id})");
    // Leptos wraps each <stop> child in a <g>, which SVG parsers ignore inside
    // <linearGradient>. Injecting as raw innerHTML bypasses that wrapping.
    let defs_html = format!(
        "<linearGradient id=\"{grad_income_id}\" gradientUnits=\"userSpaceOnUse\" x1=\"0\" y1=\"{CHART_TOP}\" x2=\"0\" y2=\"{CHART_BOT}\"><stop offset=\"0%\" stop-color=\"var(--bc-good)\" stop-opacity=\"0.3\"/><stop offset=\"100%\" stop-color=\"var(--bc-good)\" stop-opacity=\"0\"/></linearGradient>\
         <linearGradient id=\"{grad_expense_id}\" gradientUnits=\"userSpaceOnUse\" x1=\"0\" y1=\"{CHART_TOP}\" x2=\"0\" y2=\"{CHART_BOT}\"><stop offset=\"0%\" stop-color=\"var(--bc-bad)\" stop-opacity=\"0.3\"/><stop offset=\"100%\" stop-color=\"var(--bc-bad)\" stop-opacity=\"0\"/></linearGradient>"
    );

    // Endpoint dot positions as CSS percentages for non-distorted HTML rendering.
    let has_data = !income_pts.is_empty();
    let last_x_pct = income_pts.last().map_or(100.0_f32, |p| p.0 / W * 100.0);
    let last_income_y_pct = income_pts
        .last()
        .map_or(CHART_TOP / H * 100.0, |p| p.1 / H * 100.0);
    let last_expense_y_pct = expense_pts
        .last()
        .map_or(CHART_BOT / H * 100.0, |p| p.1 / H * 100.0);

    let labels: Vec<String> = points.iter().map(|p| p.label.clone()).collect();

    // Hover state.
    let hovered: RwSignal<Option<usize>> = RwSignal::new(None);
    let container_ref = NodeRef::<leptos::html::Div>::new();

    // Store computed data for reactive closures (Copy handles).
    let stored_xs: StoredValue<Vec<f32>> =
        StoredValue::new(income_pts.iter().map(|p| p.0).collect());
    let stored_income_pts = StoredValue::new(income_pts);
    let stored_expense_pts = StoredValue::new(expense_pts);
    let stored_points = StoredValue::new(points);

    let on_mousemove = move |ev: web_sys::MouseEvent| {
        let Some(el) = container_ref.get_untracked() else {
            return;
        };
        let rect = el.get_bounding_client_rect();
        let el_w = rect.width() as f32;
        if el_w <= 0.0 {
            return;
        }
        let svg_x = (ev.client_x() as f32 - rect.left() as f32) / el_w * W;
        let xs = stored_xs.get_value();
        if xs.is_empty() {
            return;
        }
        let idx = xs
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                ((*a - svg_x).abs())
                    .partial_cmp(&((*b - svg_x).abs()))
                    .unwrap_or(Ordering::Equal)
            })
            .map(|(i, _)| i);
        hovered.set(idx);
    };

    let on_mouseleave = move |_: web_sys::MouseEvent| {
        hovered.set(None);
    };

    let hover_info = move || {
        let i = hovered.get()?;
        stored_points.with_value(|pts| {
            let p = pts.get(i)?;
            let inc = {
                let s = format_amount(&p.income.value, currency);
                s.strip_prefix('+').map(ToOwned::to_owned).unwrap_or(s)
            };
            let exp = {
                let s = format_amount(&p.expenses.value, currency);
                s.strip_prefix('+').map(ToOwned::to_owned).unwrap_or(s)
            };
            Some(view! {
                {p.label.clone()}
                " — "
                <span class=style::income>{inc}</span>
                " · "
                <span class=style::expense>{exp}</span>
            })
        })
    };

    // Crosshair: SVG line only — dots are rendered as HTML below to stay circular.
    let crosshair_line = move || {
        hovered.get().map(|i| {
            let (x, _) =
                stored_income_pts.with_value(|pts| pts.get(i).copied().unwrap_or((0.0, CHART_TOP)));
            view! {
                <line
                    x1=x
                    y1=CHART_TOP
                    x2=x
                    y2=CHART_BOT
                    stroke="var(--bc-ink-mute)"
                    stroke-width="0.5"
                    stroke-dasharray="2 2"
                    opacity="0.6"
                    vector-effect="non-scaling-stroke"
                />
            }
        })
    };

    let hover_dots = move || {
        hovered.get().map(|i| {
            let (x, income_y) =
                stored_income_pts.with_value(|pts| pts.get(i).copied().unwrap_or((0.0, CHART_TOP)));
            let expense_y =
                stored_expense_pts.with_value(|pts| pts.get(i).map_or(CHART_BOT, |p| p.1));
            let x_pct = x / W * 100.0;
            let it_pct = income_y / H * 100.0;
            let ey_pct = expense_y / H * 100.0;
            view! {
                <div class=style::dot_good style=format!("left:{x_pct:.1}%;top:{it_pct:.1}%") />
                <div class=style::dot_bad style=format!("left:{x_pct:.1}%;top:{ey_pct:.1}%") />
            }
        })
    };

    view! {
        <div class=style::wrap>
            <div class=style::title>{(title.children)()}</div>
            <div class=style::hover_info>{hover_info}</div>
            <div
                node_ref=container_ref
                class=style::svg_wrap
                on:mousemove=on_mousemove
                on:mouseleave=on_mouseleave
            >
                <svg
                    class=style::svg
                    viewBox=format!("0 0 {W} {H}")
                    width="100%"
                    preserveAspectRatio="none"
                    aria-hidden="true"
                >
                    <defs inner_html=defs_html />
                    // Fill areas (before strokes so lines render on top)
                    {show_fill
                        .then(|| {
                            view! {
                                <polygon
                                    points=income_fill_str
                                    fill=income_fill_url
                                    stroke="none"
                                />
                                <polygon
                                    points=expense_fill_str
                                    fill=expense_fill_url
                                    stroke="none"
                                />
                            }
                        })}
                    // Income line (solid green)
                    <polyline
                        points=income_str
                        fill="none"
                        stroke="var(--bc-good)"
                        stroke-width="1.5"
                        vector-effect="non-scaling-stroke"
                    />
                    // Expense line (dashed red)
                    <polyline
                        points=expense_str
                        fill="none"
                        stroke="var(--bc-bad)"
                        stroke-width="1.5"
                        stroke-dasharray="3 2"
                        opacity="0.85"
                        vector-effect="non-scaling-stroke"
                    />
                    // Hover crosshair line only — dots live in HTML below
                    {crosshair_line}
                </svg>
                // Endpoint dots — HTML so they remain circular at any SVG scale
                {has_data
                    .then(|| {
                        view! {
                            <div
                                class=style::dot_good
                                style=format!("left:{last_x_pct:.1}%;top:{last_income_y_pct:.1}%")
                            />
                            <div
                                class=style::dot_bad
                                style=format!("left:{last_x_pct:.1}%;top:{last_expense_y_pct:.1}%")
                            />
                        }
                    })}
                // Hover dots
                {hover_dots}
            </div>
            <div class=style::axis>
                {labels.into_iter().map(|l| view! { <span>{l}</span> }).collect::<Vec<_>>()}
            </div>
        </div>
    }
}

#[cfg(debug_assertions)]
pub mod qa;

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::fill_points_attr;
    use super::scale_to_svg;
    use super::scale_to_svg_with_bounds;

    #[test]
    fn scale_maps_min_to_bottom() {
        let pts = scale_to_svg(&[0, 100], 0.0, 40.0, 100.0);
        assert_eq!(pts[0].1, 40.0_f32);
    }

    #[test]
    fn scale_maps_max_to_top() {
        let pts = scale_to_svg(&[0, 100], 0.0, 40.0, 100.0);
        assert_eq!(pts[1].1, 0.0_f32);
    }

    #[test]
    fn scale_equal_values_maps_to_midpoint() {
        let pts = scale_to_svg(&[50, 50], 0.0, 40.0, 100.0);
        assert_eq!(pts[0].1, 20.0_f32);
        assert_eq!(pts[1].1, 20.0_f32);
    }

    #[test]
    fn bounds_income_at_top_expense_in_middle() {
        // income=100 is global max → y_top; expense=60 is in middle
        let income_pts = scale_to_svg_with_bounds(&[100], 0, 100, 0.0, 40.0, 100.0);
        let expense_pts = scale_to_svg_with_bounds(&[60], 0, 100, 0.0, 40.0, 100.0);
        assert_eq!(income_pts[0].1, 0.0_f32); // max → top
        assert_eq!(expense_pts[0].1, 16.0_f32); // 60% from bottom → 40 - 0.6*40 = 16
    }

    #[test]
    fn bounds_equal_range_maps_to_midpoint() {
        let pts = scale_to_svg_with_bounds(&[50, 50], 50, 50, 0.0, 40.0, 100.0);
        assert_eq!(pts[0].1, 20.0_f32);
        assert_eq!(pts[1].1, 20.0_f32);
    }

    #[test]
    fn fill_points_closes_to_bottom() {
        let pts = vec![(0.0_f32, 10.0_f32), (100.0_f32, 20.0_f32)];
        let s = fill_points_attr(&pts, 52.0);
        assert_eq!(s, "0.0,10.0 100.0,20.0 100.0,52.0 0.0,52.0");
    }

    #[test]
    fn fill_points_empty_returns_empty() {
        assert_eq!(fill_points_attr(&[], 52.0), "");
    }
}
