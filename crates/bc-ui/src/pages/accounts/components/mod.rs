//! Sub-components of the accounts page.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

pub mod add_transaction;
#[cfg(target_arch = "wasm32")]
pub mod sidebar;
#[cfg(target_arch = "wasm32")]
pub mod sticky_bar;
#[cfg(target_arch = "wasm32")]
pub mod transaction_register;
