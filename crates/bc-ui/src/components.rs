//! Primitive design-system components.
//!
//! All monetary values must use [`num::Num`]. Every screen is built from these
//! atoms — resist inventing alternatives.

pub mod error_banner;
pub mod num;
pub mod sparkline;
pub mod stat_card;
pub mod status_pill;
pub mod tag_token;
pub mod toml_view;
pub mod transaction_row;
