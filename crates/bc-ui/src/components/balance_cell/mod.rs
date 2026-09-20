//! Running-balance cell: the formatted amount over an in-cell bar on a shared
//! per-commodity axis.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

pub mod geometry;
