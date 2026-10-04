//! Global filter store: the active Filter, and what a palette commit does to it.
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

/// Where a palette commit lands in the stored query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditTarget {
    /// Joined onto the stored query by `and`.
    Append,
    /// In place of the top-level conjunct printed as this text.
    Conjunct(String),
    /// In place of the whole stored query.
    Whole,
}

/// What the palette opens on: its starting text and where a commit lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    /// Where a commit lands.
    pub target: EditTarget,
    /// The text the input starts with.
    pub text: String,
}

impl Edit {
    /// A blank palette whose commit joins the stored query.
    #[must_use]
    pub const fn append() -> Self {
        Self {
            target: EditTarget::Append,
            text: String::new(),
        }
    }

    /// A palette editing one chip's conjunct.
    ///
    /// # Arguments
    ///
    /// * `text` - The conjunct's canonical text.
    #[must_use]
    pub fn conjunct(text: &str) -> Self {
        Self {
            target: EditTarget::Conjunct(text.to_owned()),
            text: text.to_owned(),
        }
    }

    /// A palette editing the whole stored query.
    ///
    /// # Arguments
    ///
    /// * `text` - The stored query's text.
    #[must_use]
    pub fn whole(text: &str) -> Self {
        Self {
            target: EditTarget::Whole,
            text: text.to_owned(),
        }
    }
}

/// `filter` after a palette commit of `expr` to `target`; `None` is blank text.
///
/// An append ANDs `expr` on, so an `or` prints in brackets among other
/// conjuncts, and a blank append changes nothing. A conjunct edit replaces that
/// conjunct in place, and a blank one removes it. A whole edit replaces
/// everything.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `target` - Where the commit lands.
/// * `expr` - The committed query, `None` when the text was blank.
#[must_use]
pub fn commit(filter: &Filter, target: &EditTarget, expr: Option<Expr>) -> Filter {
    let stored = query_expr(filter);
    let next = match target {
        EditTarget::Append => match expr {
            Some(add) => Some(shape::and_onto(stored.as_ref(), add)),
            None => stored,
        },
        EditTarget::Conjunct(old) => match stored {
            Some(base) => shape::replace_conjunct(&base, old, expr),
            None => expr,
        },
        EditTarget::Whole => expr,
    };
    with_query(filter, next.as_ref())
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

    use leptos::prelude::*;

    use super::ChipLabel;
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

    /// Opens the command palette on an [`super::Edit`]; provided once at the
    /// shell root.
    #[derive(Clone, Copy)]
    pub struct PaletteOpener(Callback<super::Edit>);

    impl PaletteOpener {
        /// Opens the palette on `edit`.
        ///
        /// # Arguments
        ///
        /// * `edit` - The starting text and where a commit lands.
        pub fn open(&self, edit: super::Edit) {
            self.0.run(edit);
        }
    }

    /// Provides the palette opener into context. Call once, at the shell root.
    ///
    /// # Arguments
    ///
    /// * `open` - Opens the palette on an edit.
    pub fn provide_palette_opener(open: Callback<super::Edit>) {
        provide_context(PaletteOpener(open));
    }

    /// Reads the palette opener from context; outside the shell it does nothing.
    ///
    /// # Returns
    ///
    /// The opener from context, or one that ignores every edit.
    #[must_use]
    pub fn use_palette_opener() -> PaletteOpener {
        use_context::<PaletteOpener>().unwrap_or_else(|| PaletteOpener(Callback::new(|_| {})))
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
#[expect(
    unused_imports,
    reason = "re-exported for callers naming the PaletteOpener type explicitly; \
              current call sites only use type inference via use_palette_opener()"
)]
pub use wasm::PaletteOpener;
#[cfg(target_arch = "wasm32")]
pub use wasm::provide_filter_store;
#[cfg(target_arch = "wasm32")]
pub use wasm::provide_palette_opener;
#[cfg(target_arch = "wasm32")]
pub use wasm::use_filter_store;
#[cfg(target_arch = "wasm32")]
pub use wasm::use_palette_opener;

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::HashMap;

    use bc_ipc::Filter;
    use bc_query::parse;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::ChipLabel;
    use super::Edit;
    use super::EditTarget;
    use super::chips_from_filter;
    use super::commit;
    use super::remove_conjunct;

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

    #[rstest]
    #[case("tag:me", EditTarget::Append, Some("coffee"), "tag:me coffee")]
    #[case("tag:me", EditTarget::Append, Some("a or b"), "tag:me (a or b)")]
    #[case("", EditTarget::Append, Some("a or b"), "a or b")]
    #[case("tag:me", EditTarget::Append, Some("tag:me"), "tag:me")]
    #[case("tag:me", EditTarget::Append, None, "tag:me")]
    #[case("a (b or c) d", EditTarget::Conjunct("b or c".to_owned()), Some("x"), "a x d")]
    #[case("a (b or c) d", EditTarget::Conjunct("d".to_owned()), None, "a (b or c)")]
    #[case("a (b or c) d", EditTarget::Conjunct("gone".to_owned()), Some("x"), "a (b or c) d x")]
    #[case("", EditTarget::Conjunct("gone".to_owned()), Some("x"), "x")]
    #[case("a b", EditTarget::Whole, Some("x y"), "x y")]
    #[case("a b", EditTarget::Whole, None, "")]
    fn commits_land_where_the_edit_points(
        #[case] stored: &str,
        #[case] target: EditTarget,
        #[case] typed: Option<&str>,
        #[case] expected: &str,
    ) {
        let next = commit(
            &filter(stored),
            &target,
            typed.map(|text| parse(text).expect("parses")),
        );
        assert_eq!(next.query, expected);
    }

    #[test]
    fn dismissing_a_chip_removes_its_conjunct() {
        let one_left = remove_conjunct(&filter("(tag:recurring or tag:work) tea"), "tea");
        assert_eq!(one_left.query, "tag:recurring or tag:work");
        let none_left = remove_conjunct(&one_left, "tag:recurring or tag:work");
        assert_eq!(none_left.query, "");
    }

    #[test]
    fn edits_open_with_their_text() {
        assert_eq!(
            Edit::append(),
            Edit {
                target: EditTarget::Append,
                text: String::new()
            }
        );
        assert_eq!(
            Edit::conjunct("tag:me"),
            Edit {
                target: EditTarget::Conjunct("tag:me".to_owned()),
                text: "tag:me".to_owned()
            }
        );
        assert_eq!(
            Edit::whole("a b"),
            Edit {
                target: EditTarget::Whole,
                text: "a b".to_owned()
            }
        );
    }
}
