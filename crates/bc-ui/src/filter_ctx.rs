//! Global filter store: the active `Filter` plus a presentation-only strictness
//! toggle, provided once at the shell root. Chip derivation is pure.

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

/// One active filter dimension rendered as a removable chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    /// Stable dimension key (used to remove the dimension).
    pub key: &'static str,
    /// Display text, e.g. `text: amazon`.
    pub label: String,
}

/// Derives the removable chips for the active dimensions of `filter`.
#[must_use]
#[cfg_attr(
    target_arch = "wasm32",
    expect(dead_code, reason = "wired into the chip-rendering UI in a later task")
)]
pub fn chips_from_filter(filter: &bc_ipc::Filter) -> Vec<Chip> {
    let mut chips = Vec::new();
    if filter.date_from.is_some() || filter.date_until.is_some() {
        let from = filter.date_from.map(|d| d.to_string()).unwrap_or_default();
        let until = filter.date_until.map(|d| d.to_string()).unwrap_or_default();
        chips.push(Chip {
            key: "date",
            label: format!("date: {from}…{until}"),
        });
    }
    if !filter.accounts.is_empty() {
        chips.push(Chip {
            key: "accounts",
            label: format!("account: {} selected", filter.accounts.len()),
        });
    }
    if !filter.tags.is_empty() {
        chips.push(Chip {
            key: "tags",
            label: format!("tag: {} selected", filter.tags.len()),
        });
    }
    if let Some(text) = &filter.text {
        chips.push(Chip {
            key: "text",
            label: format!("text: {text}"),
        });
    }
    if let Some(amount) = &filter.amount {
        let min = amount.min.map(|d| d.to_string()).unwrap_or_default();
        let max = amount.max.map(|d| d.to_string()).unwrap_or_default();
        chips.push(Chip {
            key: "amount",
            label: format!("amount: {min}…{max}"),
        });
    }
    if let Some(rec) = filter.reconciliation {
        chips.push(Chip {
            key: "status",
            label: format!("status: {}", rec.label()),
        });
    }
    chips
}

/// Signal-backed pieces of the filter store; kept in a submodule so only its
/// `RwSignal`/`provide_context` internals are gated on `wasm32`, while the
/// pure `Strictness`/`Chip`/`chips_from_filter` above stay natively testable.
#[cfg(target_arch = "wasm32")]
mod wasm {
    use leptos::prelude::*;

    use super::Strictness;

    /// Reactive global filter state, provided once at the shell root.
    #[derive(Clone, Copy)]
    pub struct FilterStore {
        /// The active filter.
        pub filter: RwSignal<bc_ipc::Filter>,
        /// Presentation-only strictness toggle.
        #[expect(
            dead_code,
            reason = "wired into the strictness toggle UI in a later task"
        )]
        pub strictness: RwSignal<Strictness>,
    }

    impl FilterStore {
        /// Clears the dimension identified by a chip `key`.
        ///
        /// # Arguments
        ///
        /// * `key` - The chip key identifying which filter dimension to clear.
        #[expect(dead_code, reason = "wired into the chip-rendering UI in a later task")]
        pub fn clear_dimension(&self, key: &str) {
            self.filter.update(|f| match key {
                "date" => {
                    f.date_from = None;
                    f.date_until = None;
                }
                "accounts" => f.accounts.clear(),
                "tags" => f.tags.clear(),
                "text" => f.text = None,
                "amount" => f.amount = None,
                "status" => f.reconciliation = None,
                _ => {}
            });
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
    #[expect(
        dead_code,
        reason = "consumed by per-view filter chips in a later task"
    )]
    pub fn use_filter_store() -> FilterStore {
        use_context::<FilterStore>().unwrap_or_else(|| FilterStore {
            filter: RwSignal::new(bc_ipc::Filter::default()),
            strictness: RwSignal::new(Strictness::default()),
        })
    }
}

#[cfg(target_arch = "wasm32")]
#[expect(
    unused_imports,
    reason = "FilterStore/use_filter_store are consumed by per-view filter chips in a later task"
)]
pub use wasm::FilterStore;
#[cfg(target_arch = "wasm32")]
pub use wasm::provide_filter_store;
#[cfg(target_arch = "wasm32")]
#[expect(
    unused_imports,
    reason = "FilterStore/use_filter_store are consumed by per-view filter chips in a later task"
)]
pub use wasm::use_filter_store;

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::chips_from_filter;

    #[test]
    fn empty_filter_has_no_chips() {
        let chips = chips_from_filter(&bc_ipc::Filter::default());
        assert!(chips.is_empty());
    }

    #[test]
    fn active_dimensions_become_chips() {
        // `bc_ipc::Filter` is `#[non_exhaustive]`, so it cannot be built with a
        // struct literal outside its crate (even with `..Default::default()`);
        // mutate a default instance instead.
        let mut filter = bc_ipc::Filter::default();
        filter.text = Some("amazon".to_owned());
        filter.tags = vec!["t1".to_owned(), "t2".to_owned()];
        let keys: Vec<_> = chips_from_filter(&filter)
            .into_iter()
            .map(|c| c.key)
            .collect();
        assert!(keys.contains(&"text"));
        assert!(keys.contains(&"tags"));
        assert_eq!(keys.len(), 2);
    }
}
