//! BorrowChecker desktop GUI — Tauri application library.
//!
//! Exposes [`run`], called by both `main.rs` (desktop) and the Tauri mobile
//! entry point (future work). All command handlers are registered here.

pub mod commands;

/// Initialise and run the Tauri application.
///
/// # Panics
///
/// Panics if Tauri cannot initialise the `WebView` runtime. This is
/// unrecoverable for a desktop GUI.
#[expect(
    clippy::expect_used,
    reason = "Tauri startup failure is unrecoverable for a desktop GUI"
)]
#[expect(
    clippy::exit,
    reason = "tauri::generate_context!() macro internally calls process::exit"
)]
#[inline]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![])
        .setup(|_app| Ok(()))
        .run(tauri::generate_context!())
        .expect("error while running borrow-checker");
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
