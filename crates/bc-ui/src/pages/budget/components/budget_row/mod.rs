//! Single row in the budget allocation tree.
// `#![expect(dead_code)]` does not work here: the `#[component]` macro generates a struct
// outside the function body, and rustc reports the expectation as unfulfilled even though
// the lint fires. `#![allow]` is the only functional suppression in this context.
// See: https://github.com/rust-lang/rust/issues/unfulfilled-expect-proc-macro
#![allow(
    dead_code,
    reason = "stub component — fields will be read when this component is implemented"
)]

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "row.module.scss");

/// One row in the budget allocation grid, representing a single budget line.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    unused_variables,
    reason = "stub — props will be consumed in later tasks"
)]
pub fn BudgetRow(
    /// The tree node this row represents.
    node: BudgetTreeNode,
) -> impl IntoView {
    view! { <div class=style::root /> }
}
