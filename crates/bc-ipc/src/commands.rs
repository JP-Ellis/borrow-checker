//! Command name constants shared between the Tauri backend and the WASM client.
//!
//! Both sides reference the same constant strings so a rename on either end
//! produces a compile error rather than a silent runtime mismatch.

/// Command: list all active accounts.
pub const LIST_ACCOUNTS: &str = "list_accounts";

/// Command: list transactions for a specific account.
pub const LIST_TRANSACTIONS: &str = "list_transactions";

/// Command: create a new transaction.
pub const CREATE_TRANSACTION: &str = "create_transaction";
