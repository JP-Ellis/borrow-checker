// The target the picker's "manage accounts ↗" link opens in.
//
// Leptos-free and also mounted natively via `include!` in `main.rs`'s
// `components_tests` shim, which is why this header uses `//` rather than
// `//!`.

/// Whether this build opens the link in a new tab: the web build does, so
/// the form around the picker keeps its unsaved input. The Tauri webview
/// has no new-window handler and would drop a `_blank` link, so the desktop
/// build navigates in place.
pub const NEW_TAB: bool = cfg!(feature = "http");

/// The link's `target` and `rel` attributes.
///
/// # Arguments
///
/// * `new_tab` - Whether the link opens a new tab.
///
/// # Returns
///
/// `("_blank", "noopener")` for a new tab; neither attribute otherwise.
#[must_use]
pub const fn attrs(new_tab: bool) -> (Option<&'static str>, Option<&'static str>) {
    if new_tab {
        (Some("_blank"), Some("noopener"))
    } else {
        (None, None)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn a_new_tab_link_is_blank_and_noopener() {
        assert_eq!(attrs(true), (Some("_blank"), Some("noopener")));
    }

    #[test]
    fn an_in_place_link_has_no_target() {
        assert_eq!(attrs(false), (None, None));
    }

    /// Native tests build without `http`, as the desktop bundle does.
    #[cfg(not(feature = "http"))]
    #[test]
    fn the_desktop_build_navigates_in_place() {
        assert_eq!(attrs(NEW_TAB), (None, None));
    }
}
