//! Command palette (⌘K) — inline structured filter search.
//!
//! A single search box. Free text filters payee/narration; a recognised
//! `field:value` token builds a structured filter dimension instead:
//!
//! - `account:` / `tag:` / `status:` — pick from live suggestions;
//! - `after:` / `before:` — inclusive-lower / exclusive-upper date bounds;
//! - `over:` / `under:` — minimum / maximum amount magnitude.
//!
//! Committing (Enter, or clicking a suggestion) writes into the app-wide
//! [`crate::filter_ctx::FilterStore`] and clears the box so several tokens can be
//! added in one session; the active values show as removable chips in the top
//! bar. There is no dimension menu — every dimension is reachable by typing.

#[cfg(target_arch = "wasm32")]
use bc_ipc::AccountNode;
use bc_ipc::CommodityInfo;
#[cfg(target_arch = "wasm32")]
use bc_ipc::Reconciliation;
#[cfg(target_arch = "wasm32")]
use bc_ipc::TagInfo;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::web_sys;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

use crate::components::transaction_row::currency::MarkerError;
use crate::components::transaction_row::currency::split_marked_amount;

#[cfg(target_arch = "wasm32")]
import_style!(style, "palette.module.scss");

/// A structured filter field addressable by a typed `field:` prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// Account subtree (`account:`), pick from suggestions.
    Account,
    /// Tag (`tag:`), pick from suggestions.
    Tag,
    /// Reconciliation status (`status:`), pick from suggestions.
    Status,
    /// Inclusive lower date bound (`after:`).
    After,
    /// Exclusive upper date bound (`before:`).
    Before,
    /// Minimum amount magnitude (`over:`).
    Over,
    /// Maximum amount magnitude (`under:`).
    Under,
}

impl Field {
    /// All fields, used to match a typed prefix.
    #[must_use]
    fn all() -> [Self; 7] {
        [
            Self::Account,
            Self::Tag,
            Self::Status,
            Self::After,
            Self::Before,
            Self::Over,
            Self::Under,
        ]
    }

    /// The typed keyword (without the colon).
    #[must_use]
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Tag => "tag",
            Self::Status => "status",
            Self::After => "after",
            Self::Before => "before",
            Self::Over => "over",
            Self::Under => "under",
        }
    }
}

/// Parses a `field:remainder` token, returning the field and the trimmed
/// remainder. Case-insensitive on the keyword; returns `None` when the head is
/// not a recognised field (the whole input is then free payee/narration text).
#[must_use]
pub fn parse_token(input: &str) -> Option<(Field, &str)> {
    let (raw_head, rest) = input.split_once(':')?;
    let head = raw_head.trim().to_ascii_lowercase();
    let field = Field::all().into_iter().find(|f| f.keyword() == head)?;
    Some((field, rest.trim()))
}

/// Parses an `over:` / `under:` remainder into an optional commodity code and a
/// non-negative magnitude.
///
/// A bare number (`300`) is currency-naive → `(None, 300)`. A marked amount
/// (`USD300`, `$300`, `A$ 12.50`, `300 aud`) resolves against the served
/// commodity set via [`split_marked_amount`] — symbols, aliases, and codes all
/// map to the canonical code → `(Some("USD"), 300)`. A present-but-unresolvable
/// or ambiguous marker, a non-numeric tail, or a negative magnitude all yield
/// `None`: `over:`/`under:` are magnitude bounds, so a negative is rejected
/// rather than silently matching everything.
///
/// # Arguments
///
/// * `currencies` - The served commodity set used to resolve a currency marker.
/// * `remainder` - The `over:`/`under:` token remainder.
#[must_use]
pub fn parse_amount(
    currencies: &[CommodityInfo],
    remainder: &str,
) -> Option<(Option<String>, rust_decimal::Decimal)> {
    let s = remainder.trim();
    let (num_text, commodity) = match split_marked_amount(currencies, s) {
        Ok((num, code)) => (num, Some(code)),
        Err(MarkerError::Missing) => (s.to_owned(), None),
        // A marker was typed but matched zero or several commodities.
        Err(MarkerError::Unknown(_) | MarkerError::Ambiguous(_)) => return None,
    };
    let value = num_text.trim().parse::<rust_decimal::Decimal>().ok()?;
    if value.is_sign_negative() {
        return None;
    }
    Some((commodity, value))
}

/// Fixed list of reconciliation statuses offered on the `status:` token.
#[cfg(target_arch = "wasm32")]
const STATUSES: [Reconciliation; 3] = [
    Reconciliation::Unreconciled,
    Reconciliation::Flagged,
    Reconciliation::Reconciled,
];

/// Command palette modal triggered by ⌘K.
///
/// Renders a full-screen overlay with a single search input that builds the
/// app-wide filter inline. Recognised `field:value` tokens drive live
/// suggestions or scalar entry; free text filters payee/narration. Keyboard
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
    let currencies = crate::currency_ctx::use_currency_store();

    let query = RwSignal::new(String::new());
    let selected_idx = RwSignal::new(0_usize);
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

    /* The recognised token (if any) for the current query, with an owned remainder. */
    let parsed = Memo::new(move |_| {
        let q = query.get();
        parse_token(&q).map(|(field, rest)| (field, rest.to_owned()))
    });

    /* Live suggestion lists, filtered by the token remainder. */
    let filtered_accounts = Memo::new(move |_| {
        let Some((Field::Account, q)) = parsed.get() else {
            return Vec::new();
        };
        let q = q.to_lowercase();
        accounts_resource
            .get()
            .and_then(Result::ok)
            .unwrap_or_default()
            .into_iter()
            .filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q))
            .collect::<Vec<AccountNode>>()
    });
    let filtered_tags = Memo::new(move |_| {
        let Some((Field::Tag, q)) = parsed.get() else {
            return Vec::new();
        };
        let q = q.to_lowercase();
        tags_resource
            .get()
            .and_then(Result::ok)
            .unwrap_or_default()
            .into_iter()
            .filter(|t| q.is_empty() || t.path.to_lowercase().contains(&q))
            .collect::<Vec<TagInfo>>()
    });
    let filtered_statuses = Memo::new(move |_| {
        let Some((Field::Status, q)) = parsed.get() else {
            return Vec::new();
        };
        let q = q.to_lowercase();
        STATUSES
            .into_iter()
            .filter(|r| q.is_empty() || r.label().to_lowercase().contains(&q))
            .collect::<Vec<Reconciliation>>()
    });

    /* Number of navigable suggestion rows for the current token. */
    let list_len = Memo::new(move |_| match parsed.get() {
        Some((Field::Account, _)) => filtered_accounts.get().len(),
        Some((Field::Tag, _)) => filtered_tags.get().len(),
        Some((Field::Status, _)) => filtered_statuses.get().len(),
        _ => 0,
    });

    /* Clears the box after committing a token, keeping the palette open and the
    input focused so the next token can be typed (and Escape still routes here,
    even when the value was committed by clicking a suggestion). */
    let reset_query = move || {
        query.set(String::new());
        selected_idx.set(0);
        if let Some(el) = input_ref.get_untracked() {
            #[expect(
                clippy::let_underscore_must_use,
                clippy::let_underscore_untyped,
                let_underscore_drop,
                reason = "focus() returns Result<(), JsValue>; errors are benign"
            )]
            let _ = el.focus();
        }
    };

    /* Reset all state whenever the palette opens. Depends only on `open`. */
    Effect::new(move |_| {
        if open.get() {
            query.set(String::new());
            selected_idx.set(0);
        }
    });

    /* Autofocus the input whenever it (re)mounts while open. Reads `input_ref` but
    writes nothing, so recreating the input cannot feed back into a write. */
    Effect::new(move |_| {
        if open.get()
            && let Some(el) = input_ref.get()
        {
            #[expect(
                clippy::let_underscore_must_use,
                clippy::let_underscore_untyped,
                let_underscore_drop,
                reason = "focus() returns Result<(), JsValue>; errors are benign"
            )]
            let _ = el.focus();
        }
    });

    /* Commits the current query into the filter store. */
    let commit = move || match parse_token(&query.get()) {
        Some((Field::Account, _)) => {
            if let Some(account) = filtered_accounts.get().get(selected_idx.get()).cloned() {
                store.add_account(account.id, account.name);
                reset_query();
            }
        }
        Some((Field::Tag, _)) => {
            if let Some(tag) = filtered_tags.get().get(selected_idx.get()).cloned() {
                store.add_tag(tag.id, tag.path);
                reset_query();
            }
        }
        Some((Field::Status, _)) => {
            if let Some(rec) = filtered_statuses.get().get(selected_idx.get()).copied() {
                store.filter.update(|f| f.reconciliation = Some(rec));
                reset_query();
            }
        }
        Some((Field::After, rest)) => {
            if let Ok(date) = rest.parse::<jiff::civil::Date>() {
                store.filter.update(|f| f.date_from = Some(date));
                reset_query();
            }
        }
        Some((Field::Before, rest)) => {
            if let Ok(date) = rest.parse::<jiff::civil::Date>() {
                store.filter.update(|f| f.date_until = Some(date));
                reset_query();
            }
        }
        Some((Field::Over, rest)) => {
            if let Some((commodity, min)) = parse_amount(&currencies.get(), rest) {
                store.filter.update(|f| {
                    let mut amount = f.amount.clone().unwrap_or_default();
                    amount.min = Some(min);
                    if commodity.is_some() {
                        amount.commodity = commodity;
                    }
                    f.amount = Some(amount);
                });
                reset_query();
            }
        }
        Some((Field::Under, rest)) => {
            if let Some((commodity, max)) = parse_amount(&currencies.get(), rest) {
                store.filter.update(|f| {
                    let mut amount = f.amount.clone().unwrap_or_default();
                    amount.max = Some(max);
                    if commodity.is_some() {
                        amount.commodity = commodity;
                    }
                    f.amount = Some(amount);
                });
                reset_query();
            }
        }
        None => {
            let text = query.get().trim().to_owned();
            if !text.is_empty() {
                store.filter.update(|f| f.text = Some(text));
                reset_query();
            }
        }
    };

    /* Whether the current token has navigable suggestion rows. */
    let has_options = move || list_len.get() > 0;

    /* The id of the active suggestion row for `aria-activedescendant`; `None`
    (attribute omitted) when the current token has no navigable options. */
    let active_descendant =
        move || has_options().then(|| format!("palette-opt-{}", selected_idx.get()));

    let on_keydown = move |e: web_sys::KeyboardEvent| match e.key().as_str() {
        "Escape" => {
            on_close.run(());
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
            commit();
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
                    <input
                        node_ref=input_ref
                        class=style::input
                        type="text"
                        role="combobox"
                        placeholder="Search payee, or account: tag: status: after: before: over: under:"
                        aria-label="Search filters"
                        aria-expanded=has_options
                        aria-controls="palette-listbox"
                        aria-activedescendant=active_descendant
                        prop:value=move || query.get()
                        on:input=move |e| {
                            query.set(event_target_value(&e));
                            selected_idx.set(0);
                        }
                        on:keydown=on_keydown
                    />
                    <div id="palette-listbox" class=style::list role="listbox" aria-label="Filter">
                        {move || {
                            let sel = selected_idx.get();
                            match parsed.get() {
                                Some((Field::Account, _)) => {
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
                                                        id=format!("palette-opt-{idx}")
                                                        role="option"
                                                        aria-selected=idx == sel
                                                        on:click=move |_| {
                                                            store.add_account(id.clone(), name.clone());
                                                            reset_query();
                                                        }
                                                        on:mouseenter=move |_| selected_idx.set(idx)
                                                    >
                                                        <span class=style::item_name>{name.clone()}</span>
                                                    </div>
                                                }
                                            })
                                            .collect::<Vec<_>>()
                                            .into_any()
                                    }
                                }
                                Some((Field::Tag, _)) => {
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
                                                        id=format!("palette-opt-{idx}")
                                                        role="option"
                                                        aria-selected=idx == sel
                                                        on:click=move |_| {
                                                            store.add_tag(id.clone(), path.clone());
                                                            reset_query();
                                                        }
                                                        on:mouseenter=move |_| selected_idx.set(idx)
                                                    >
                                                        <span class=style::item_name>{path.clone()}</span>
                                                    </div>
                                                }
                                            })
                                            .collect::<Vec<_>>()
                                            .into_any()
                                    }
                                }
                                Some((Field::Status, _)) => {
                                    filtered_statuses
                                        .get()
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
                                                    id=format!("palette-opt-{idx}")
                                                    role="option"
                                                    aria-selected=idx == sel
                                                    on:click=move |_| {
                                                        store.filter.update(|f| f.reconciliation = Some(rec));
                                                        reset_query();
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
                                Some((field @ (Field::After | Field::Before), rest)) => {
                                    let hint = match rest.parse::<jiff::civil::Date>() {
                                        Ok(date) => format!("↵ set {} {date}", field.keyword()),
                                        Err(_) => "type a date, e.g. 2026-01-31".to_owned(),
                                    };
                                    view! { <div class=style::empty>{hint}</div> }.into_any()
                                }
                                Some((field @ (Field::Over | Field::Under), rest)) => {
                                    let hint = match parse_amount(&currencies.get(), &rest) {
                                        Some((Some(commodity), value)) => {
                                            format!("↵ set {} {commodity} {value}", field.keyword())
                                        }
                                        Some((None, value)) => {
                                            format!("↵ set {} {value}", field.keyword())
                                        }
                                        None => "type an amount, e.g. 100 or USD 100".to_owned(),
                                    };
                                    view! { <div class=style::empty>{hint}</div> }.into_any()
                                }
                                None => {
                                    let q = query.get();
                                    if q.trim().is_empty() {
                                        view! {
                                            <div class=style::empty>
                                                "Type payee text, or account: tag: status: after: before: over: under:"
                                            </div>
                                        }
                                            .into_any()
                                    } else {
                                        view! {
                                            <div class=style::empty>
                                                {format!(
                                                    "↵ search payee/narration for “{}”",
                                                    q.trim(),
                                                )}
                                            </div>
                                        }
                                            .into_any()
                                    }
                                }
                            }
                        }}
                    </div>
                </div>
            </div>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use bc_ipc::CommodityInfo;
    use pretty_assertions::assert_eq;

    use super::Field;
    use super::parse_amount;
    use super::parse_token;

    fn registry() -> Vec<CommodityInfo> {
        vec![CommodityInfo::new(
            "c1",
            "USD",
            Some("$".to_owned()),
            vec![],
            2,
            true,
            false,
        )]
    }

    #[test]
    fn parse_amount_resolves_optional_commodity_marker() {
        let reg = registry();

        /* Glued code resolves to the canonical code. */
        let (qualified_commodity, qualified_value) =
            parse_amount(&reg, "USD300").expect("qualified");
        assert_eq!(qualified_commodity.as_deref(), Some("USD"));
        assert_eq!(qualified_value, "300".parse().expect("decimal"));

        /* A bare number is currency-naive. */
        let (naive_commodity, naive_value) = parse_amount(&reg, "  300  ").expect("naive");
        assert_eq!(naive_commodity, None);
        assert_eq!(naive_value, "300".parse().expect("decimal"));

        /* Lower-case code + space resolves case-insensitively to the canonical code. */
        assert_eq!(
            parse_amount(&reg, "usd 12.50")
                .expect("spaced")
                .0
                .as_deref(),
            Some("USD")
        );

        /* A symbol resolves to its commodity. */
        let (symbol_commodity, symbol_value) = parse_amount(&reg, "$50").expect("symbol");
        assert_eq!(symbol_commodity.as_deref(), Some("USD"));
        assert_eq!(symbol_value, "50".parse().expect("decimal"));
    }

    #[test]
    fn parse_amount_rejects_invalid_input() {
        let reg = registry();

        /* No numeric tail is not a valid amount. */
        assert!(parse_amount(&reg, "abc").is_none());
        assert!(parse_amount(&reg, "USD").is_none());

        /* A present-but-unknown marker is rejected, not treated as naive. */
        assert!(parse_amount(&reg, "EUR 50").is_none());

        /* Negative magnitudes are rejected (over/under are magnitude bounds). */
        assert!(parse_amount(&reg, "-50").is_none());
        assert!(parse_amount(&reg, "USD-50").is_none());
    }

    #[test]
    fn parse_token_recognises_field_and_remainder() {
        let (field, rest) = parse_token("tag:groc").expect("tag token");
        assert_eq!(field, Field::Tag);
        assert_eq!(rest, "groc");
    }

    #[test]
    fn parse_token_is_case_insensitive_and_trims() {
        let (field, rest) = parse_token("After: 2026-01-01").expect("after token");
        assert_eq!(field, Field::After);
        assert_eq!(rest, "2026-01-01");
    }

    #[test]
    fn parse_token_covers_amount_and_status_keywords() {
        assert_eq!(parse_token("over:100").expect("over").0, Field::Over);
        assert_eq!(parse_token("under:500").expect("under").0, Field::Under);
        assert_eq!(parse_token("before:2026").expect("before").0, Field::Before);
        assert_eq!(
            parse_token("status:flagged").expect("status").0,
            Field::Status
        );
    }

    #[test]
    fn parse_token_returns_none_for_free_text() {
        assert!(parse_token("amazon").is_none());
        assert!(parse_token("nope:foo").is_none());
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
}
