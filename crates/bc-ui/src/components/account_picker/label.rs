//! An account path rendered with a muted prefix, an emphasised leaf and the
//! query's hit highlighted.

use leptos::prelude::*;
use stylance::import_style;

use super::matching::match_segments;
use super::matching::split_leaf;

import_style!(style, "label.module.scss");

/// Renders `path` as prefix and leaf, marking the first case-insensitive
/// occurrence of `query` within each part.
///
/// # Arguments
///
/// * `path` - The ` :: `-joined account path.
/// * `query` - The user's search text; empty highlights nothing.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos props must take String for default value support; the prop is used by reference internally"
)]
pub fn AccountPathLabel(
    /// The ` :: `-joined account path.
    #[prop(into)]
    path: String,
    /// The user's search text.
    #[prop(into)]
    query: String,
) -> impl IntoView {
    let (prefix, leaf) = split_leaf(&path);
    let runs = |s: &str| {
        match_segments(s, &query)
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
    let prefix_runs = runs(&prefix);
    let leaf_runs = runs(&leaf);
    view! {
        <span class=style::label>
            <span class=style::prefix>{prefix_runs}</span>
            <span class=style::leaf>{leaf_runs}</span>
        </span>
    }
}
