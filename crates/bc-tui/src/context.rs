//! [`TuiContext`] — bc-core services plus a tokio handle for blocking calls.
//!
//! The tui-realm event loop is synchronous. All bc-core services are async.
//! [`TuiContext::block_on`] bridges the two by calling
//! [`tokio::runtime::Handle::block_on`], which is safe from a non-async thread.

use core::future::Future;
use std::path::Path;

use tokio::runtime::Handle;

/// Holds all bc-core services and a tokio runtime handle.
///
/// Constructed once in `main()` before the TUI starts, then shared across
/// all screens via `Arc<TuiContext>`.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as context::TuiContext; repetition is intentional"
)]
#[expect(
    clippy::partial_pub_fields,
    reason = "handle is intentionally private to prevent external use of the runtime"
)]
#[non_exhaustive]
pub struct TuiContext {
    /// Account service.
    pub accounts: bc_core::AccountService,
    /// Transaction service.
    pub transactions: bc_core::TransactionService,
    /// Balance calculation engine.
    pub balances: bc_core::BalanceEngine,
    /// Import profile service.
    pub profiles: bc_core::ImportProfileService,
    /// Asset valuation and depreciation service.
    pub assets: bc_core::AssetService,
    /// Loan terms and amortization service.
    pub loans: bc_core::LoanService,
    /// Budget envelope service.
    pub envelopes: bc_core::EnvelopeService,
    /// Budget calculation engine.
    pub budget: bc_core::BudgetEngine,
    /// Tokio runtime handle — used by [`Self::block_on`] to bridge async bc-core calls.
    handle: Handle,
}

impl TuiContext {
    /// Open the database at `db_path`, run migrations, and construct all services.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or migrations fail.
    #[inline]
    pub async fn open(db_path: &Path) -> bc_core::BcResult<Self> {
        let pool = bc_core::open_db_at(db_path).await?;
        Ok(Self {
            accounts: bc_core::AccountService::new(pool.clone()),
            transactions: bc_core::TransactionService::new(pool.clone()),
            balances: bc_core::BalanceEngine::new(pool.clone()),
            profiles: bc_core::ImportProfileService::new(pool.clone()),
            assets: bc_core::AssetService::new(pool.clone()),
            loans: bc_core::LoanService::new(pool.clone()),
            envelopes: bc_core::EnvelopeService::new(pool.clone()),
            budget: bc_core::BudgetEngine::new(pool),
            handle: Handle::current(),
        })
    }

    /// Run an async bc-core call, blocking the current thread until complete.
    ///
    /// Safe to call from within the synchronous tui-realm event loop because
    /// [`Handle::block_on`] does not require being called from within an async
    /// context — only that a runtime exists on another thread.
    ///
    /// # Panics
    ///
    /// Panics if called from within an async execution context (e.g., inside
    /// a `tokio::spawn` or `#[tokio::test]`). Use `.await` directly in tests.
    #[inline]
    pub fn block_on<F: Future>(&self, f: F) -> F::Output {
        self.handle.block_on(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_creates_all_services() {
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("test.db");
        // If open succeeds without error, all services were created and
        // migrations ran successfully.
        let _ctx = TuiContext::open(&db_path).await.expect("open test context");
    }
}
