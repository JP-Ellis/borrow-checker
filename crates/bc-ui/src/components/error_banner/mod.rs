//! Inline error banner component.

use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "error_banner.module.scss");

/// Displays an inline error message with `bad` semantic styling.
///
/// Use this whenever a data-fetch or command fails — it is visually distinct
/// from the empty-state paragraph so users can distinguish "nothing here" from
/// "something went wrong".
///
/// # Arguments
///
/// * `message` - The error string to display. Keep it short; one sentence.
#[component]
pub fn ErrorBanner(
    /// The error message to display.
    #[prop(into)]
    message: String,
) -> impl IntoView {
    view! {
        <p role="alert" class=style::banner>
            {message}
        </p>
    }
}

#[cfg(debug_assertions)]
pub mod qa;
