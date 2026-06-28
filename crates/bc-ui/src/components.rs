//! Primitive design-system components.
//!
//! All monetary values must use [`num::Num`]. Every screen is built from these
//! atoms — resist inventing alternatives.

pub mod account_picker;
#[cfg(target_arch = "wasm32")]
pub mod error_banner;
#[cfg(target_arch = "wasm32")]
pub mod num;
pub mod sparkline;
#[cfg(target_arch = "wasm32")]
pub mod stat_card;
pub mod status_pill;
pub mod tag_picker;
pub mod tag_token;
pub mod transaction_row;
