//! Shared IPC types for the BorrowChecker Tauri application.
//!
//! This crate is the only channel between the native Tauri backend (`bc-app`)
//! and the WASM frontend (`bc-ui`). It must compile to both
//! `wasm32-unknown-unknown` and native targets — zero native-only dependencies.
//!
//! # Type conventions
//!
//! - Monetary amounts use [`Money`] — a currency-aware minor-unit value
//! - IDs use `String` — newtype IDs serialise to their string representation
//! - All public enums carry `#[non_exhaustive]` for forward compatibility
//! - All types implement `Send + Sync`, `Serialize`, `Deserialize`, `Clone`,
//!   `Debug`

mod accounts;
#[cfg(target_arch = "wasm32")]
pub mod client;
pub mod commands;
mod currency;
mod error;
mod money;

pub use accounts::AccountNode;
pub use accounts::AccountType;
pub use accounts::AuditEntry;
pub use accounts::NewPosting;
pub use accounts::NewTransaction;
pub use accounts::Posting;
pub use accounts::Transaction;
pub use accounts::TxStatus;
pub use currency::AUD;
pub use currency::BTC;
pub use currency::Currency;
pub use currency::ETH;
pub use currency::EUR;
pub use currency::GBP;
pub use currency::INR;
pub use currency::JPY;
pub use currency::KRW;
pub use currency::USD;
pub use currency::currency_from_code;
pub use error::Error as BcError;
pub use money::Money;
