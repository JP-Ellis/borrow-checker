//! Account autocomplete picker used by the posting-row editor.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

#[cfg(all(target_arch = "wasm32", debug_assertions))]
pub mod qa;

#[cfg(target_arch = "wasm32")]
mod label;
/// Pure account-filtering logic with no framework dependencies.
mod matching;
#[cfg(target_arch = "wasm32")]
use std::collections::HashMap;

#[cfg(target_arch = "wasm32")]
use bc_ipc::AccountRef;
#[cfg(target_arch = "wasm32")]
pub use label::AccountPathLabel;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
pub use matching::account_currencies;
#[cfg(target_arch = "wasm32")]
pub use matching::account_paths;
#[cfg(target_arch = "wasm32")]
pub use matching::balance_currency;
#[cfg(target_arch = "wasm32")]
pub use matching::filter_accounts;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
use crate::pages::settings::accounts::links::ACCOUNTS_HREF;

#[cfg(target_arch = "wasm32")]
import_style!(style, "picker.module.scss");

/// Each account's default commodity, keyed by account id, provided by a host
/// that holds the account list.
///
/// The posting editor reads it when an account is picked, to fill in a leg
/// whose currency is still empty. Without it in context, picking leaves the
/// currency alone.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
pub struct AccountCurrencies(pub Signal<HashMap<String, String>>);

/// An account autocomplete: a text input plus a filtered suggestion list.
///
/// Keyboard-driven: `↑`/`↓` move the highlight, `Enter` selects, `Escape`
/// closes and restores the last committed value. Leaving the input closes the
/// menu.
///
/// The picker never creates accounts, by design: a typo would otherwise
/// become a new account. When nothing matches, the menu links to Settings →
/// Accounts instead. On the web build the link opens a new tab, so the form
/// around the picker keeps its unsaved input.
///
/// # Arguments
///
/// * `accounts` - All selectable accounts.
/// * `selected_id` - Bound signal holding the chosen account ID.
/// * `selected_name` - Bound signal holding the input text / chosen name.
/// * `on_pick` - Called with the chosen [`AccountRef`] when selected.
#[cfg(target_arch = "wasm32")]
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "count is bounded by suggestions len; h < count is pre-checked"
)]
#[expect(
    clippy::indexing_slicing,
    reason = "matches[h] is guarded by h < matches.len() check above"
)]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn AccountPicker(
    /// All selectable accounts.
    accounts: Vec<AccountRef>,
    /// Bound signal holding the chosen account ID.
    selected_id: RwSignal<String>,
    /// Bound signal holding the input text / chosen name.
    selected_name: RwSignal<String>,
    /// Called with the chosen account when selected via click or keyboard.
    on_pick: Callback<AccountRef>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let highlighted = RwSignal::new(0_usize);
    let accounts = StoredValue::new(accounts);
    let last_committed = StoredValue::new(selected_name.get_untracked());

    let suggestions = move || {
        let query = selected_name.get();
        accounts.with_value(|a| filter_accounts(a, &query))
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        // Escape is handled before the count check so the menu can always
        // close. An open menu consumes it; a closed one lets it reach the
        // editor, where it discards a draft.
        if ev.key() == "Escape" {
            selected_name.set(last_committed.get_value());
            if open.get_untracked() {
                open.set(false);
                ev.prevent_default();
                ev.stop_propagation();
            }
            return;
        }
        let matches = suggestions();
        let count = matches.len();
        if count == 0 {
            return;
        }
        match ev.key().as_str() {
            "ArrowDown" => {
                highlighted.update(|h| *h = (*h + 1).min(count - 1));
                ev.prevent_default();
            }
            "ArrowUp" => {
                highlighted.update(|h| *h = h.saturating_sub(1));
                ev.prevent_default();
            }
            "Enter" => {
                let h = highlighted.get();
                if h < matches.len() {
                    let a = matches[h].clone();
                    last_committed.set_value(a.name.clone());
                    selected_id.set(a.id.clone());
                    selected_name.set(a.name.clone());
                    on_pick.run(a);
                }
                open.set(false);
                ev.prevent_default();
            }
            _ => {}
        }
    };

    view! {
        <div class=style::picker>
            <input
                class=style::input
                prop:value=move || selected_name.get()
                on:input=move |ev| {
                    selected_name.set(event_target_value(&ev));
                    highlighted.set(0);
                    open.set(true);
                }
                on:focus=move |_| open.set(true)
                on:blur=move |_| open.set(false)
                on:keydown=on_keydown
                data-testid="account-input"
            />
            {move || {
                open.get()
                    .then(|| {
                        let list = suggestions();
                        let q = selected_name.get();
                        if list.is_empty() {
                            return no_match(open).into_any();
                        }
                        view! {
                            <ul class=style::menu>
                                {list
                                    .into_iter()
                                    .enumerate()
                                    .map(|(idx, a)| {
                                        let picked = a.clone();
                                        view! {
                                            <li
                                                class=move || {
                                                    if highlighted.get() == idx {
                                                        format!("{} {}", style::option, style::option_hi)
                                                    } else {
                                                        style::option.to_owned()
                                                    }
                                                }
                                                on:mouseenter=move |_| highlighted.set(idx)
                                                on:mousedown=move |ev| {
                                                    ev.prevent_default();
                                                    last_committed.set_value(picked.name.clone());
                                                    selected_id.set(picked.id.clone());
                                                    selected_name.set(picked.name.clone());
                                                    on_pick.run(picked.clone());
                                                    open.set(false);
                                                }
                                            >
                                                <AccountPathLabel path=a.name.clone() query=q.clone() />
                                            </li>
                                        }
                                    })
                                    .collect::<Vec<_>>()}
                            </ul>
                        }
                            .into_any()
                    })
            }}
        </div>
    }
}

/// The menu shown when no account matches: a link to Settings → Accounts.
///
/// The link keeps the input focused on `mousedown`, so the input's blur does
/// not close the menu before the click lands; the click then closes it. The
/// web build opens a new tab. The Tauri webview has no new-window handler and
/// would drop a `_blank` link, so the desktop build navigates in place.
///
/// # Arguments
///
/// * `open` - The picker's menu state.
#[cfg(target_arch = "wasm32")]
fn no_match(open: RwSignal<bool>) -> impl IntoView {
    let new_tab = cfg!(feature = "http");
    view! {
        <ul class=style::menu>
            <li class=style::empty>
                "// no account matches — "
                <a
                    class=style::empty_link
                    href=ACCOUNTS_HREF
                    target=new_tab.then_some("_blank")
                    rel=new_tab.then_some("noopener")
                    on:mousedown=|ev| ev.prevent_default()
                    on:click=move |_| open.set(false)
                >
                    "manage accounts ↗"
                </a>
            </li>
        </ul>
    }
}
