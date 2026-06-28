//! Route entry for `/__test/component/account-picker`.

pub use crate::components::account_picker::qa::AccountPickerQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "AccountPicker";
/// Route path.
pub const PATH: &str = "/__test/component/account-picker";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Account autocomplete with substring filtering.";
