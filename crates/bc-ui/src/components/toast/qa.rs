//! QA showcase for the toast system.

use leptos::prelude::*;

use super::ToastAction;
use super::ToastHost;
use super::ToastKind;
use super::provide_toast_store;

/// QA fixture: buttons that push each toast kind, plus a warn toast with an
/// action, rendered against a live [`ToastHost`].
#[component]
pub fn ToastQa() -> impl IntoView {
    let store = provide_toast_store();

    view! {
        <div style="padding:24px;display:flex;flex-direction:column;gap:12px;max-width:480px;">
            <button on:click=move |_| {
                store.push(ToastKind::Info, "Informational message.", None);
            }>"Push info"</button>
            <button on:click=move |_| {
                store.push(ToastKind::Success, "Saved successfully.", None);
            }>"Push success"</button>
            <button on:click=move |_| {
                store.push(ToastKind::Error, "Something went wrong.", None);
            }>"Push error"</button>
            <button on:click=move |_| {
                store
                    .push(
                        ToastKind::Warn,
                        "Transaction saved on 2026-06-01 — outside the current view (January 2025).",
                        Some(ToastAction {
                            label: "View".to_owned(),
                            on_activate: Callback::new(|()| {}),
                        }),
                    );
            }>"Push warn + action"</button>
        </div>
        <ToastHost />
    }
}
