//! Shared out-of-period toast helper used by the add and edit flows.

#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;

use crate::components::period_nav::is_outside_window;
use crate::components::period_nav::window_containing;
use crate::components::period_nav::window_label;
use crate::components::toast::ToastAction;
use crate::components::toast::ToastKind;
use crate::components::toast::ToastStore;

/// If `date` is outside the current window, pushes a warning toast telling the
/// user the saved transaction is not visible, with a "View" action that jumps
/// the register to that transaction's period.
///
/// Does nothing when `date` is within the window beginning at `window_start`.
///
/// # Arguments
///
/// * `toasts` - The toast store to push into.
/// * `period` - The active period granularity.
/// * `window_start` - The page's display-window start signal (written by the
///   "View" action).
/// * `date` - The saved transaction's date.
pub(crate) fn notify_if_out_of_period(
    toasts: ToastStore,
    period: bc_ipc::Period,
    window_start: RwSignal<jiff::civil::Date>,
    date: jiff::civil::Date,
) {
    if !is_outside_window(&period, window_start.get_untracked(), date) {
        return;
    }
    let current = window_label(&period, window_start.get_untracked());
    let message = format!("Transaction saved on {date} — outside the current view ({current}).");
    let on_activate = Callback::new(move |()| {
        window_start.set(window_containing(&period, date));
    });
    toasts.push(
        ToastKind::Warn,
        message,
        Some(ToastAction {
            label: "View".to_owned(),
            on_activate,
        }),
    );
}
