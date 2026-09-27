//! Global filter store: the active `Filter`, provided once at the shell root.
//! Chip derivation is pure.

use std::collections::HashMap;

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
    /// Clears the balance status.
    Balance,
    /// Removes one selected account by id.
    Account(String),
    /// Removes one selected tag by id.
    Tag(String),
}

/// The two display forms of a picked account or tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChipLabel {
    /// Shown on the chip: an account's shortest unique path suffix, a tag's path.
    pub short: String,
    /// Shown on hover: the full path.
    pub full: String,
}

/// One active filter value rendered as a removable chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    /// Stable key for the `<For>` list (unique per active value).
    pub key: String,
    /// Display text, e.g. `account: Checking` or `over: 100`.
    pub label: String,
    /// Hover text; the full path for account and tag chips, else `None`.
    pub title: Option<String>,
    /// Which filter value this chip removes.
    pub remove: ChipRemove,
}

/// Derives one removable chip per active filter value. Account and tag ids are
/// resolved via `names` to a short label and a full-path title (populated as
/// the user picks them), falling back to the raw id when a name is not known.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `names` - Map of account/tag id to its short and full labels.
#[must_use]
pub fn chips_from_filter(filter: &bc_ipc::Filter, names: &HashMap<String, ChipLabel>) -> Vec<Chip> {
    let mut chips = Vec::new();

    for id in &filter.accounts {
        let (name, title) = resolve_label(names, id);
        chips.push(Chip {
            key: format!("account:{id}"),
            label: format!("account: {name}"),
            title,
            remove: ChipRemove::Account(id.clone()),
        });
    }
    for id in &filter.tags {
        let (name, title) = resolve_label(names, id);
        chips.push(Chip {
            key: format!("tag:{id}"),
            label: format!("tag: {name}"),
            title,
            remove: ChipRemove::Tag(id.clone()),
        });
    }
    if let Some(text) = &filter.text {
        chips.push(Chip {
            key: "text".to_owned(),
            label: format!("text: {text}"),
            title: None,
            remove: ChipRemove::Text,
        });
    }
    if let Some(after) = filter.date_from {
        chips.push(Chip {
            key: "after".to_owned(),
            label: format!("after: {after}"),
            title: None,
            remove: ChipRemove::DateFrom,
        });
    }
    if let Some(before) = filter.date_until {
        chips.push(Chip {
            key: "before".to_owned(),
            label: format!("before: {before}"),
            title: None,
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
                title: None,
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
                title: None,
                remove: ChipRemove::AmountMax,
            });
        }
    }
    if let Some(rec) = filter.reconciliation {
        chips.push(Chip {
            key: "status".to_owned(),
            label: format!("status: {}", rec.label()),
            title: None,
            remove: ChipRemove::Status,
        });
    }
    if let Some(balance) = filter.balance {
        chips.push(Chip {
            key: "balance".to_owned(),
            label: format!("status: {}", balance.label()),
            title: None,
            remove: ChipRemove::Balance,
        });
    }
    chips
}

/// Looks up an account or tag id's short label and full-path title, falling
/// back to the raw id with no title when the id is unknown.
fn resolve_label(names: &HashMap<String, ChipLabel>, id: &str) -> (String, Option<String>) {
    names.get(id).map_or_else(
        || (id.to_owned(), None),
        |l| (l.short.clone(), Some(l.full.clone())),
    )
}

/// Clears the single filter value identified by `target`, leaving every other
/// dimension untouched.
///
/// # Arguments
///
/// * `filter` - The filter to update.
/// * `target` - Which filter value to clear.
fn apply_chip_remove(filter: &mut bc_ipc::Filter, target: &ChipRemove) {
    match target {
        ChipRemove::DateFrom => filter.date_from = None,
        ChipRemove::DateUntil => filter.date_until = None,
        ChipRemove::Text => filter.text = None,
        ChipRemove::Status => filter.reconciliation = None,
        ChipRemove::Balance => filter.balance = None,
        ChipRemove::Account(id) => filter.accounts.retain(|a| a != id),
        ChipRemove::Tag(id) => filter.tags.retain(|t| t != id),
        ChipRemove::AmountMin => {
            if let Some(a) = filter.amount.as_mut() {
                a.min = None;
            }
            drop_empty_amount(filter);
        }
        ChipRemove::AmountMax => {
            if let Some(a) = filter.amount.as_mut() {
                a.max = None;
            }
            drop_empty_amount(filter);
        }
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

/// Signal-backed pieces of the filter store; kept in a submodule so only its
/// `RwSignal`/`provide_context` internals are gated on `wasm32`, while the
/// pure `Chip`/`chips_from_filter` above stay natively testable.
#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::collections::HashMap;

    use leptos::prelude::*;

    use super::ChipRemove;
    use super::apply_chip_remove;

    /// Reactive global filter state, provided once at the shell root.
    #[derive(Clone, Copy)]
    pub struct FilterStore {
        /// The active filter.
        pub filter: RwSignal<bc_ipc::Filter>,
        /// Short and full labels for the account/tag ids in `filter`, recorded
        /// as the user picks them so chips resolve names without a round-trip.
        pub labels: RwSignal<HashMap<String, super::ChipLabel>>,
    }

    impl FilterStore {
        /// Adds an account to the filter (no-op if already present), recording
        /// its short label and full path for chip rendering.
        ///
        /// # Arguments
        ///
        /// * `id` - The account id.
        /// * `short` - The account's shortest unique path suffix.
        /// * `full` - The account's full path.
        pub fn add_account(&self, id: String, short: String, full: String) {
            self.labels.update(|m| {
                m.insert(id.clone(), super::ChipLabel { short, full });
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
                m.insert(
                    id.clone(),
                    super::ChipLabel {
                        short: path.clone(),
                        full: path,
                    },
                );
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
            self.filter.update(|f| apply_chip_remove(f, target));
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
        })
    }
}

#[cfg(target_arch = "wasm32")]
#[expect(
    unused_imports,
    reason = "re-exported for callers naming the FilterStore type explicitly; \
              current call sites only use type inference via use_filter_store()/provide_filter_store()"
)]
pub use wasm::FilterStore;
#[cfg(target_arch = "wasm32")]
pub use wasm::provide_filter_store;
#[cfg(target_arch = "wasm32")]
pub use wasm::use_filter_store;

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::HashMap;

    use pretty_assertions::assert_eq;

    use super::ChipLabel;
    use super::ChipRemove;
    use super::apply_chip_remove;
    use super::chips_from_filter;

    #[test]
    fn remove_chip_balance_clears_only_balance() {
        let mut filter = bc_ipc::Filter::default();
        filter.reconciliation = Some(bc_ipc::Reconciliation::Unreconciled);
        filter.balance = Some(bc_ipc::BalanceStatus::Unbalanced);

        apply_chip_remove(&mut filter, &ChipRemove::Balance);

        assert_eq!(filter.balance, None);
        assert_eq!(
            filter.reconciliation,
            Some(bc_ipc::Reconciliation::Unreconciled)
        );
    }

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
            (
                "a1".to_owned(),
                ChipLabel {
                    short: "Checking".to_owned(),
                    full: "Assets :: Checking".to_owned(),
                },
            ),
            (
                "t1".to_owned(),
                ChipLabel {
                    short: "groceries".to_owned(),
                    full: "groceries".to_owned(),
                },
            ),
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
    fn account_chip_shows_the_short_label_and_titles_the_full_path() {
        let mut filter = bc_ipc::Filter::default();
        filter.accounts = vec!["a1".to_owned()];
        filter.tags = vec!["t1".to_owned()];
        let names = HashMap::from([
            (
                "a1".to_owned(),
                ChipLabel {
                    short: "BankA :: Holiday".to_owned(),
                    full: "Assets :: BankA :: Holiday".to_owned(),
                },
            ),
            (
                "t1".to_owned(),
                ChipLabel {
                    short: "trip:beach".to_owned(),
                    full: "trip:beach".to_owned(),
                },
            ),
        ]);

        let chips = chips_from_filter(&filter, &names);

        assert_eq!(
            chips
                .first()
                .map(|c| (c.label.as_str(), c.title.as_deref())),
            Some((
                "account: BankA :: Holiday",
                Some("Assets :: BankA :: Holiday")
            ))
        );
        assert_eq!(
            chips.get(1).map(|c| (c.label.as_str(), c.title.as_deref())),
            Some(("tag: trip:beach", Some("trip:beach")))
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
    fn balance_and_reconciliation_are_separate_status_chips() {
        let mut filter = bc_ipc::Filter::default();
        filter.reconciliation = Some(bc_ipc::Reconciliation::Unreconciled);
        filter.balance = Some(bc_ipc::BalanceStatus::Unbalanced);

        let chips = chips_from_filter(&filter, &HashMap::new());
        let labels: Vec<_> = chips.iter().map(|c| c.label.as_str()).collect();

        assert_eq!(labels, vec!["status: unreconciled", "status: unbalanced"]);
        assert_eq!(chips.get(1).map(|c| c.key.as_str()), Some("balance"));
        assert_eq!(chips.get(1).map(|c| &c.remove), Some(&ChipRemove::Balance));
    }
}
