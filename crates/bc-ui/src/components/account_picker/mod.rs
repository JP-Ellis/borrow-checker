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
/// closes and restores the last committed value. A "Create" row appears, when
/// the query does not match any existing account name, only if `on_create` is
/// provided.
///
/// # Arguments
///
/// * `accounts` - All selectable accounts.
/// * `selected_id` - Bound signal holding the chosen account ID.
/// * `selected_name` - Bound signal holding the input text / chosen name.
/// * `on_pick` - Called with the chosen [`AccountRef`] when selected.
/// * `on_create` - Optional callback invoked when the user confirms the "Create"
///   row. The "Create" row is rendered only when this callback is provided.
#[cfg(target_arch = "wasm32")]
#[component]
#[expect(clippy::too_many_lines, reason = "Leptos view! macro")]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "count is bounded by suggestions len + 1; h < count is pre-checked"
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
    /// Optional callback invoked when the user confirms the "Create" row. The
    /// "Create" row is rendered only when this callback is provided.
    #[prop(optional)]
    on_create: Option<Callback<String>>,
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
        let show_create = on_create.is_some() && {
            let q = selected_name.get();
            let q = q.trim();
            !q.is_empty() && !matches.iter().any(|a| a.name.eq_ignore_ascii_case(q))
        };
        let count = matches.len() + usize::from(show_create);
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
                } else if show_create {
                    let q = selected_name.get().trim().to_owned();
                    last_committed.set_value(q.clone());
                    selected_id.set(String::new());
                    if let Some(cb) = on_create {
                        cb.run(q);
                    }
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
                        let matches_len = list.len();
                        let q = selected_name.get();
                        let show_create = on_create.is_some()
                            && {
                                let qt = q.trim();
                                !qt.is_empty()
                                    && !list.iter().any(|a| a.name.eq_ignore_ascii_case(qt))
                            };
                        let q_trimmed = q.trim().to_owned();
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
                                {show_create
                                    .then(|| {
                                        let q_for_create = q_trimmed.clone();
                                        let q_for_display = q_trimmed.clone();
                                        view! {
                                            <li
                                                class=move || {
                                                    if highlighted.get() == matches_len {
                                                        format!(
                                                            "{} {} {}",
                                                            style::option,
                                                            style::option_create,
                                                            style::option_hi,
                                                        )
                                                    } else {
                                                        format!("{} {}", style::option, style::option_create)
                                                    }
                                                }
                                                on:mouseenter=move |_| highlighted.set(matches_len)
                                                on:mousedown=move |ev| {
                                                    ev.prevent_default();
                                                    last_committed.set_value(q_for_create.clone());
                                                    selected_id.set(String::new());
                                                    if let Some(cb) = on_create {
                                                        cb.run(q_for_create.clone());
                                                    }
                                                    open.set(false);
                                                }
                                            >
                                                {"Create \""}
                                                {q_for_display}
                                                {"\""}
                                            </li>
                                        }
                                    })}
                            </ul>
                        }
                    })
            }}
        </div>
    }
}
