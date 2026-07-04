//! Tauri command handlers.
//!
//! One sub-module per domain (accounts, transactions, budget, etc.) — added in
//! Phase 2 plans. Commands are registered in [`crate::run`] via
//! `tauri::generate_handler!`.
//!
//! All commands return `Result<T, bc_ipc::BcError>` where `T` is a type from
//! `bc_ipc`. `bc-app` is the only crate allowed to import both `bc-core` and
//! `bc-ipc`; `bc-ui` must never see `bc-core` types.

pub mod accounts;
pub mod backup;
pub mod budget;
pub mod commodities;
pub mod plugins;
pub mod settings;
pub mod tags;
