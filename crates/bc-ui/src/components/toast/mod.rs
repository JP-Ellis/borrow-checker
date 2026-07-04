//! Transient, top-layer toast notifications.
//!
//! Toasts render in the browser top layer via the Popover API, so they float
//! above all page content without z-index juggling. Each toast auto-dismisses
//! after [`DEFAULT_TOAST_MS`] and can carry a single optional action button.

use core::time::Duration;

use leptos::html::Div;
use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "toast.module.scss");

/// Auto-dismiss delay for every toast, in milliseconds.
const DEFAULT_TOAST_MS: u64 = 7_000;

// MARK: Data model

/// Semantic category of a toast, controlling colour token and styling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    /// Neutral informational message.
    Info,
    /// Successful operation.
    Success,
    /// Advisory warning (the default for out-of-period notices).
    Warn,
    /// Error condition.
    Error,
}

impl ToastKind {
    /// Returns the kebab-case identifier used for the `data-kind` attribute.
    ///
    /// # Returns
    ///
    /// A static string: `"info"`, `"success"`, `"warn"`, or `"error"`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// An optional action rendered as a button inside a toast.
#[derive(Clone)]
pub struct ToastAction {
    /// Button label.
    pub label: String,
    /// Invoked when the button is activated; the toast is dismissed afterward.
    pub on_activate: Callback<()>,
}

/// A single transient notification.
#[derive(Clone)]
pub struct Toast {
    /// Unique, monotonically increasing identifier (stable `<For>` key).
    pub id: u64,
    /// Semantic category.
    pub kind: ToastKind,
    /// Message body.
    pub message: String,
    /// Optional action button.
    pub action: Option<ToastAction>,
}

// MARK: Store

/// Reactive handle to the live toast list, provided once at the shell root.
#[derive(Clone, Copy)]
pub struct ToastStore {
    /// Live toast list, newest last.
    items: RwSignal<Vec<Toast>>,
    /// Next id to assign, monotonically increasing.
    next_id: RwSignal<u64>,
}

impl ToastStore {
    /// Pushes a new toast and schedules its auto-dismiss.
    ///
    /// # Arguments
    ///
    /// * `kind` - Semantic category.
    /// * `message` - Message body.
    /// * `action` - Optional action button.
    ///
    /// # Returns
    ///
    /// The id assigned to the new toast.
    pub fn push(
        &self,
        kind: ToastKind,
        message: impl Into<String>,
        action: Option<ToastAction>,
    ) -> u64 {
        let id = self.next_id.get_untracked();
        self.next_id.set(id.wrapping_add(1));
        self.items.update(|list| {
            list.push(Toast {
                id,
                kind,
                message: message.into(),
                action,
            });
        });
        let store = *self;
        set_timeout(
            move || store.dismiss(id),
            Duration::from_millis(DEFAULT_TOAST_MS),
        );
        id
    }

    /// Removes the toast with the given id, if it is still present.
    ///
    /// # Arguments
    ///
    /// * `id` - The id returned by [`ToastStore::push`].
    pub fn dismiss(&self, id: u64) {
        self.items.update(|list| list.retain(|t| t.id != id));
    }
}

/// Provides an empty [`ToastStore`] into context. Call once, at the shell root.
///
/// # Returns
///
/// The provided [`ToastStore`] handle.
#[must_use]
pub fn provide_toast_store() -> ToastStore {
    let store = ToastStore {
        items: RwSignal::new(Vec::new()),
        next_id: RwSignal::new(0),
    };
    provide_context(store);
    store
}

/// Reads the [`ToastStore`] from context.
///
/// # Returns
///
/// The shared [`ToastStore`]; a fresh detached store if none was provided
/// (keeps callers panic-free in isolated test/QA contexts).
#[must_use]
pub fn use_toasts() -> ToastStore {
    use_context::<ToastStore>().unwrap_or_else(|| ToastStore {
        items: RwSignal::new(Vec::new()),
        next_id: RwSignal::new(0),
    })
}

// MARK: View

/// Top-layer host that renders the live toast stack. Mount once, in the shell.
#[component]
pub fn ToastHost() -> impl IntoView {
    let store = use_toasts();
    let items = store.items;
    let container = NodeRef::<Div>::new();

    // Promote the container into the top layer once it is mounted. Errors
    // (e.g. an already-open popover) are benign and ignored.
    Effect::new(move |_| {
        if let Some(el) = container.get() {
            #[expect(
                clippy::let_underscore_must_use,
                clippy::let_underscore_untyped,
                let_underscore_drop,
                reason = "show_popover() returns Result<(), JsValue>; errors are benign"
            )]
            let _ = el.show_popover();
        }
    });

    view! {
        <div node_ref=container class=style::host popover="manual" role="status" aria-live="polite">
            <For
                each=move || items.get()
                key=|t| t.id
                children=move |toast| toast_item(toast, store)
            />
        </div>
    }
}

/// A single toast card with message, optional action, and a dismiss button.
///
/// A plain helper rather than a `#[component]`: it is only ever rendered from
/// the `<For>` in [`ToastHost`], so it does not need its own props struct.
///
/// # Arguments
///
/// * `toast` - The toast to render.
/// * `store` - Store used to dismiss this toast on action/close.
fn toast_item(toast: Toast, store: ToastStore) -> impl IntoView {
    let id = toast.id;
    let action = toast.action.clone();
    let action_view = action.map(|a| {
        let on_activate = a.on_activate;
        view! {
            <button
                class=style::action
                type="button"
                on:click=move |_| {
                    on_activate.run(());
                    store.dismiss(id);
                }
            >
                {a.label}
            </button>
        }
    });

    view! {
        <div class=style::toast data-kind=toast.kind.as_str()>
            <span class=style::message>{toast.message}</span>
            {action_view}
            <button
                class=style::dismiss
                type="button"
                aria-label="dismiss notification"
                on:click=move |_| store.dismiss(id)
            >
                "\u{00D7}"
            </button>
        </div>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
