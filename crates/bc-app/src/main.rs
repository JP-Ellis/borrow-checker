//! BorrowChecker desktop GUI — application entry point.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    bc_app::run();
}
