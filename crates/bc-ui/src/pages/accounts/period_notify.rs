//! Shared out-of-period toast helper used by the add and edit flows.

#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;

use crate::components::period_nav::DisplayWindow;
use crate::components::period_nav::window_containing;
use crate::components::toast::ToastAction;
use crate::components::toast::ToastKind;
use crate::components::toast::ToastStore;
use crate::filter_ctx::FilterStore;
use crate::url_state::History;

/// If `date` is outside the current window, pushes a warning toast telling the
/// user the saved transaction is not visible, with a "View" action that moves
/// the window to that transaction's period in place. All time never notifies.
///
/// # Arguments
///
/// * `toasts` - The toast store to push into.
/// * `store` - The filter store holding the display window.
/// * `date` - The saved transaction's date.
pub(crate) fn notify_if_out_of_period(
    toasts: ToastStore,
    store: FilterStore,
    date: jiff::civil::Date,
) {
    let Some(period) = store
        .window
        .with_untracked(|w| w.period_excluding(date).cloned())
    else {
        return;
    };
    let current = store.window.with_untracked(DisplayWindow::label);
    let message = format!("Transaction saved on {date} — outside the current view ({current}).");
    let on_activate = Callback::new(move |()| {
        store.set_window(
            DisplayWindow::Period {
                start: window_containing(&period, date),
                period: period.clone(),
            },
            History::Replace,
        );
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
