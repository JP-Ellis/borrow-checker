//! Global "New transaction" modal: the register editor's new mode, opened
//! from the top bar or a budget detail.

#[cfg(debug_assertions)]
pub mod qa;

use bc_ipc::AccountNode;
use bc_ipc::AccountRef;
use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast as _;
use leptos::web_sys;
use stylance::import_style;

use crate::components::account_picker::AccountCurrencies;
use crate::components::account_picker::account_currencies;
use crate::components::account_picker::account_paths;
use crate::components::account_picker::balance_currency;
use crate::components::toast::use_toasts;
use crate::components::transaction_row::Created;
use crate::components::transaction_row::DetailSource;
use crate::components::transaction_row::TransactionDetail;
use crate::components::transaction_row::editable::EditableTransaction;
use crate::pages::accounts::use_transaction_link;
use crate::pending_warnings::WarnedToast;
use crate::pending_warnings::toast_created;
use crate::write_epoch::use_write_epoch;

import_style!(style, "new_transaction.module.scss");

/// The elements Tab can land on inside the dialog.
const FOCUSABLE: &str = "a[href], button:not([disabled]), input:not([disabled]), \
                         select:not([disabled]), textarea:not([disabled]), \
                         [tabindex]:not([tabindex=\"-1\"]), [contenteditable=\"true\"]";

// MARK: Opener

/// Opens the new-transaction overlay, seeded with an account or not; provided
/// once by the shell.
#[derive(Clone, Copy)]
pub struct NewTxOpener(pub Callback<Option<AccountRef>>);

impl NewTxOpener {
    /// Opens the overlay.
    ///
    /// # Arguments
    ///
    /// * `seed` - The account the first leg starts in, or `None` for a blank
    ///   draft.
    pub fn open(&self, seed: Option<AccountRef>) {
        self.0.run(seed);
    }
}

/// Provides the new-transaction opener into context. Call once, at the shell
/// root.
///
/// # Arguments
///
/// * `open` - Opens the overlay on a seed.
pub fn provide_new_tx_opener(open: Callback<Option<AccountRef>>) {
    provide_context(NewTxOpener(open));
}

/// Reads the new-transaction opener from context; outside the shell it does
/// nothing.
///
/// # Returns
///
/// The opener from context, or one that ignores every seed.
#[must_use]
pub fn use_new_tx_opener() -> NewTxOpener {
    use_context::<NewTxOpener>().unwrap_or_else(|| NewTxOpener(Callback::new(|_| {})))
}

// MARK: Overlay

/// The overlay's state: `None` when closed, `Some(seed)` while open.
pub type OverlayState = Option<Option<AccountRef>>;

/// The new-transaction modal, mounted once in the shell.
///
/// While `open` holds `Some(seed)` the modal shows a draft whose first leg
/// is in `seed`'s account, when there is one. The account list is fetched
/// on each open, so the pickers see accounts created since. Opening while
/// already open keeps the current draft.
///
/// # Arguments
///
/// * `open` - `Some(seed)` while the overlay is open; cleared on close.
#[component]
pub fn NewTransactionOverlay(
    /// `Some(seed)` while the overlay is open; cleared on close.
    open: RwSignal<OverlayState>,
) -> impl IntoView {
    let is_open = Memo::new(move |_| open.with(Option::is_some));
    let on_close = Callback::new(move |()| open.set(None));
    move || {
        is_open.get().then(|| {
            let seed = open.get_untracked().flatten();
            let nodes = LocalResource::new(|| async {
                bc_ipc::client::list_accounts()
                    .await
                    .map_err(|e| e.to_string())
            });
            let accounts = Signal::derive(move || nodes.get());
            view! { <NewTransactionModal seed=seed accounts=accounts on_close=on_close /> }
        })
    }
}

/// The modal body: backdrop, focus-trapped dialog and the editor.
///
/// Focus moves into the dialog at mount and returns to the element that held
/// it on unmount. The editor mounts once `accounts` resolves.
///
/// # Arguments
///
/// * `seed` - The first leg's account, or `None`.
/// * `accounts` - The account list: `None` while loading, `Some(Err(_))`
///   when the fetch failed.
/// * `on_close` - Closes the overlay.
#[component]
fn NewTransactionModal(
    /// The first leg's account, or `None`.
    seed: Option<AccountRef>,
    /// The account list: `None` while loading, `Some(Err(_))` when the fetch
    /// failed.
    accounts: Signal<Option<Result<Vec<AccountNode>, String>>>,
    /// Closes the overlay.
    on_close: Callback<()>,
) -> impl IntoView {
    let backdrop_ref = NodeRef::<leptos::html::Div>::new();
    let dialog_ref = NodeRef::<leptos::html::Div>::new();
    let toasts = use_toasts();
    let link = use_transaction_link();
    let writes = use_write_epoch();

    // MARK: Focus

    let opener = StoredValue::new_local(
        document()
            .active_element()
            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok()),
    );
    on_cleanup(move || {
        if let Some(el) = opener.try_get_value().flatten() {
            let _focused = el.focus();
        }
    });

    // `None` while loading, then whether the list arrived.
    let ready = Memo::new(move |_| accounts.with(|a| a.as_ref().map(Result::is_ok)));
    Effect::new(move |_| {
        let loaded = ready.get() == Some(true);
        request_animation_frame(move || {
            let Some(dialog) = dialog_ref.get_untracked() else {
                return;
            };
            let target = loaded
                .then(|| dialog.query_selector("input").ok().flatten())
                .flatten()
                .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok());
            let _focused = match target {
                Some(input) => input.focus(),
                None => dialog.focus(),
            };
        });
    });

    // MARK: Keys

    let on_keydown = move |e: web_sys::KeyboardEvent| {
        if e.default_prevented() {
            return;
        }
        let Some(dialog) = dialog_ref.get_untracked() else {
            return;
        };
        match e.key().as_str() {
            // The editor discards a draft on Escape from inside it and marks
            // the event handled; this catches the rest of the dialog.
            "Escape" => {
                e.prevent_default();
                on_close.run(());
            }
            "Tab" => trap_tab(&dialog, &e),
            // The accounts page opens an inline draft on an unhandled Enter.
            "Enter" if targets(&e, dialog.as_ref()) => e.prevent_default(),
            _ => {}
        }
    };

    // MARK: Backdrop

    // Closes only on a press whose mousedown and mouseup both land on the
    // backdrop itself. A drag between the backdrop and the dialog in either
    // direction fires `click` on the backdrop, their common ancestor.
    let pressed_backdrop = StoredValue::new(false);
    let released_backdrop = StoredValue::new(false);
    let on_backdrop = move |e: &web_sys::Event| {
        backdrop_ref
            .get_untracked()
            .is_some_and(|b| targets(e, b.as_ref()))
    };

    // MARK: Save

    let on_created = Callback::new(move |created: Created| {
        // The editor stays mounted with Save enabled after a create, so close
        // first to rule out a second submit.
        on_close.run(());
        writes.bump();
        toast_created(
            toasts,
            &created.warnings,
            WarnedToast::Sticky(link.view_action(&created)),
        );
    });

    let seed = StoredValue::new(seed);

    view! {
        <div
            class=style::overlay
            node_ref=backdrop_ref
            data-testid="new-transaction-overlay"
            on:mousedown=move |e| pressed_backdrop.set_value(on_backdrop(&e))
            on:mouseup=move |e| released_backdrop.set_value(on_backdrop(&e))
            on:click=move |e| {
                if pressed_backdrop.get_value() && released_backdrop.get_value() && on_backdrop(&e)
                {
                    on_close.run(());
                }
                pressed_backdrop.set_value(false);
                released_backdrop.set_value(false);
            }
        >
            <div
                class=style::dialog
                node_ref=dialog_ref
                role="dialog"
                aria-modal="true"
                aria-label="New transaction"
                tabindex="-1"
                on:keydown=on_keydown
            >
                <div class=style::header>
                    <h2 class=style::title>"New transaction"</h2>
                    <button
                        class=style::close
                        aria-label="close"
                        data-testid="new-transaction-close"
                        on:click=move |_| on_close.run(())
                    >
                        "\u{2715}"
                    </button>
                </div>
                // The editor provides its own context; the closure gives it an
                // owner of its own.
                {move || match ready.get() {
                    None => {
                        view! {
                            <div class=style::status role="status">
                                "loading accounts\u{2026}"
                            </div>
                        }
                            .into_any()
                    }
                    Some(false) => {
                        let message = accounts
                            .with_untracked(|a| match a {
                                Some(Err(e)) => e.clone(),
                                _ => String::new(),
                            });
                        view! {
                            <div class=format!("{} {}", style::status, style::error) role="alert">
                                {format!("Couldn't load accounts: {message}")}
                            </div>
                        }
                            .into_any()
                    }
                    Some(true) => {
                        let nodes = accounts
                            .with_untracked(|a| a.clone().and_then(Result::ok))
                            .unwrap_or_default();
                        let refs = account_paths(&nodes);
                        provide_context(
                            AccountCurrencies(Signal::stored(account_currencies(&nodes))),
                        );
                        let draft = seed_draft(seed.get_value(), &nodes, &refs);
                        view! {
                            <TransactionDetail
                                source=DetailSource::New(draft)
                                on_created=on_created
                                on_discard=on_close
                                accounts=refs
                            />
                        }
                            .into_any()
                    }
                }}
            </div>
        </div>
    }
}

// MARK: Helpers

/// Builds the overlay's draft, dated today.
///
/// A seed is resolved against the account list for its full path, and its
/// leg starts in the account's balance currency.
///
/// # Arguments
///
/// * `seed` - The first leg's account, or `None`.
/// * `nodes` - The account list.
/// * `refs` - The picker entries for `nodes`.
fn seed_draft(
    seed: Option<AccountRef>,
    nodes: &[AccountNode],
    refs: &[AccountRef],
) -> EditableTransaction {
    let seed = seed.map(|s| refs.iter().find(|r| r.id == s.id).cloned().unwrap_or(s));
    let currency = seed
        .as_ref()
        .and_then(|s| nodes.iter().find(|n| n.id == s.id))
        .map_or_default(balance_currency);
    EditableTransaction::new_draft(seed.as_ref(), &currency, jiff::Zoned::now().date())
}

/// Whether `e` was dispatched at `el` itself, not at a descendant.
///
/// # Arguments
///
/// * `e` - The event.
/// * `el` - The element to compare against.
fn targets(e: &web_sys::Event, el: &web_sys::EventTarget) -> bool {
    e.target().is_some_and(|t| t == *el)
}

/// Keeps Tab and Shift+Tab inside `dialog`, wrapping at either end.
///
/// # Arguments
///
/// * `dialog` - The dialog element.
/// * `e` - The Tab keydown.
fn trap_tab(dialog: &web_sys::HtmlElement, e: &web_sys::KeyboardEvent) {
    let items: Vec<web_sys::HtmlElement> =
        dialog.query_selector_all(FOCUSABLE).map_or_default(|list| {
            (0..list.length())
                .filter_map(|i| list.item(i))
                .filter_map(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
                // A hidden element has no offset parent.
                .filter(|el| el.offset_parent().is_some())
                .collect()
        });
    let (Some(first), Some(last)) = (items.first(), items.last()) else {
        e.prevent_default();
        return;
    };
    let active = document().active_element();
    let dialog_el: &web_sys::Element = dialog.as_ref();
    let inside = active
        .as_ref()
        .is_some_and(|a| dialog.contains(Some(a)) && a != dialog_el);
    let at = |el: &web_sys::HtmlElement| active.as_ref() == Some(el.as_ref());
    let wrap_to = if e.shift_key() {
        (!inside || at(first)).then_some(last)
    } else {
        (!inside || at(last)).then_some(first)
    };
    if let Some(target) = wrap_to {
        e.prevent_default();
        let _focused = target.focus();
    }
}
