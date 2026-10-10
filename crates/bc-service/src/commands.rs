//! Command bodies, one sub-module per domain; [`crate::dispatch`] routes names
//! to them.
//!
//! All commands return `Result<T, bc_ipc::BcError>` where `T` is a type from
//! `bc_ipc`; `bc-ui` never sees `bc-core` types.

pub mod accounts;
pub mod backup;
pub mod budget;
pub mod commodities;
pub mod metadata;
pub mod plugins;
pub mod query;
pub mod settings;
pub mod sources;
pub mod tags;
pub mod transfers;
