//! Command palette (⌘K) — structured filter builder.
//!
//! The palette is a small screen state machine: a [`Screen::Root`] menu of
//! [`Dimension`]s (with a `dimension:value` prefix jump) and a
//! [`Screen::Dimension`] value-entry screen per dimension. Committing a value
//! writes into the app-wide [`crate::filter_ctx::FilterStore`] and returns to
//! the root menu so several dimensions can be added in one session. Account
//! navigation (the previous ⌘K behaviour) has been dropped — the palette only
//! builds filters now.

#[cfg(target_arch = "wasm32")]
use bc_ipc::AccountNode;
#[cfg(target_arch = "wasm32")]
use bc_ipc::AmountFilter;
#[cfg(target_arch = "wasm32")]
use bc_ipc::Reconciliation;
#[cfg(target_arch = "wasm32")]
use bc_ipc::TagInfo;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::web_sys;
#[cfg(target_arch = "wasm32")]
use rust_decimal::Decimal;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
import_style!(style, "palette.module.scss");

/// A filter dimension selectable in the palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    /// Account subtree.
    Account,
    /// Tag.
    Tag,
    /// Date range.
    Date,
    /// Payee/narration text.
    Text,
    /// Amount magnitude.
    Amount,
    /// Reconciliation status.
    Status,
}

impl Dimension {
    /// All dimensions, in menu order.
    #[must_use]
    pub fn all() -> [Self; 6] {
        [
            Self::Account,
            Self::Tag,
            Self::Date,
            Self::Text,
            Self::Amount,
            Self::Status,
        ]
    }

    /// Menu label.
    #[must_use]
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(
            dead_code,
            reason = "only rendered by the wasm32-gated CommandPalette view"
        )
    )]
    pub fn label(self) -> &'static str {
        match self {
            Self::Account => "Account",
            Self::Tag => "Tag",
            Self::Date => "Date",
            Self::Text => "Text",
            Self::Amount => "Amount",
            Self::Status => "Status",
        }
    }

    /// Typed prefix (without the colon).
    #[must_use]
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Tag => "tag",
            Self::Date => "date",
            Self::Text => "text",
            Self::Amount => "amount",
            Self::Status => "status",
        }
    }
}

/// Parses a `dimension:remainder` prefix, returning the dimension and the
/// trimmed remainder. Case-insensitive; returns `None` if unrecognised.
#[must_use]
pub fn parse_prefix(input: &str) -> Option<(Dimension, &str)> {
    let (raw_head, rest) = input.split_once(':')?;
    let head = raw_head.trim().to_ascii_lowercase();
    let dim = Dimension::all().into_iter().find(|d| d.prefix() == head)?;
    Some((dim, rest.trim()))
}

/// Which screen of the palette is currently shown.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    /// Dimension menu + prefix-jump search.
    Root,
    /// Value-entry screen for a single dimension.
    Dimension(Dimension),
}

/// Fixed list of reconciliation statuses offered on the Status screen.
#[cfg(target_arch = "wasm32")]
const STATUSES: [Reconciliation; 3] = [
    Reconciliation::Unreconciled,
    Reconciliation::Flagged,
    Reconciliation::Reconciled,
];

/// Command palette modal triggered by ⌘K.
///
/// Renders a full-screen overlay that walks the user through picking a filter
/// dimension (or jumping straight to one via a `dimension:value` prefix) and
/// entering a value for it. Committing a value writes into the
/// [`crate::filter_ctx::FilterStore`] and returns to the root menu. Keyboard
/// navigation (Arrow keys, Enter, Escape) and click-to-select are supported.
///
/// # Arguments
///
/// * `open` - Read signal controlling whether the palette is visible.
/// * `on_close` - Callback invoked when the palette should close.
#[cfg(target_arch = "wasm32")]
#[component]
#[expect(clippy::too_many_lines, reason = "Leptos view! block")]
pub fn CommandPalette(
    /// Whether the palette is visible.
    open: ReadSignal<bool>,
    /// Called when the palette should close (Escape, backdrop click).
    on_close: Callback<()>,
) -> impl IntoView {
    let store = crate::filter_ctx::use_filter_store();

    let screen = RwSignal::new(Screen::Root);
    let query = RwSignal::new(String::new());
    let selected_idx = RwSignal::new(0_usize);
    let amount_min = RwSignal::new(String::new());
    let amount_max = RwSignal::new(String::new());
    let date_from = RwSignal::new(String::new());
    let date_until = RwSignal::new(String::new());
    let input_ref = NodeRef::<leptos::html::Input>::new();

    /* Increment only when opening so closing does not reset the cached lists. */
    let open_count = RwSignal::new(0_usize);
    Effect::new(move |_| {
        if open.get() {
            open_count.update(|n| *n = n.wrapping_add(1));
        }
    });
    let accounts_resource = LocalResource::new(move || async move {
        if open_count.get() == 0 {
            return Ok(vec![]);
        }
        bc_ipc::client::list_accounts().await
    });
    let tags_resource = LocalResource::new(move || async move {
        if open_count.get() == 0 {
            return Ok(vec![]);
        }
        bc_ipc::client::list_tags().await
    });

    /* Root dimension menu, filtered by label substring. */
    let root_items = Memo::new(move |_| {
        let q_lower = query.get().to_lowercase();
        Dimension::all()
            .into_iter()
            .filter(|d| q_lower.is_empty() || d.label().to_lowercase().contains(&q_lower))
            .collect::<Vec<Dimension>>()
    });
    let filtered_accounts = Memo::new(move |_| {
        let q_lower = query.get().to_lowercase();
        accounts_resource
            .get()
            .and_then(Result::ok)
            .unwrap_or_default()
            .into_iter()
            .filter(|a| q_lower.is_empty() || a.name.to_lowercase().contains(&q_lower))
            .collect::<Vec<AccountNode>>()
    });
    let filtered_tags = Memo::new(move |_| {
        let q_lower = query.get().to_lowercase();
        tags_resource
            .get()
            .and_then(Result::ok)
            .unwrap_or_default()
            .into_iter()
            .filter(|t| q_lower.is_empty() || t.path.to_lowercase().contains(&q_lower))
            .collect::<Vec<TagInfo>>()
    });
    let filtered_statuses = Memo::new(move |_| {
        let q_lower = query.get().to_lowercase();
        STATUSES
            .into_iter()
            .filter(|r| q_lower.is_empty() || r.label().contains(&q_lower))
            .collect::<Vec<Reconciliation>>()
    });

    /* Number of navigable rows for the screen currently shown. */
    let list_len = Memo::new(move |_| match screen.get() {
        Screen::Root => root_items.get().len(),
        Screen::Dimension(Dimension::Account) => filtered_accounts.get().len(),
        Screen::Dimension(Dimension::Tag) => filtered_tags.get().len(),
        Screen::Dimension(Dimension::Status) => filtered_statuses.get().len(),
        Screen::Dimension(Dimension::Text | Dimension::Amount | Dimension::Date) => 0,
    });

    /* Resets the search state and returns to the dimension menu. */
    let back_to_root = move || {
        screen.set(Screen::Root);
        query.set(String::new());
        selected_idx.set(0);
    };

    /* Autofocus the input and reset all state whenever the palette opens. */
    Effect::new(move |_| {
        if open.get() {
            screen.set(Screen::Root);
            query.set(String::new());
            selected_idx.set(0);
            amount_min.set(String::new());
            amount_max.set(String::new());
            date_from.set(String::new());
            date_until.set(String::new());
            if let Some(el) = input_ref.get() {
                #[expect(
                    clippy::let_underscore_must_use,
                    clippy::let_underscore_untyped,
                    let_underscore_drop,
                    reason = "focus() returns Result<(), JsValue>; errors are benign"
                )]
                let _ = el.focus();
            }
        }
    });

    /* Commits whichever row is selected on the current list screen. */
    let commit_selected = move || {
        let idx = selected_idx.get();
        match screen.get() {
            Screen::Root => {
                if let Some(dim) = root_items.get().get(idx).copied() {
                    screen.set(Screen::Dimension(dim));
                    query.set(String::new());
                    selected_idx.set(0);
                }
            }
            Screen::Dimension(Dimension::Account) => {
                if let Some(account) = filtered_accounts.get().get(idx).cloned() {
                    store.filter.update(|f| {
                        if !f.accounts.contains(&account.id) {
                            f.accounts.push(account.id);
                        }
                    });
                    back_to_root();
                }
            }
            Screen::Dimension(Dimension::Tag) => {
                if let Some(tag) = filtered_tags.get().get(idx).cloned() {
                    store.filter.update(|f| {
                        if !f.tags.contains(&tag.id) {
                            f.tags.push(tag.id);
                        }
                    });
                    back_to_root();
                }
            }
            Screen::Dimension(Dimension::Status) => {
                if let Some(rec) = filtered_statuses.get().get(idx).copied() {
                    store.filter.update(|f| f.reconciliation = Some(rec));
                    back_to_root();
                }
            }
            Screen::Dimension(Dimension::Text) => {
                let value = query.get().trim().to_owned();
                store.filter.update(|f| {
                    f.text = if value.is_empty() { None } else { Some(value) };
                });
                back_to_root();
            }
            Screen::Dimension(Dimension::Amount | Dimension::Date) => {
                /* Handled by the dedicated form fields' own Enter handlers. */
            }
        }
    };

    /* Commits the min/max amount form. */
    let commit_amount = move || {
        let min = amount_min.get().trim().parse::<Decimal>().ok();
        let max = amount_max.get().trim().parse::<Decimal>().ok();
        store.filter.update(|f| {
            f.amount = if min.is_none() && max.is_none() {
                None
            } else {
                let mut filter = AmountFilter::default();
                filter.min = min;
                filter.max = max;
                Some(filter)
            };
        });
        back_to_root();
    };

    /* Commits the from/until date form. */
    let commit_date = move || {
        let from = date_from.get().trim().parse::<jiff::civil::Date>().ok();
        let until = date_until.get().trim().parse::<jiff::civil::Date>().ok();
        store.filter.update(|f| {
            f.date_from = from;
            f.date_until = until;
        });
        back_to_root();
    };

    /* Shared keydown handler for the single-line screens (Root, Account, Tag, Text, Status). */
    let on_list_keydown = move |e: web_sys::KeyboardEvent| match e.key().as_str() {
        "Escape" => {
            if matches!(screen.get(), Screen::Root) {
                on_close.run(());
            } else {
                back_to_root();
            }
            e.prevent_default();
        }
        "ArrowDown" => {
            let count = list_len.get();
            if count > 0 {
                selected_idx.update(|i| {
                    *i = i.saturating_add(1).min(count.saturating_sub(1));
                });
            }
            e.prevent_default();
        }
        "ArrowUp" => {
            selected_idx.update(|i| {
                *i = i.saturating_sub(1);
            });
            e.prevent_default();
        }
        "Enter" => {
            commit_selected();
            e.prevent_default();
        }
        _ => {}
    };

    view! {
        <Show when=move || open.get()>
            <div class=style::overlay on:click=move |_| on_close.run(())>
                <div
                    class=style::modal
                    role="dialog"
                    aria-label="Command palette"
                    aria-modal="true"
                    on:click=move |e| e.stop_propagation()
                >
                    {move || match screen.get() {
                        Screen::Dimension(Dimension::Amount) => {
                            view! {
                                <div class=style::form>
                                    <label class=style::form_label for="palette-amount-min">
                                        "min"
                                    </label>
                                    <input
                                        id="palette-amount-min"
                                        class=style::input
                                        type="text"
                                        placeholder="min amount"
                                        prop:value=move || amount_min.get()
                                        on:input=move |e| amount_min.set(event_target_value(&e))
                                        on:keydown=move |e: web_sys::KeyboardEvent| {
                                            match e.key().as_str() {
                                                "Enter" => {
                                                    commit_amount();
                                                    e.prevent_default();
                                                }
                                                "Escape" => {
                                                    back_to_root();
                                                    e.prevent_default();
                                                }
                                                _ => {}
                                            }
                                        }
                                    />
                                    <label class=style::form_label for="palette-amount-max">
                                        "max"
                                    </label>
                                    <input
                                        id="palette-amount-max"
                                        class=style::input
                                        type="text"
                                        placeholder="max amount"
                                        prop:value=move || amount_max.get()
                                        on:input=move |e| amount_max.set(event_target_value(&e))
                                        on:keydown=move |e: web_sys::KeyboardEvent| {
                                            match e.key().as_str() {
                                                "Enter" => {
                                                    commit_amount();
                                                    e.prevent_default();
                                                }
                                                "Escape" => {
                                                    back_to_root();
                                                    e.prevent_default();
                                                }
                                                _ => {}
                                            }
                                        }
                                    />
                                </div>
                            }
                                .into_any()
                        }
                        Screen::Dimension(Dimension::Date) => {
                            view! {
                                <div class=style::form>
                                    <label class=style::form_label for="palette-date-from">
                                        "from"
                                    </label>
                                    <input
                                        id="palette-date-from"
                                        class=style::input
                                        type="date"
                                        prop:value=move || date_from.get()
                                        on:input=move |e| date_from.set(event_target_value(&e))
                                        on:keydown=move |e: web_sys::KeyboardEvent| {
                                            match e.key().as_str() {
                                                "Enter" => {
                                                    commit_date();
                                                    e.prevent_default();
                                                }
                                                "Escape" => {
                                                    back_to_root();
                                                    e.prevent_default();
                                                }
                                                _ => {}
                                            }
                                        }
                                    />
                                    <label class=style::form_label for="palette-date-until">
                                        "until"
                                    </label>
                                    <input
                                        id="palette-date-until"
                                        class=style::input
                                        type="date"
                                        prop:value=move || date_until.get()
                                        on:input=move |e| date_until.set(event_target_value(&e))
                                        on:keydown=move |e: web_sys::KeyboardEvent| {
                                            match e.key().as_str() {
                                                "Enter" => {
                                                    commit_date();
                                                    e.prevent_default();
                                                }
                                                "Escape" => {
                                                    back_to_root();
                                                    e.prevent_default();
                                                }
                                                _ => {}
                                            }
                                        }
                                    />
                                </div>
                            }
                                .into_any()
                        }
                        current @ (Screen::Root
                        | Screen::Dimension(
                            Dimension::Account
                            | Dimension::Tag
                            | Dimension::Text
                            | Dimension::Status,
                        )) => {
                            let placeholder = match current {
                                Screen::Root => "Search filters, or type e.g. tag:groceries…",
                                Screen::Dimension(Dimension::Account) => "Search accounts…",
                                Screen::Dimension(Dimension::Tag) => "Search tags…",
                                Screen::Dimension(Dimension::Text) => "Payee or narration text…",
                                Screen::Dimension(Dimension::Status) => "Search status…",
                                Screen::Dimension(Dimension::Amount | Dimension::Date) => {
                                    unreachable!("handled by the earlier match arms")
                                }
                            };
                            let aria_label = match current {
                                Screen::Root => "Search filters",
                                Screen::Dimension(dim) => dim.label(),
                            };
                            view! {
                                <input
                                    node_ref=input_ref
                                    class=style::input
                                    type="text"
                                    role="combobox"
                                    placeholder=placeholder
                                    aria-label=aria_label
                                    aria-expanded=move || open.get()
                                    aria-controls="palette-listbox"
                                    prop:value=move || query.get()
                                    on:input=move |e| {
                                        let val = event_target_value(&e);
                                        if matches!(screen.get(), Screen::Root)
                                            && let Some((dim, rest)) = parse_prefix(&val)
                                        {
                                            screen.set(Screen::Dimension(dim));
                                            query.set(rest.to_owned());
                                            selected_idx.set(0);
                                            return;
                                        }
                                        query.set(val);
                                        selected_idx.set(0);
                                    }
                                    on:keydown=on_list_keydown
                                />
                                <div
                                    id="palette-listbox"
                                    class=style::list
                                    role="listbox"
                                    aria-label=aria_label
                                >
                                    {move || {
                                        let sel = selected_idx.get();
                                        match screen.get() {
                                            Screen::Root => {
                                                let items = root_items.get();
                                                items
                                                    .into_iter()
                                                    .enumerate()
                                                    .map(|(idx, dim)| {
                                                        let item_class = if idx == sel {
                                                            format!("{} {}", style::item, style::item_selected)
                                                        } else {
                                                            style::item.to_owned()
                                                        };
                                                        view! {
                                                            <div
                                                                class=item_class
                                                                role="option"
                                                                aria-selected=idx == sel
                                                                on:click=move |_| {
                                                                    screen.set(Screen::Dimension(dim));
                                                                    query.set(String::new());
                                                                    selected_idx.set(0);
                                                                }
                                                                on:mouseenter=move |_| selected_idx.set(idx)
                                                            >
                                                                <span class=style::item_name>{dim.label()}</span>
                                                                <span class=style::badge>
                                                                    {format!("{}:", dim.prefix())}
                                                                </span>
                                                            </div>
                                                        }
                                                    })
                                                    .collect::<Vec<_>>()
                                                    .into_any()
                                            }
                                            Screen::Dimension(Dimension::Account) => {
                                                let items = filtered_accounts.get();
                                                if items.is_empty() {
                                                    view! { <div class=style::empty>"no accounts found"</div> }
                                                        .into_any()
                                                } else {
                                                    items
                                                        .into_iter()
                                                        .enumerate()
                                                        .map(|(idx, node)| {
                                                            let item_class = if idx == sel {
                                                                format!("{} {}", style::item, style::item_selected)
                                                            } else {
                                                                style::item.to_owned()
                                                            };
                                                            let name = node.name.clone();
                                                            let id = node.id.clone();
                                                            view! {
                                                                <div
                                                                    class=item_class
                                                                    role="option"
                                                                    aria-selected=idx == sel
                                                                    on:click=move |_| {
                                                                        store
                                                                            .filter
                                                                            .update(|f| {
                                                                                if !f.accounts.contains(&id) {
                                                                                    f.accounts.push(id.clone());
                                                                                }
                                                                            });
                                                                        back_to_root();
                                                                    }
                                                                    on:mouseenter=move |_| selected_idx.set(idx)
                                                                >
                                                                    <span class=style::item_name>{name}</span>
                                                                </div>
                                                            }
                                                        })
                                                        .collect::<Vec<_>>()
                                                        .into_any()
                                                }
                                            }
                                            Screen::Dimension(Dimension::Tag) => {
                                                let items = filtered_tags.get();
                                                if items.is_empty() {
                                                    view! { <div class=style::empty>"no tags found"</div> }
                                                        .into_any()
                                                } else {
                                                    items
                                                        .into_iter()
                                                        .enumerate()
                                                        .map(|(idx, tag)| {
                                                            let item_class = if idx == sel {
                                                                format!("{} {}", style::item, style::item_selected)
                                                            } else {
                                                                style::item.to_owned()
                                                            };
                                                            let path = tag.path.clone();
                                                            let id = tag.id.clone();
                                                            view! {
                                                                <div
                                                                    class=item_class
                                                                    role="option"
                                                                    aria-selected=idx == sel
                                                                    on:click=move |_| {
                                                                        store
                                                                            .filter
                                                                            .update(|f| {
                                                                                if !f.tags.contains(&id) {
                                                                                    f.tags.push(id.clone());
                                                                                }
                                                                            });
                                                                        back_to_root();
                                                                    }
                                                                    on:mouseenter=move |_| selected_idx.set(idx)
                                                                >
                                                                    <span class=style::item_name>{path}</span>
                                                                </div>
                                                            }
                                                        })
                                                        .collect::<Vec<_>>()
                                                        .into_any()
                                                }
                                            }
                                            Screen::Dimension(Dimension::Status) => {
                                                let items = filtered_statuses.get();
                                                items
                                                    .into_iter()
                                                    .enumerate()
                                                    .map(|(idx, rec)| {
                                                        let item_class = if idx == sel {
                                                            format!("{} {}", style::item, style::item_selected)
                                                        } else {
                                                            style::item.to_owned()
                                                        };
                                                        view! {
                                                            <div
                                                                class=item_class
                                                                role="option"
                                                                aria-selected=idx == sel
                                                                on:click=move |_| {
                                                                    store.filter.update(|f| f.reconciliation = Some(rec));
                                                                    back_to_root();
                                                                }
                                                                on:mouseenter=move |_| selected_idx.set(idx)
                                                            >
                                                                <span class=style::item_name>{rec.label()}</span>
                                                            </div>
                                                        }
                                                    })
                                                    .collect::<Vec<_>>()
                                                    .into_any()
                                            }
                                            Screen::Dimension(Dimension::Text) => {
                                                view! {
                                                    <div class=style::empty>
                                                        "Press Enter to set the payee/narration filter."
                                                    </div>
                                                }
                                                    .into_any()
                                            }
                                            Screen::Dimension(Dimension::Amount | Dimension::Date) => {
                                                unreachable!("handled by the earlier match arms")
                                            }
                                        }
                                    }}
                                </div>
                            }
                                .into_any()
                        }
                    }}
                </div>
            </div>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::Dimension;
    use super::parse_prefix;

    #[test]
    fn parse_prefix_recognises_dimension_and_remainder() {
        let (dim, rest) = parse_prefix("tag:groc").expect("tag prefix");
        assert_eq!(dim, Dimension::Tag);
        assert_eq!(rest, "groc");
    }

    #[test]
    fn parse_prefix_is_case_insensitive_and_trims() {
        let (dim, rest) = parse_prefix("Date: 2026-01").expect("date prefix");
        assert_eq!(dim, Dimension::Date);
        assert_eq!(rest, "2026-01");
    }

    #[test]
    fn parse_prefix_returns_none_without_recognised_prefix() {
        assert!(parse_prefix("amazon").is_none());
        assert!(parse_prefix("nope:foo").is_none());
    }

    #[test]
    fn selected_idx_clamping_arrow_down_at_last() {
        /* ArrowDown at the last item (idx 2, count 3) stays at 2. */
        let i = 2_usize;
        let count = 3_usize;
        let next = i.saturating_add(1).min(count.saturating_sub(1));
        assert_eq!(next, 2);
    }

    #[test]
    fn selected_idx_clamping_arrow_up_at_first() {
        /* ArrowUp at the first item stays at 0. */
        let i = 0_usize;
        let prev = i.saturating_sub(1);
        assert_eq!(prev, 0);
    }

    #[test]
    fn selected_idx_clamping_empty_list_arrow_down() {
        /* ArrowDown on an empty list — guarded by count > 0 check — is a no-op. */
        let count = 0_usize;
        let i = 0_usize;
        if count > 0 {
            let _next: usize = i.saturating_add(1).min(count.saturating_sub(1));
            panic!("should not reach here when count == 0");
        }
        assert_eq!(i, 0);
    }

    #[test]
    fn selected_idx_clamping_single_item_arrow_down() {
        /* ArrowDown with one item stays at 0. */
        let i = 0_usize;
        let count = 1_usize;
        let next = i.saturating_add(1).min(count.saturating_sub(1));
        assert_eq!(next, 0);
    }
}
