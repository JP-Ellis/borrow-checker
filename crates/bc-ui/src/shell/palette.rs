//! Command palette (⌘K) — inline structured filter search.
//!
//! A single search box. Free text is parsed as query text and joined onto the
//! filter's query by `and`; a recognised `field:value` token writes a query
//! term instead:
//!
//! - `account:` / `tag:` — pick from live suggestions; each pick joins the
//!   field's top-level `or` group;
//! - `status:` — pick from live suggestions; replaces its status family;
//! - `after:` / `before:` — `date:>=` / `date:<` bounds;
//! - `over:` / `under:` — `amount:>=` / `amount:<=` magnitude bounds.
//!
//! Committing (Enter, or clicking a suggestion) writes into the app-wide
//! [`crate::filter_ctx::FilterStore`] and clears the box so several tokens can be
//! added in one session; the query's top-level conjuncts show as removable
//! chips in the top bar.

#[cfg(target_arch = "wasm32")]
use bc_ipc::AccountRef;
use bc_ipc::BalanceStatus;
use bc_ipc::CommodityInfo;
use bc_ipc::Filter;
use bc_ipc::Reconciliation;
#[cfg(target_arch = "wasm32")]
use bc_ipc::TagInfo;
use bc_query::Expr;
use bc_query::ParseError;
use bc_query::ast::Op;
use bc_query::currency::MarkerError;
use bc_query::currency::split_marked_amount;
use bc_query::shape;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::web_sys;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
use crate::components::account_picker::AccountPathLabel;
#[cfg(target_arch = "wasm32")]
use crate::components::account_picker::account_paths;
#[cfg(target_arch = "wasm32")]
use crate::components::account_picker::filter_accounts;
#[cfg(target_arch = "wasm32")]
use crate::components::account_picker::short_account_labels;
use crate::filter_ctx::and_term;
use crate::filter_ctx::replace_term;

#[cfg(target_arch = "wasm32")]
import_style!(style, "palette.module.scss");

/// A structured filter field addressable by a typed `field:` prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// Account subtree (`account:`), pick from suggestions.
    Account,
    /// Tag (`tag:`), pick from suggestions.
    Tag,
    /// Reconciliation or balance status (`status:`), pick from suggestions.
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
/// not a recognised field (the whole input is then free query-language text).
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

/// One entry offered on the `status:` token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusOption {
    /// A reconciliation state: `status:unreconciled`, `flagged` or `reconciled`.
    Reconciliation(Reconciliation),
    /// A balance state: `status:balanced` or `unbalanced`.
    Balance(BalanceStatus),
}

impl StatusOption {
    /// The lowercase label typed after `status:` and shown in the list.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Reconciliation(r) => r.label(),
            Self::Balance(b) => b.label(),
        }
    }

    /// The `status:` term this option commits.
    #[must_use]
    pub fn term(self) -> Expr {
        shape::builtin_term("status", Op::Match, self.label())
    }

    /// Whether `conjunct` is a status term of this option's family, which
    /// committing the option replaces.
    #[must_use]
    pub fn replaces(self, conjunct: &Expr) -> bool {
        match self {
            Self::Reconciliation(_) => shape::is_reconciliation_term(conjunct),
            Self::Balance(_) => shape::is_balance_term(conjunct),
        }
    }
}

/// Every option offered on the `status:` token, in display order.
const STATUS_OPTIONS: [StatusOption; 5] = [
    StatusOption::Reconciliation(Reconciliation::Unreconciled),
    StatusOption::Reconciliation(Reconciliation::Flagged),
    StatusOption::Reconciliation(Reconciliation::Reconciled),
    StatusOption::Balance(BalanceStatus::Balanced),
    StatusOption::Balance(BalanceStatus::Unbalanced),
];

/// Returns the status options whose label contains `query`
/// (case-insensitive), with an exact label match moved to the front so
/// Enter commits the status typed in full.
///
/// # Arguments
///
/// * `query` - The `status:` remainder.
#[must_use]
pub fn status_options(query: &str) -> Vec<StatusOption> {
    let q = query.trim().to_lowercase();
    let (exact, rest): (Vec<StatusOption>, Vec<StatusOption>) = STATUS_OPTIONS
        .into_iter()
        .filter(|o| o.label().contains(&q))
        .partition(|o| o.label() == q);
    exact.into_iter().chain(rest).collect()
}

/// `filter` with `opt` replacing any status of its family.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `opt` - The picked status.
#[must_use]
pub fn status_filter(filter: &Filter, opt: StatusOption) -> Filter {
    replace_term(filter, |c| opt.replaces(c), opt.term())
}

/// `filter` with an `after:` (`date:>=`) or `before:` (`date:<`) bound
/// replacing any earlier bound of the same kind; `None` for other fields.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `field` - `Field::After` or `Field::Before`.
/// * `date` - The bound.
#[must_use]
pub fn date_filter(filter: &Filter, field: Field, date: jiff::civil::Date) -> Option<Filter> {
    let op = match field {
        Field::After => Op::Ge,
        Field::Before => Op::Lt,
        Field::Account | Field::Tag | Field::Status | Field::Over | Field::Under => return None,
    };
    let term = shape::builtin_term("date", op, &date.to_string());
    Some(replace_term(
        filter,
        |c| shape::is_builtin_with(c, "date", op),
        term,
    ))
}

/// `filter` with an `over:` (`amount:>=`) or `under:` (`amount:<=`) bound
/// replacing any earlier bound of the same kind; `None` for other fields.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `field` - `Field::Over` or `Field::Under`.
/// * `commodity` - The resolved commodity code, if a marker was typed.
/// * `value` - The magnitude.
#[must_use]
pub fn amount_filter(
    filter: &Filter,
    field: Field,
    commodity: Option<&str>,
    value: rust_decimal::Decimal,
) -> Option<Filter> {
    let op = match field {
        Field::Over => Op::Ge,
        Field::Under => Op::Le,
        Field::Account | Field::Tag | Field::Status | Field::After | Field::Before => return None,
    };
    let text = commodity.map_or_else(|| value.to_string(), |code| format!("{code}{value}"));
    let term = shape::builtin_term("amount", op, &text);
    Some(replace_term(
        filter,
        |c| shape::is_builtin_with(c, "amount", op),
        term,
    ))
}

/// `filter` with free query text joined on by `and`.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `text` - The typed query text.
///
/// # Errors
///
/// Returns the [`ParseError`] when `text` does not parse; nothing is committed.
pub fn text_filter(filter: &Filter, text: &str) -> Result<Filter, ParseError> {
    Ok(and_term(filter, bc_query::parse(text.trim())?))
}

/// Command palette modal triggered by ⌘K.
///
/// Renders a full-screen overlay with a single search input that builds the
/// app-wide filter inline. Recognised `field:value` tokens drive live
/// suggestions or scalar entry; free text is parsed as query text and joined
/// to the filter with `and`. Keyboard navigation (Arrow keys, Enter, Escape)
/// and click-to-select are supported.
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

    /* Qualified paths, and each account's short chip label, rebuilt only when the list reloads. */
    let account_refs = Memo::new(move |_| {
        account_paths(
            &accounts_resource
                .get()
                .and_then(Result::ok)
                .unwrap_or_default(),
        )
    });
    let short_labels = Memo::new(move |_| short_account_labels(&account_refs.get()));

    /* Live suggestion lists, filtered by the token remainder. */
    let filtered_accounts = Memo::new(move |_| {
        let Some((Field::Account, q)) = parsed.get() else {
            return Vec::new();
        };
        filter_accounts(&account_refs.get(), &q)
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
        status_options(&q)
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

    /* Adds an account term labelled by its shortest unique path suffix. */
    let pick_account = move |account: AccountRef| {
        let short = short_labels
            .get_untracked()
            .get(&account.id)
            .cloned()
            .unwrap_or_else(|| account.name.clone());
        store.add_account(
            &account.name.replace(" :: ", ":"),
            &short.replace(" :: ", ":"),
        );
        reset_query();
    };

    /* Commits the current query into the filter store. */
    let commit = move || match parse_token(&query.get()) {
        Some((Field::Account, _)) => {
            if let Some(account) = filtered_accounts.get().get(selected_idx.get()).cloned() {
                pick_account(account);
            }
        }
        Some((Field::Tag, _)) => {
            if let Some(tag) = filtered_tags.get().get(selected_idx.get()).cloned() {
                store.add_tag(&tag.path);
                reset_query();
            }
        }
        Some((Field::Status, _)) => {
            if let Some(opt) = filtered_statuses.get().get(selected_idx.get()).copied() {
                store.filter.update(|f| *f = status_filter(f, opt));
                reset_query();
            }
        }
        Some((field @ (Field::After | Field::Before), rest)) => {
            if let Ok(date) = rest.parse::<jiff::civil::Date>()
                && let Some(next) = store.filter.with(|f| date_filter(f, field, date))
            {
                store.filter.set(next);
                reset_query();
            }
        }
        Some((field @ (Field::Over | Field::Under), rest)) => {
            if let Some((commodity, value)) = parse_amount(&currencies.get(), rest)
                && let Some(next) = store
                    .filter
                    .with(|f| amount_filter(f, field, commodity.as_deref(), value))
            {
                store.filter.set(next);
                reset_query();
            }
        }
        None => {
            let text = query.get();
            if !text.trim().is_empty()
                && let Ok(next) = store.filter.with(|f| text_filter(f, &text))
            {
                store.filter.set(next);
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
                                Some((Field::Account, q)) => {
                                    let items = filtered_accounts.get();
                                    if items.is_empty() {
                                        view! { <div class=style::empty>"no accounts found"</div> }
                                            .into_any()
                                    } else {
                                        items
                                            .into_iter()
                                            .enumerate()
                                            .map(|(idx, account)| {
                                                let item_class = if idx == sel {
                                                    format!("{} {}", style::item, style::item_selected)
                                                } else {
                                                    style::item.to_owned()
                                                };
                                                let path = account.name.clone();
                                                view! {
                                                    <div
                                                        class=item_class
                                                        id=format!("palette-opt-{idx}")
                                                        role="option"
                                                        aria-selected=idx == sel
                                                        on:click=move |_| pick_account(account.clone())
                                                        on:mouseenter=move |_| selected_idx.set(idx)
                                                    >
                                                        <span class=style::item_name>
                                                            <AccountPathLabel path=path query=q.clone() />
                                                        </span>
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
                                                view! {
                                                    <div
                                                        class=item_class
                                                        id=format!("palette-opt-{idx}")
                                                        role="option"
                                                        aria-selected=idx == sel
                                                        on:click=move |_| {
                                                            store.add_tag(&path);
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
                                        .map(|(idx, opt)| {
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
                                                        store.filter.update(|f| *f = status_filter(f, opt));
                                                        reset_query();
                                                    }
                                                    on:mouseenter=move |_| selected_idx.set(idx)
                                                >
                                                    <span class=style::item_name>{opt.label()}</span>
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
                                    } else if let Err(error) = bc_query::parse(q.trim()) {
                                        view! { <div class=style::empty>{error.message}</div> }
                                            .into_any()
                                    } else {
                                        view! {
                                            <div class=style::empty>
                                                {format!("↵ add “{}” to the filter", q.trim())}
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
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::BalanceStatus;
    use bc_ipc::CommodityInfo;
    use bc_ipc::Filter;
    use bc_ipc::Reconciliation;
    use pretty_assertions::assert_eq;

    use super::Field;
    use super::StatusOption;
    use super::amount_filter;
    use super::date_filter;
    use super::parse_amount;
    use super::parse_token;
    use super::status_filter;
    use super::status_options;
    use super::text_filter;

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
    fn status_options_offer_all_five_on_an_empty_remainder() {
        let labels: Vec<_> = status_options("")
            .into_iter()
            .map(StatusOption::label)
            .collect();
        assert_eq!(
            labels,
            vec![
                "unreconciled",
                "flagged",
                "reconciled",
                "balanced",
                "unbalanced"
            ]
        );
    }

    #[test]
    fn status_options_rank_the_exact_label_first() {
        /* `balanced` is a substring of `unbalanced`; Enter must commit the one typed. */
        assert_eq!(
            status_options("balanced").first().copied(),
            Some(StatusOption::Balance(BalanceStatus::Balanced))
        );
        assert_eq!(
            status_options("Reconciled").first().copied(),
            Some(StatusOption::Reconciliation(Reconciliation::Reconciled))
        );
        assert_eq!(status_options("bal").len(), 2);
    }

    #[test]
    fn a_status_replaces_only_its_own_family() {
        let unreconciled = status_filter(
            &Filter::default(),
            StatusOption::Reconciliation(Reconciliation::Unreconciled),
        );
        let balanced = status_filter(
            &unreconciled,
            StatusOption::Balance(BalanceStatus::Balanced),
        );
        let unbalanced = status_filter(&balanced, StatusOption::Balance(BalanceStatus::Unbalanced));
        assert_eq!(unbalanced.query, "status:unreconciled status:unbalanced");
    }

    #[test]
    fn date_and_amount_tokens_write_new_grammar() {
        let after = date_filter(
            &Filter::default(),
            Field::After,
            jiff::civil::date(2026, 3, 1),
        )
        .expect("after");
        let before =
            date_filter(&after, Field::Before, jiff::civil::date(2026, 4, 1)).expect("before");
        let dated = date_filter(&before, Field::After, jiff::civil::date(2026, 3, 15))
            .expect("after again");
        assert_eq!(dated.query, "date:<2026-04-01 date:>=2026-03-15");
        let over = amount_filter(
            &dated,
            Field::Over,
            Some("USD"),
            "300".parse().expect("decimal"),
        )
        .expect("over");
        let under = amount_filter(&over, Field::Under, None, "500".parse().expect("decimal"))
            .expect("under");
        assert_eq!(
            under.query,
            "date:<2026-04-01 date:>=2026-03-15 amount:>=USD300 amount:<=500"
        );
        assert_eq!(
            date_filter(&under, Field::Tag, jiff::civil::date(2026, 1, 1)),
            None
        );
    }

    #[test]
    fn free_text_is_parsed_and_joined() {
        let payee = text_filter(&Filter::default(), "  @payee:cafe  ").expect("parses");
        assert_eq!(payee.query, "@payee:cafe");
        let words = text_filter(&payee, "coffee shop").expect("parses");
        assert_eq!(words.query, "@payee:cafe coffee shop");
        text_filter(&words, "(open").expect_err("an unclosed group does not parse");
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
