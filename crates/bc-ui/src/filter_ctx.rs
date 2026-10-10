//! Global filter store: the active Filter, and what a palette commit does to it.
//! Chips and query edits are pure: each chip is one top-level conjunct of the
//! query text.

use std::collections::HashMap;

use bc_ipc::Filter;
use bc_query::Catalog;
use bc_query::Diagnostic;
use bc_query::Expr;
use bc_query::Severity;
use bc_query::Span;
use bc_query::ast::Criterion;
use bc_query::catalog::shortest_endings;
use bc_query::filter::Pred;
use bc_query::filter::ResolvedExpr;
use bc_query::filter::TagPred;
use bc_query::print;
use bc_query::resolve;
use bc_query::shape;
use bc_query::value_text;

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

/// One top-level conjunct of the query, rendered as a chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    /// Stable key for the `<For>` list.
    pub key: String,
    /// What the chip shows: a path term's shortest unique ending, else the
    /// conjunct's canonical text.
    pub label: String,
    /// Hover text: the full path when the label shortens it, then every
    /// problem a resolve finds in the conjunct.
    pub title: Option<String>,
    /// The conjunct's canonical text, which editing opens and dismissing removes.
    pub text: String,
    /// The worst error or warning in the conjunct.
    pub severity: Option<Severity>,
}

/// Every account's and tag's shortest unique ending, keyed by id.
struct Endings {
    /// Account endings.
    accounts: HashMap<String, String>,
    /// Tag endings.
    tags: HashMap<String, String>,
}

impl Endings {
    /// The endings for `catalog`.
    fn new<C>(catalog: &C) -> Self
    where
        C: Catalog,
    {
        Self {
            accounts: shortest_endings(catalog.accounts()),
            tags: shortest_endings(catalog.tags()),
        }
    }
}

/// One chip per top-level conjunct of the stored query.
///
/// With a catalog, path terms show their shortest unique ending, and each chip
/// carries the problems a resolve finds in its conjunct. Before the catalog
/// loads, chips show their canonical text with no severity.
///
/// # Arguments
///
/// * `filter` - The active filter.
/// * `catalog` - The query catalog, once loaded.
#[must_use]
pub fn chips_from_filter<C>(filter: &Filter, catalog: Option<&C>) -> Vec<Chip>
where
    C: Catalog,
{
    let Some(expr) = query_expr(filter) else {
        return Vec::new();
    };
    let diagnostics: Vec<Diagnostic> = catalog
        .map(|c| resolve(&expr, c).diagnostics)
        .unwrap_or_default();
    let endings = catalog.map(Endings::new);
    shape::conjuncts(&expr)
        .into_iter()
        .enumerate()
        .map(|(index, conjunct)| {
            let text = print(conjunct);
            let mine: Vec<&Diagnostic> = diagnostics
                .iter()
                .filter(|d| touches(conjunct.span(), d.span))
                .collect();
            let severity = mine
                .iter()
                .map(|d| d.severity)
                .filter(|s| *s != Severity::Hint)
                .min_by_key(|s| s.rank());
            let (label, full) = catalog
                .zip(endings.as_ref())
                .and_then(|(c, e)| path_labels(conjunct, c, e))
                .unwrap_or_else(|| (text.clone(), text.clone()));
            let mut lines: Vec<String> = Vec::new();
            if full != label {
                lines.push(full);
            }
            lines.extend(mine.iter().map(|d| d.message.clone()));
            Chip {
                key: format!("{index}:{text}"),
                label,
                title: (!lines.is_empty()).then(|| lines.join("\n")),
                text,
                severity,
            }
        })
        .collect()
}

/// Whether diagnostic `span` starts or ends inside `conjunct`.
///
/// A contradiction between two conjuncts spans everything between them, so it
/// marks its two ends and spares the conjuncts it merely covers.
fn touches(conjunct: Span, span: Span) -> bool {
    let starts_inside = conjunct.start <= span.start && span.start < conjunct.end;
    let ends_inside =
        span.end > span.start && conjunct.start < span.end && span.end <= conjunct.end;
    starts_inside || ends_inside
}

/// The short and full labels of a conjunct made only of path terms, joined by
/// `or`; `None` for any other conjunct.
fn path_labels<C>(conjunct: &Expr, catalog: &C, endings: &Endings) -> Option<(String, String)>
where
    C: Catalog,
{
    let parts: Vec<&Expr> = match conjunct {
        Expr::Or(items, _) => items.iter().collect(),
        Expr::And(..) | Expr::Not(..) | Expr::Term(_) | Expr::Word(_) => vec![conjunct],
    };
    let labels = parts
        .into_iter()
        .map(|part| path_label(part, catalog, endings))
        .collect::<Option<Vec<(String, String)>>>()?;
    let short: Vec<&str> = labels.iter().map(|(s, _)| s.as_str()).collect();
    let full: Vec<&str> = labels.iter().map(|(_, f)| f.as_str()).collect();
    Some((short.join(" or "), full.join(" or ")))
}

/// A built-in `account:` or `tag:` term's short and full label; `None` for
/// any other expression, or one that does not resolve.
fn path_label<C>(part: &Expr, catalog: &C, endings: &Endings) -> Option<(String, String)>
where
    C: Catalog,
{
    let Expr::Term(term) = part else {
        return None;
    };
    let Criterion::Compare { op, .. } = &term.criterion else {
        return None;
    };
    if term.field.meta {
        return None;
    }
    let Some(ResolvedExpr::Pred(pred)) = resolve(part, catalog).expr else {
        return None;
    };
    let (field, entries, ends, id) = if let Pred::Account { id, .. } = &pred {
        ("account", catalog.accounts(), &endings.accounts, id)
    } else if let Pred::Tag(TagPred::Tag { id, .. }) = &pred {
        ("tag", catalog.tags(), &endings.tags, id)
    } else {
        return None;
    };
    let entry = entries.iter().find(|e| &e.id == id)?;
    let ending = ends.get(id)?;
    let op_text = op.as_str();
    Some((
        format!("{field}:{op_text}{}", value_text(ending)),
        format!("{field}:{op_text}{}", value_text(&entry.display())),
    ))
}

/// A server's query error as one sentence, quoting the text each problem
/// concerns.
///
/// # Arguments
///
/// * `query` - The query the server received.
/// * `problems` - Its problems, with byte spans into `query`.
#[must_use]
pub fn query_failure(query: &str, problems: &[bc_ipc::QueryProblem]) -> String {
    let parts: Vec<String> = problems
        .iter()
        .map(
            |p| match query.get(p.start..p.end).filter(|s| !s.trim().is_empty()) {
                Some(text) => format!("{} (at \u{201c}{text}\u{201d})", p.message),
                None => p.message.clone(),
            },
        )
        .collect();
    format!("The filter didn\u{2019}t run: {}.", parts.join("; "))
}

/// A failed load as one sentence: a query error names its spans; any other
/// error follows `prefix`.
///
/// # Arguments
///
/// * `prefix` - What failed, for errors other than a query's.
/// * `error` - The error.
/// * `query` - The query the load sent.
#[must_use]
pub fn load_error_text(prefix: &str, error: &bc_ipc::BcError, query: &str) -> String {
    if let bc_ipc::BcError::Query(problems) = error {
        query_failure(query, problems)
    } else {
        format!("{prefix}: {error}")
    }
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
    use leptos::prelude::*;
    use leptos_router::NavigateOptions;
    use leptos_router::hooks::use_location;
    use leptos_router::hooks::use_navigate;

    use super::remove_conjunct;
    use crate::components::period_nav::DisplayWindow;
    use crate::url_state::History;
    use crate::url_state::UrlState;

    /// The router's navigate function, kept on the thread that owns it.
    type Navigate = StoredValue<Box<dyn Fn(&str, NavigateOptions)>, LocalStorage>;

    /// Pushes `target` onto the browser history when the address bar does not
    /// already show it.
    fn push_if_not_shown(target: &str) {
        let Some(window) = leptos::web_sys::window() else {
            return;
        };
        let location = window.location();
        let shown = format!(
            "{}{}",
            location.pathname().unwrap_or_default(),
            location.search().unwrap_or_default()
        );
        if shown != target
            && let Ok(history) = window.history()
        {
            drop(history.push_state_with_url(
                &leptos::wasm_bindgen::JsValue::NULL,
                "",
                Some(target),
            ));
        }
    }

    /// Global filter state read from the URL query string, provided once at
    /// the shell root. Every write is a navigation, so back and forward
    /// restore earlier states.
    #[derive(Clone, Copy)]
    #[expect(
        clippy::partial_pub_fields,
        reason = "the filter and window are the read API; the rest is wiring"
    )]
    pub struct FilterStore {
        /// The active filter: the URL's query, with no date bounds.
        pub filter: Memo<bc_ipc::Filter>,
        /// The accounts page's display window.
        pub window: Memo<DisplayWindow>,
        /// The full URL state the memos above are derived from.
        state: Memo<UrlState>,
        /// The current path, kept when a write changes only the query string.
        pathname: Signal<String>,
        /// Performs the navigation behind every write.
        navigate: Navigate,
    }

    impl FilterStore {
        /// Builds a store over a location's pieces and a navigate function.
        fn from_location(
            pathname: Signal<String>,
            search: Signal<String>,
            navigate: Navigate,
        ) -> Self {
            let state = Memo::new(move |_| search.with(|s| UrlState::from_search(s).0));
            let filter = Memo::new(move |_| {
                state.with(|s| bc_ipc::Filter::new(s.query.clone(), None, None))
            });
            let window = Memo::new(move |_| state.with(|s| s.window.clone()));
            Self {
                filter,
                window,
                state,
                pathname,
                navigate,
            }
        }

        /// Navigates to `next` on the current path.
        fn go(&self, next: &UrlState, history: History) {
            let url = format!("{}{}", self.pathname.get_untracked(), next.to_search());
            let options = NavigateOptions {
                replace: history == History::Replace,
                scroll: false,
                ..NavigateOptions::default()
            };
            self.navigate.with_value(|nav| nav(&url, options));
            // The router skips its `pushState` when the target equals the last
            // entry of its private path stack, which a back and forward leaves stale.
            if history == History::Push {
                push_if_not_shown(&url);
            }
        }

        /// Navigates to `next`; does nothing when it is already current.
        ///
        /// # Arguments
        ///
        /// * `next` - The query and window to show.
        /// * `history` - Whether the write pushes or replaces.
        #[expect(
            clippy::needless_pass_by_value,
            reason = "callers hand over a freshly built state"
        )]
        pub fn set(&self, next: UrlState, history: History) {
            if self.state.with_untracked(|s| *s != next) {
                self.go(&next, history);
            }
        }

        /// Replaces the query, keeping the window.
        ///
        /// # Arguments
        ///
        /// * `query` - The new query text.
        /// * `history` - Whether the write pushes or replaces.
        pub fn set_query(&self, query: String, history: History) {
            let mut next = self.state.get_untracked();
            next.query = query;
            self.set(next, history);
        }

        /// Replaces the window, keeping the query.
        ///
        /// # Arguments
        ///
        /// * `window` - The new display window.
        /// * `history` - Whether the write pushes or replaces.
        pub fn set_window(&self, window: DisplayWindow, history: History) {
            let mut next = self.state.get_untracked();
            next.window = window;
            self.set(next, history);
        }

        /// Removes the chip whose conjunct prints as `target`.
        ///
        /// # Arguments
        ///
        /// * `target` - The chip's `text`.
        pub fn remove_chip(&self, target: &str) {
            let next = self.filter.with_untracked(|f| remove_conjunct(f, target));
            self.set_query(next.query, History::Push);
        }

        /// `path` with the current query string, so a link keeps the filter
        /// and window. Reactive.
        ///
        /// # Arguments
        ///
        /// * `path` - An in-app path with no query string.
        #[must_use]
        pub fn href(&self, path: &str) -> String {
            format!("{path}{}", self.state.with(UrlState::to_search))
        }
    }

    /// Provides the URL-backed [`FilterStore`] into context and rewrites a
    /// non-canonical query string in place. Call once at the shell root.
    ///
    /// # Returns
    ///
    /// The provided [`FilterStore`] handle.
    #[must_use]
    pub fn provide_filter_store() -> FilterStore {
        let location = use_location();
        let nav = use_navigate();
        let navigate: Navigate =
            StoredValue::new_local(Box::new(move |url: &str, options| nav(url, options)));
        let store =
            FilterStore::from_location(location.pathname.into(), location.search.into(), navigate);
        Effect::new(move |_| {
            let (state, canonical) = location.search.with(|s| UrlState::from_search(s));
            if !canonical {
                untrack(|| store.go(&state, History::Replace));
            }
        });
        provide_context(store);
        store
    }

    /// Reads the [`FilterStore`] from context; outside the shell, a detached
    /// empty store whose writes do nothing.
    ///
    /// # Returns
    ///
    /// The [`FilterStore`] handle from context, or a detached one.
    #[must_use]
    pub fn use_filter_store() -> FilterStore {
        use_context::<FilterStore>().unwrap_or_else(|| {
            FilterStore::from_location(
                Signal::stored(String::new()),
                Signal::stored(String::new()),
                StoredValue::new_local(Box::new(|_: &str, _| {})),
            )
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
    use bc_ipc::BcError;
    use bc_ipc::Filter;
    use bc_ipc::QueryProblem;
    use bc_query::Severity;
    use bc_query::catalog::MetaKey;
    use bc_query::catalog::MetaType;
    use bc_query::catalog::PathEntry;
    use bc_query::catalog::Snapshot;
    use bc_query::parse;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::Chip;
    use super::Edit;
    use super::EditTarget;
    use super::chips_from_filter;
    use super::commit;
    use super::load_error_text;
    use super::query_failure;
    use super::remove_conjunct;

    fn filter(query: &str) -> Filter {
        Filter::new(query, None, None)
    }

    /// An invented ledger.
    fn catalog() -> Snapshot {
        Snapshot::new(
            vec![
                PathEntry::new("a1", ["Expenses", "Food"]),
                PathEntry::new("a2", ["Expenses", "Food", "Groceries"]),
                PathEntry::new("a3", ["Income", "Food"]),
                PathEntry::new("a4", ["Assets", "Bank"]),
                PathEntry::new("a5", ["Liabilities", "Credit Card"]),
            ],
            vec![
                PathEntry::new("t1", ["me"]),
                PathEntry::new("t3", ["institution"]),
                PathEntry::new("t4", ["institution", "bank-a"]),
            ],
            Vec::new(),
            vec![MetaKey::new("km", MetaType::Number, 2)],
        )
    }

    /// The chips for `query` against the fixture catalog.
    fn chips(query: &str) -> Vec<Chip> {
        chips_from_filter(&filter(query), Some(&catalog()))
    }

    #[test]
    fn a_blank_or_unparsable_query_has_no_chips() {
        assert_eq!(chips(""), []);
        assert_eq!(chips("(open"), []);
    }

    #[test]
    fn each_top_level_conjunct_is_a_chip_carrying_its_text() {
        let got = chips("tag:me (coffee or tea) -status:flagged");
        let texts: Vec<&str> = got.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["tag:me", "coffee or tea", "-status:flagged"]);
        assert_eq!(got.get(1).map(|c| c.key.as_str()), Some("1:coffee or tea"));
    }

    #[rstest]
    #[case(
        "account:Expenses:Food:Groceries",
        "account:Groceries",
        Some("account:Expenses:Food:Groceries")
    )]
    #[case("account:=Assets:Bank", "account:=Bank", Some("account:=Assets:Bank"))]
    #[case("account:Expenses:Food", "account:Expenses:Food", None)]
    #[case("tag:institution:bank-a", "tag:bank-a", Some("tag:institution:bank-a"))]
    #[case(
        "account:\"Liabilities:Credit Card\"",
        "account:\"Credit Card\"",
        Some("account:\"Liabilities:Credit Card\"")
    )]
    #[case(
        "(account:Assets:Bank or account:Groceries)",
        "account:Bank or account:Groceries",
        Some("account:Assets:Bank or account:Expenses:Food:Groceries")
    )]
    #[case("coffee", "coffee", None)]
    #[case(
        "@km:>1 x",
        "@km:>1",
        Some("2 values of '@km' are not numbers and were not compared")
    )]
    fn path_chips_show_the_shortest_ending(
        #[case] query: &str,
        #[case] label: &str,
        #[case] title: Option<&str>,
    ) {
        let first = chips(query).into_iter().next().expect("a chip");
        assert_eq!(
            (first.label.as_str(), first.title.as_deref()),
            (label, title)
        );
    }

    #[test]
    fn before_the_catalog_loads_chips_show_their_text_only() {
        let got = chips_from_filter::<Snapshot>(&filter("account:Expenses:Food:Groceries"), None);
        assert_eq!(
            got,
            vec![Chip {
                key: "0:account:Expenses:Food:Groceries".to_owned(),
                label: "account:Expenses:Food:Groceries".to_owned(),
                title: None,
                text: "account:Expenses:Food:Groceries".to_owned(),
                severity: None,
            }]
        );
    }

    #[test]
    fn a_chip_the_catalog_no_longer_resolves_shows_the_error() {
        let got = chips("account:Assets:Gone status:reconciled");
        assert_eq!(
            got.iter().map(|c| c.severity).collect::<Vec<_>>(),
            vec![Some(Severity::Error), None]
        );
        assert!(
            got.first()
                .and_then(|c| c.title.as_deref())
                .is_some_and(|t| t.contains("no account matches 'Assets:Gone'")),
            "{got:?}"
        );
    }

    #[rstest]
    #[case("status:reconciled tag:me status:flagged")]
    #[case("account:Assets:Bank tag:me account:Expenses:Food")]
    fn a_contradiction_marks_both_ends_and_spares_the_middle(#[case] query: &str) {
        let got = chips(query);
        assert_eq!(
            got.iter().map(|c| c.severity).collect::<Vec<_>>(),
            vec![Some(Severity::Warning), None, Some(Severity::Warning)]
        );
    }

    #[test]
    fn an_unknown_field_chip_carries_its_warning() {
        let got = chips("acount:x status:reconciled"); // spellchecker:disable-line
        assert_eq!(
            got.iter().map(|c| c.severity).collect::<Vec<_>>(),
            vec![Some(Severity::Warning), None]
        );
        assert!(
            got.first()
                .and_then(|c| c.title.as_deref())
                .is_some_and(|t| t.contains("is ignored")),
            "{got:?}"
        );
    }

    #[test]
    fn a_hint_shows_on_hover_without_a_severity() {
        let first = chips("-account:Assets:Bank")
            .into_iter()
            .next()
            .expect("a chip");
        assert_eq!(first.severity, None);
        assert!(
            first.title.as_deref().is_some_and(|t| t.contains("-any:(")),
            "{first:?}"
        );
    }

    #[test]
    fn a_query_failure_quotes_each_span() {
        assert_eq!(
            query_failure(
                "@pyee:x (open",
                &[
                    QueryProblem::new("unknown key '@pyee'", 0, 5),
                    QueryProblem::new("unclosed '('", 8, 9),
                    QueryProblem::new("somewhere", 99, 120),
                ]
            ),
            "The filter didn\u{2019}t run: unknown key '@pyee' (at \u{201c}@pyee\u{201d}); \
             unclosed '(' (at \u{201c}(\u{201d}); somewhere."
        );
    }

    #[test]
    fn a_load_error_names_query_problems_and_keeps_others() {
        let query = BcError::Query(vec![QueryProblem::new("unknown key '@pyee'", 0, 5)]);
        assert_eq!(
            load_error_text("Couldn't load transactions", &query, "@pyee:x"),
            "The filter didn\u{2019}t run: unknown key '@pyee' (at \u{201c}@pyee\u{201d})."
        );
        let other = BcError::Internal("disk full".to_owned());
        assert_eq!(
            load_error_text("Couldn't load transactions", &other, "@pyee:x"),
            format!("Couldn't load transactions: {other}")
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
