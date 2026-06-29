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

/// Pure account-filtering logic with no framework dependencies.
mod matching;
#[cfg(target_arch = "wasm32")]
use bc_ipc::AccountRef;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
pub use matching::account_paths;
#[cfg(target_arch = "wasm32")]
pub use matching::filter_accounts;
#[cfg(target_arch = "wasm32")]
pub use matching::match_segments;
#[cfg(target_arch = "wasm32")]
pub use matching::split_leaf;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
import_style!(style, "picker.module.scss");

/// An account autocomplete: a text input plus a filtered suggestion list.
///
/// Keyboard-driven: `↑`/`↓` move the highlight, `Enter` selects, `Escape`
/// closes and restores the last committed value. The picker only selects
/// existing accounts; accounts must be created beforehand (e.g. on the Accounts
/// page).
///
/// # Arguments
///
/// * `accounts` - All selectable accounts.
/// * `selected_id` - Bound signal holding the chosen account ID.
/// * `selected_name` - Bound signal holding the input text / chosen name.
/// * `on_pick` - Called with the chosen [`AccountRef`] when selected.
#[cfg(target_arch = "wasm32")]
#[component]
#[expect(clippy::too_many_lines, reason = "Leptos view! macro")]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "count is bounded by suggestions len; h < count is pre-checked"
)]
#[expect(
    clippy::indexing_slicing,
    reason = "matches[h] is guarded by h < matches.len() check above"
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
        // Escape is handled before the count check so the menu can always close.
        if ev.key() == "Escape" {
            selected_name.set(last_committed.get_value());
            open.set(false);
            ev.prevent_default();
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
                on:keydown=on_keydown
                data-testid="account-input"
            />
            {move || {
                open.get()
                    .then(|| {
                        let list = suggestions();
                        let q = selected_name.get();
                        view! {
                            <ul class=style::menu>
                                {list
                                    .into_iter()
                                    .enumerate()
                                    .map(|(idx, a)| {
                                        let picked = a.clone();
                                        let (prefix, leaf) = split_leaf(&a.name);
                                        let render_runs = |s: &str| {
                                            match_segments(s, &q)
                                                .into_iter()
                                                .map(|seg| {
                                                    if seg.hit {
                                                        view! { <mark>{seg.text}</mark> }.into_any()
                                                    } else {
                                                        view! { {seg.text} }.into_any()
                                                    }
                                                })
                                                .collect::<Vec<_>>()
                                        };
                                        let prefix_runs = render_runs(&prefix);
                                        let leaf_runs = render_runs(&leaf);
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
                                                <span class=style::opt_prefix>{prefix_runs}</span>
                                                <span class=style::opt_leaf>{leaf_runs}</span>
                                            </li>
                                        }
                                    })
                                    .collect::<Vec<_>>()}
                            </ul>
                        }
                    })
            }}
        </div>
    }
}
