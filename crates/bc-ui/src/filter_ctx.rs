//! Global filter store: the active `Filter` plus a presentation-only strictness
//! toggle, provided once at the shell root. Chip derivation is pure.

use std::collections::HashMap;

/// How much of a partially-matching transaction consumers render.
#[cfg_attr(
    not(target_arch = "wasm32"),
    expect(dead_code, reason = "only read by the wasm32-gated FilterStore")
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Strictness {
    /// Show the whole transaction, greying out non-matching legs.
    #[default]
    Lenient,
    /// Hide non-matching legs (may render an unbalanced transaction).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "wired into the strictness toggle UI in a later task"
        )
    )]
    Strict,
}

impl Strictness {
    /// Human-readable label for the toggle.
    #[must_use]
    #[expect(
        dead_code,
        reason = "wired into the strictness toggle UI in a later task"
    )]
    pub fn label(self) -> &'static str {
        match self {
            Self::Lenient => "lenient",
            Self::Strict => "strict",
        }
    }
}

/// Identifies the single filter value a chip removes when dismissed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChipRemove {
    /// Clears the `after:` (inclusive lower) date bound.
    DateFrom,
    /// Clears the `before:` (exclusive upper) date bound.
    DateUntil,
    /// Clears the payee/narration text needle.
    Text,
    /// Clears the `over:` (minimum magnitude) amount bound.
    AmountMin,
    /// Clears the `under:` (maximum magnitude) amount bound.
    AmountMax,
    /// Clears the reconciliation status.
    Status,
    /// Removes one selected account by id.
    Account(String),
    /// Removes one selected tag by id.
    Tag(String),
}

/// One active filter value rendered as a removable chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    /// Stable key for the `<For>` list (unique per active value).
    pub key: String,
    /// Display text, e.g. `account: Checking` or `over: 100`.
    pub label: String,
    /// Which filter value this chip removes.
    pub remove: ChipRemove,
}

/// Derives one removable chip per active filter value. Account and tag ids are
/// resolved to display names via `names` (populated as the user picks them),
/// falling back to the raw id when a name is not known.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `names` - Map of account/tag id to display label.
#[must_use]
pub fn chips_from_filter(filter: &bc_ipc::Filter, names: &HashMap<String, String>) -> Vec<Chip> {
    let mut chips = Vec::new();

    for id in &filter.accounts {
        let name = names.get(id).cloned().unwrap_or_else(|| id.clone());
        chips.push(Chip {
            key: format!("account:{id}"),
            label: format!("account: {name}"),
            remove: ChipRemove::Account(id.clone()),
        });
    }
    for id in &filter.tags {
        let name = names.get(id).cloned().unwrap_or_else(|| id.clone());
        chips.push(Chip {
            key: format!("tag:{id}"),
            label: format!("tag: {name}"),
            remove: ChipRemove::Tag(id.clone()),
        });
    }
    if let Some(text) = &filter.text {
        chips.push(Chip {
            key: "text".to_owned(),
            label: format!("text: {text}"),
            remove: ChipRemove::Text,
        });
    }
    if let Some(after) = filter.date_from {
        chips.push(Chip {
            key: "after".to_owned(),
            label: format!("after: {after}"),
            remove: ChipRemove::DateFrom,
        });
    }
    if let Some(before) = filter.date_until {
        chips.push(Chip {
            key: "before".to_owned(),
            label: format!("before: {before}"),
            remove: ChipRemove::DateUntil,
        });
    }
    if let Some(amount) = &filter.amount {
        /* A set commodity restricts the whole amount predicate, so it shows on
        both bound chips (e.g. `over: USD 300`). */
        let commodity = amount.commodity.as_deref();
        if let Some(min) = amount.min {
            chips.push(Chip {
                key: "over".to_owned(),
                label: match commodity {
                    Some(c) => format!("over: {c} {min}"),
                    None => format!("over: {min}"),
                },
                remove: ChipRemove::AmountMin,
            });
        }
        if let Some(max) = amount.max {
            chips.push(Chip {
                key: "under".to_owned(),
                label: match commodity {
                    Some(c) => format!("under: {c} {max}"),
                    None => format!("under: {max}"),
                },
                remove: ChipRemove::AmountMax,
            });
        }
    }
    if let Some(rec) = filter.reconciliation {
        chips.push(Chip {
            key: "status".to_owned(),
            label: format!("status: {}", rec.label()),
            remove: ChipRemove::Status,
        });
    }
    chips
}

/// Returns `true` when any filter dimension is set. Mirrors the visibility of
/// [`chips_from_filter`] (an empty filter yields no chips and is inactive), but
/// avoids allocating the chip list.
///
/// # Arguments
///
/// * `filter` - The filter to inspect.
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        dead_code,
        reason = "consumed by register filter wiring in a later task"
    )
)]
#[must_use]
pub fn filter_is_active(filter: &bc_ipc::Filter) -> bool {
    filter.date_from.is_some()
        || filter.date_until.is_some()
        || !filter.accounts.is_empty()
        || !filter.tags.is_empty()
        || filter.text.is_some()
        || filter.amount.is_some()
        || filter.reconciliation.is_some()
}

/// Signal-backed pieces of the filter store; kept in a submodule so only its
/// `RwSignal`/`provide_context` internals are gated on `wasm32`, while the
/// pure `Strictness`/`Chip`/`chips_from_filter` above stay natively testable.
#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::collections::HashMap;

    use leptos::prelude::*;

    use super::ChipRemove;
    use super::Strictness;

    /// Reactive global filter state, provided once at the shell root.
    #[derive(Clone, Copy)]
    pub struct FilterStore {
        /// The active filter.
        pub filter: RwSignal<bc_ipc::Filter>,
        /// Display labels for the account/tag ids in `filter`, recorded as the
        /// user picks them so chips resolve names without a round-trip.
        pub labels: RwSignal<HashMap<String, String>>,
        /// Presentation-only strictness toggle.
        #[expect(
            dead_code,
            reason = "wired into the strictness toggle UI in a later task"
        )]
        pub strictness: RwSignal<Strictness>,
    }

    impl FilterStore {
        /// Adds an account to the filter (no-op if already present), recording its
        /// display name for chip rendering.
        ///
        /// # Arguments
        ///
        /// * `id` - The account id.
        /// * `name` - The account display name.
        pub fn add_account(&self, id: String, name: String) {
            self.labels.update(|m| {
                m.insert(id.clone(), name);
            });
            self.filter.update(|f| {
                if !f.accounts.contains(&id) {
                    f.accounts.push(id);
                }
            });
        }

        /// Adds a tag to the filter (no-op if already present), recording its
        /// display path for chip rendering.
        ///
        /// # Arguments
        ///
        /// * `id` - The tag id.
        /// * `path` - The tag colon-path.
        pub fn add_tag(&self, id: String, path: String) {
            self.labels.update(|m| {
                m.insert(id.clone(), path);
            });
            self.filter.update(|f| {
                if !f.tags.contains(&id) {
                    f.tags.push(id);
                }
            });
        }

        /// Removes the single filter value identified by `target`.
        ///
        /// # Arguments
        ///
        /// * `target` - Which filter value to clear.
        pub fn remove_chip(&self, target: &ChipRemove) {
            self.filter.update(|f| match target {
                ChipRemove::DateFrom => f.date_from = None,
                ChipRemove::DateUntil => f.date_until = None,
                ChipRemove::Text => f.text = None,
                ChipRemove::Status => f.reconciliation = None,
                ChipRemove::Account(id) => f.accounts.retain(|a| a != id),
                ChipRemove::Tag(id) => f.tags.retain(|t| t != id),
                ChipRemove::AmountMin => {
                    if let Some(a) = f.amount.as_mut() {
                        a.min = None;
                    }
                    drop_empty_amount(f);
                }
                ChipRemove::AmountMax => {
                    if let Some(a) = f.amount.as_mut() {
                        a.max = None;
                    }
                    drop_empty_amount(f);
                }
            });
        }
    }

    /// Drops the amount predicate entirely once neither bound remains set. A
    /// commodity alone is not a magnitude filter, so it is dropped with the
    /// bounds rather than lingering as an invisible active predicate.
    fn drop_empty_amount(f: &mut bc_ipc::Filter) {
        if let Some(a) = f.amount.as_ref()
            && a.min.is_none()
            && a.max.is_none()
        {
            f.amount = None;
        }
    }

    /// Provides an empty [`FilterStore`] into context. Call once at the shell root.
    ///
    /// # Returns
    ///
    /// The provided [`FilterStore`] handle.
    #[must_use]
    pub fn provide_filter_store() -> FilterStore {
        let store = FilterStore {
            filter: RwSignal::new(bc_ipc::Filter::default()),
            labels: RwSignal::new(HashMap::new()),
            strictness: RwSignal::new(Strictness::default()),
        };
        provide_context(store);
        store
    }

    /// Reads the [`FilterStore`] from context (creating a detached default if absent).
    ///
    /// # Returns
    ///
    /// The [`FilterStore`] handle from context, or a fresh detached one.
    #[must_use]
    pub fn use_filter_store() -> FilterStore {
        use_context::<FilterStore>().unwrap_or_else(|| FilterStore {
            filter: RwSignal::new(bc_ipc::Filter::default()),
            labels: RwSignal::new(HashMap::new()),
            strictness: RwSignal::new(Strictness::default()),
        })
    }
}

#[cfg(target_arch = "wasm32")]
#[expect(
    unused_imports,
    reason = "FilterStore is consumed by per-view filter surfaces in a later task"
)]
pub use wasm::FilterStore;
#[cfg(target_arch = "wasm32")]
pub use wasm::provide_filter_store;
#[cfg(target_arch = "wasm32")]
pub use wasm::use_filter_store;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use pretty_assertions::assert_eq;

    use super::ChipRemove;
    use super::chips_from_filter;

    #[test]
    fn empty_filter_has_no_chips() {
        let chips = chips_from_filter(&bc_ipc::Filter::default(), &HashMap::new());
        assert!(chips.is_empty());
    }

    #[test]
    fn each_account_and_tag_becomes_its_own_named_chip() {
        // `bc_ipc::Filter` is `#[non_exhaustive]`, so it cannot be built with a
        // struct literal outside its crate (even with `..Default::default()`);
        // mutate a default instance instead.
        let mut filter = bc_ipc::Filter::default();
        filter.accounts = vec!["a1".to_owned(), "a2".to_owned()];
        filter.tags = vec!["t1".to_owned()];

        /* a2 intentionally unresolved — it should fall back to the raw id. */
        let names = HashMap::from([
            ("a1".to_owned(), "Checking".to_owned()),
            ("t1".to_owned(), "groceries".to_owned()),
        ]);

        let chips = chips_from_filter(&filter, &names);
        let labels: Vec<_> = chips.iter().map(|c| c.label.as_str()).collect();

        assert_eq!(
            labels,
            vec!["account: Checking", "account: a2", "tag: groceries"]
        );
        assert_eq!(
            chips.first().map(|c| &c.remove),
            Some(&ChipRemove::Account("a1".to_owned()))
        );
        assert_eq!(
            chips.get(2).map(|c| &c.remove),
            Some(&ChipRemove::Tag("t1".to_owned()))
        );
    }

    #[test]
    fn amount_bounds_become_separate_over_under_chips() {
        let mut amount = bc_ipc::AmountFilter::default();
        amount.min = Some("100".parse().expect("decimal"));
        amount.max = Some("500".parse().expect("decimal"));
        let mut filter = bc_ipc::Filter::default();
        filter.amount = Some(amount);

        let chips = chips_from_filter(&filter, &HashMap::new());
        let labels: Vec<_> = chips.iter().map(|c| c.label.as_str()).collect();

        assert_eq!(labels, vec!["over: 100", "under: 500"]);
        assert_eq!(
            chips.first().map(|c| &c.remove),
            Some(&ChipRemove::AmountMin)
        );
        assert_eq!(
            chips.get(1).map(|c| &c.remove),
            Some(&ChipRemove::AmountMax)
        );
    }

    #[test]
    fn amount_commodity_shows_on_both_bound_chips() {
        let mut amount = bc_ipc::AmountFilter::default();
        amount.min = Some("100".parse().expect("decimal"));
        amount.max = Some("500".parse().expect("decimal"));
        amount.commodity = Some("USD".to_owned());
        let mut filter = bc_ipc::Filter::default();
        filter.amount = Some(amount);

        let chips = chips_from_filter(&filter, &HashMap::new());
        let labels: Vec<_> = chips.iter().map(|c| c.label.as_str()).collect();

        assert_eq!(labels, vec!["over: USD 100", "under: USD 500"]);
    }

    #[test]
    fn empty_filter_is_inactive() {
        assert!(!super::filter_is_active(&bc_ipc::Filter::default()));
    }

    #[test]
    fn filter_with_any_dimension_is_active() {
        let mut text = bc_ipc::Filter::default();
        text.text = Some("coles".to_owned());
        assert!(super::filter_is_active(&text));

        let mut acct = bc_ipc::Filter::default();
        acct.accounts = vec!["a1".to_owned()];
        assert!(super::filter_is_active(&acct));

        let mut date = bc_ipc::Filter::default();
        date.date_from = Some(jiff::civil::Date::constant(2026, 1, 1));
        assert!(super::filter_is_active(&date));
    }
}
