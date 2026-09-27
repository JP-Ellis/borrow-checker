//! Holds the register's scroll position across a reset: capture the rows to
//! anchor on before it, restore after the new rows mount.

use leptos::web_sys;
use wasm_bindgen::JsCast as _;

use crate::pages::accounts::register_pages::ScrollAnchor;

/// Selector for register rows; each carries its transaction id.
const ROW: &str = "[data-tx-id]";

/// Anchor candidates in preference order: the row `preferred` names, then the
/// first row whose bottom edge is below the container's top.
#[expect(clippy::float_arithmetic, reason = "pixel offsets from bounding rects")]
#[must_use]
pub fn capture(container: &web_sys::Element, preferred: Option<&str>) -> Vec<ScrollAnchor> {
    let top = container.get_bounding_client_rect().top();
    let mut candidates = Vec::with_capacity(2);
    let Ok(rows) = container.query_selector_all(ROW) else {
        return candidates;
    };
    let mut first_visible = None;
    for i in 0..rows.length() {
        let Some(row) = rows
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        let Some(id) = row.get_attribute("data-tx-id") else {
            continue;
        };
        let rect = row.get_bounding_client_rect();
        if preferred == Some(id.as_str()) {
            candidates.push(ScrollAnchor {
                id: id.clone(),
                offset_px: rect.top() - top,
            });
        }
        if first_visible.is_none() && rect.bottom() > top {
            first_visible = Some(ScrollAnchor {
                id,
                offset_px: rect.top() - top,
            });
        }
        if first_visible.is_some() && (preferred.is_none() || !candidates.is_empty()) {
            break;
        }
    }
    candidates.extend(first_visible);
    candidates
}

/// Scrolls `container` so the anchor's row sits at its captured offset again.
/// Does nothing when the row is not mounted.
#[expect(clippy::float_arithmetic, reason = "pixel offsets from bounding rects")]
pub fn restore(container: &web_sys::Element, anchor: &ScrollAnchor) {
    let Ok(rows) = container.query_selector_all(ROW) else {
        return;
    };
    let top = container.get_bounding_client_rect().top();
    for i in 0..rows.length() {
        let Some(row) = rows
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        if row.get_attribute("data-tx-id").as_deref() == Some(anchor.id.as_str()) {
            let delta = row.get_bounding_client_rect().top() - top - anchor.offset_px;
            #[expect(
                clippy::as_conversions,
                clippy::cast_possible_truncation,
                reason = "a pixel delta fits i32; scrollTop is an i32 in web-sys"
            )]
            let delta = delta.round() as i32;
            container.set_scroll_top(container.scroll_top().saturating_add(delta));
            return;
        }
    }
}

/// Scrolls `container` up to the register's top when the view is below it.
#[expect(clippy::float_arithmetic, reason = "pixel offsets from bounding rects")]
pub fn up_to_register(container: &web_sys::Element) {
    let Ok(Some(register)) = container.query_selector("[aria-label=\"transaction register\"]")
    else {
        return;
    };
    let offset =
        register.get_bounding_client_rect().top() - container.get_bounding_client_rect().top();
    if offset < 0.0_f64 {
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            reason = "a pixel offset fits i32; scrollTop is an i32 in web-sys"
        )]
        let offset = offset.round() as i32;
        container.set_scroll_top(container.scroll_top().saturating_add(offset));
    }
}
