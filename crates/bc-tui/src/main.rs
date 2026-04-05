//! BorrowChecker TUI — keyboard-first terminal interface built with ratatui.

// Scaffold in progress — modules are forward-declared but not all wired yet.
// Removed in Task 7 once the full main loop connects every module.
#![allow(
    dead_code,
    reason = "scaffold in progress — removed in Task 7 once the main loop wires all modules"
)]

mod context;
mod id;
mod mode;
mod msg;
mod screen;

#[expect(clippy::print_stdout, reason = "stub placeholder")]
fn main() {
    println!("stub");
}
