//! Global filter store: the active `Filter`, provided once at the shell root.
//! Chips and query edits are pure: each chip is one top-level conjunct of the
//! query text.

use std::collections::HashMap;

use bc_ipc::Filter;
use bc_query::Expr;
use bc_query::print;
use bc_query::shape;

/// The two display forms of a palette-picked account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChipLabel {
    /// Shown on the chip: `account:` and the shortest unique path ending.
    pub short: String,
    /// Shown on hover: the term's canonical text.
    pub full: String,
}

/// One top-level conjunct of the query, rendered as a removable chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    /// Stable key for the `<For>` list.
    pub key: String,
    /// Display text: the conjunct's canonical text, or a picked account's short label.
    pub label: String,
    /// Hover text; the canonical text when the label shortens it.
    pub title: Option<String>,
    /// The conjunct's canonical text, which dismissing the chip removes.
    pub remove: String,
}

/// The filter's query as an expression; `None` when blank or unparsable.
///
/// # Arguments
///
/// * `filter` - The active filter.
#[must_use]
pub fn query_expr(filter: &Filter) -> Option<Expr> {
    let text = filter.query.trim();
    if text.is_empty() {
        return None;
    }
    bc_query::parse(text).ok()
}

/// One chip per top-level conjunct. A conjunct made of picked accounts (alone
/// or as `or` alternatives) shows their short labels with the canonical text
/// on hover.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `labels` - Short and full labels of picked accounts, by term text.
#[must_use]
pub fn chips_from_filter(filter: &Filter, labels: &HashMap<String, ChipLabel>) -> Vec<Chip> {
    let Some(expr) = query_expr(filter) else {
        return Vec::new();
    };
    shape::conjuncts(&expr)
        .into_iter()
        .enumerate()
        .map(|(index, conjunct)| {
            let text = print(conjunct);
            let (label, title) = display(conjunct, labels);
            Chip {
                key: format!("{index}:{text}"),
                label,
                title,
                remove: text,
            }
        })
        .collect()
}

/// A conjunct's label and title.
fn display(conjunct: &Expr, labels: &HashMap<String, ChipLabel>) -> (String, Option<String>) {
    let parts: Vec<&Expr> = match conjunct {
        Expr::Or(items, _) => items.iter().collect(),
        Expr::And(..) | Expr::Not(..) | Expr::Term(_) | Expr::Word(_) => vec![conjunct],
    };
    let picked: Vec<&ChipLabel> = parts
        .iter()
        .filter_map(|part| labels.get(&print(part)))
        .collect();
    if picked.len() != parts.len() {
        return (print(conjunct), None);
    }
    let join = |pick: fn(&ChipLabel) -> &str| {
        picked
            .iter()
            .map(|l| pick(l))
            .collect::<Vec<_>>()
            .join(" or ")
    };
    (join(|l| l.short.as_str()), Some(join(|l| l.full.as_str())))
}

/// `filter` with its query replaced by `expr` (blank for `None`).
fn with_query(filter: &Filter, expr: Option<&Expr>) -> Filter {
    let mut out = filter.clone();
    out.query = expr.map(print).unwrap_or_default();
    out
}

/// `filter` with `add` joined onto its query by `and`.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `add` - The expression to join on.
#[must_use]
pub fn and_term(filter: &Filter, add: Expr) -> Filter {
    with_query(
        filter,
        Some(&shape::and_onto(query_expr(filter).as_ref(), add)),
    )
}

/// `filter` with `add` replacing the top-level conjuncts `drop` selects.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `drop` - Selects the conjuncts to replace.
/// * `add` - The replacement.
#[must_use]
pub fn replace_term(filter: &Filter, drop: impl Fn(&Expr) -> bool, add: Expr) -> Filter {
    with_query(
        filter,
        Some(&shape::replace(query_expr(filter).as_ref(), drop, add)),
    )
}

/// `filter` with `add` offered as an alternative to its top-level `field` conjunct.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `field` - The field whose conjunct gains the alternative.
/// * `add` - The alternative.
#[must_use]
pub fn or_term(filter: &Filter, field: &str, add: Expr) -> Filter {
    with_query(
        filter,
        Some(&shape::or_merge(query_expr(filter).as_ref(), field, add)),
    )
}

/// `filter` without the top-level conjunct printed as `target`.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `target` - The conjunct's canonical text.
#[must_use]
pub fn remove_conjunct(filter: &Filter, target: &str) -> Filter {
    let kept = query_expr(filter).and_then(|expr| shape::remove(&expr, target));
    with_query(filter, kept.as_ref())
}

/// Signal-backed pieces of the filter store; kept in a submodule so only its
/// `RwSignal`/`provide_context` internals are gated on `wasm32`, while the
/// pure chip and query helpers above stay natively testable.
#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::inline_modules,
    reason = "gating one submodule keeps the pure chip helpers above natively testable"
)]
mod wasm {
    use std::collections::HashMap;

    use bc_query::ast::Op;
    use bc_query::print;
    use bc_query::shape::builtin_term;
    use leptos::prelude::*;

    use super::ChipLabel;
    use super::or_term;
    use super::remove_conjunct;

    /// Reactive global filter state, provided once at the shell root.
    #[derive(Clone, Copy)]
    pub struct FilterStore {
        /// The active filter.
        pub filter: RwSignal<bc_ipc::Filter>,
        /// Short and full labels of palette-picked accounts, by term text,
        /// recorded as the user picks them so chips shorten them.
        pub labels: RwSignal<HashMap<String, ChipLabel>>,
    }

    impl FilterStore {
        /// Offers an account subtree as one more `account:` alternative,
        /// recording its short label for the chip.
        ///
        /// # Arguments
        ///
        /// * `path` - The account's colon-separated path.
        /// * `short` - Its shortest unique colon-separated ending.
        pub fn add_account(&self, path: &str, short: &str) {
            let term = builtin_term("account", Op::Match, path);
            let text = print(&term);
            self.labels.update(|m| {
                m.insert(
                    text.clone(),
                    ChipLabel {
                        short: format!("account:{short}"),
                        full: text,
                    },
                );
            });
            self.filter.update(|f| *f = or_term(f, "account", term));
        }

        /// Offers a tag subtree as one more `tag:` alternative.
        ///
        /// # Arguments
        ///
        /// * `path` - The tag's colon-separated path.
        pub fn add_tag(&self, path: &str) {
            let term = builtin_term("tag", Op::Match, path);
            self.filter.update(|f| *f = or_term(f, "tag", term));
        }

        /// Removes the chip whose conjunct prints as `target`.
        ///
        /// # Arguments
        ///
        /// * `target` - The chip's `remove` text.
        pub fn remove_chip(&self, target: &str) {
            self.filter.update(|f| *f = remove_conjunct(f, target));
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

    use bc_ipc::Filter;
    use bc_query::parse;
    use pretty_assertions::assert_eq;

    use super::ChipLabel;
    use super::and_term;
    use super::chips_from_filter;
    use super::or_term;
    use super::remove_conjunct;
    use super::replace_term;

    fn filter(query: &str) -> Filter {
        Filter::new(query, None, None)
    }

    fn drinking() -> HashMap<String, ChipLabel> {
        HashMap::from([(
            "account:Expenses:Drinking".to_owned(),
            ChipLabel {
                short: "account:Drinking".to_owned(),
                full: "account:Expenses:Drinking".to_owned(),
            },
        )])
    }

    #[test]
    fn a_blank_or_unparsable_query_has_no_chips() {
        assert_eq!(chips_from_filter(&Filter::default(), &HashMap::new()), []);
        assert_eq!(chips_from_filter(&filter("(open"), &HashMap::new()), []);
    }

    #[test]
    fn each_top_level_conjunct_is_a_chip() {
        let f = filter(
            "account:Expenses:Drinking tag:recurring status:unreconciled date:>=2026-01-01 fortnightly",
        );
        let chips = chips_from_filter(&f, &drinking());
        let labels: Vec<&str> = chips.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "account:Drinking",
                "tag:recurring",
                "status:unreconciled",
                "date:>=2026-01-01",
                "fortnightly"
            ]
        );
        assert_eq!(
            chips.first().and_then(|c| c.title.as_deref()),
            Some("account:Expenses:Drinking")
        );
        assert_eq!(chips.get(1).and_then(|c| c.title.as_deref()), None);
        assert_eq!(
            chips.first().map(|c| c.remove.as_str()),
            Some("account:Expenses:Drinking")
        );
    }

    #[test]
    fn an_or_of_picked_accounts_shows_their_short_labels() {
        let mut labels = drinking();
        labels.insert(
            "account:Expenses:Dining".to_owned(),
            ChipLabel {
                short: "account:Dining".to_owned(),
                full: "account:Expenses:Dining".to_owned(),
            },
        );
        let chips = chips_from_filter(
            &filter("(account:Expenses:Drinking or account:Expenses:Dining) x"),
            &labels,
        );
        assert_eq!(
            chips.first().map(|c| c.label.as_str()),
            Some("account:Drinking or account:Dining")
        );
        assert_eq!(
            chips.first().map(|c| c.remove.as_str()),
            Some("account:Expenses:Drinking or account:Expenses:Dining")
        );
    }

    #[test]
    fn edits_rewrite_the_query_text() {
        let start = filter("tag:recurring");
        let joined = and_term(&start, parse("coffee").expect("parses"));
        assert_eq!(joined.query, "tag:recurring coffee");
        let merged = or_term(&joined, "tag", parse("tag:work").expect("parses"));
        assert_eq!(merged.query, "(tag:recurring or tag:work) coffee");
        let replaced = replace_term(
            &merged,
            |c| bc_query::print(c) == "coffee",
            parse("tea").expect("parses"),
        );
        assert_eq!(replaced.query, "(tag:recurring or tag:work) tea");
        let one_left = remove_conjunct(&replaced, "tea");
        assert_eq!(one_left.query, "tag:recurring or tag:work");
        let none_left = remove_conjunct(&one_left, "tag:recurring or tag:work");
        assert_eq!(none_left.query, "");
    }
}
