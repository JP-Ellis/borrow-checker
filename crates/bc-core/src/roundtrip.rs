//! Round-trip export and apply: export snapshots, short transaction handles,
//! the sidecar file that maps them back to full transactions, and the plan
//! an apply of an edited export follows.

pub mod apply;
pub mod handle;
pub mod plan;
pub mod resolve;
pub mod sidecar;
pub mod snapshot;

pub use apply::ApplyOutcome;
pub use apply::Refusal;
use bc_models::Transaction;
pub use handle::abbreviate;
use jiff::Timestamp;
pub use plan::Action;
pub use plan::ApplyPlan;
pub use plan::PreflightError;
pub use plan::SkipReason;
pub use plan::plan;
pub use resolve::Resolver;
pub use resolve::Unresolved;
pub use sidecar::Sidecar;
pub use sidecar::SidecarEntry;
pub use sidecar::database_id;
pub use snapshot::spellable_metadata;
use sqlx::SqlitePool;

use crate::AccountService;
use crate::BcResult;
use crate::BudgetService;
use crate::CommodityService;
use crate::TagService;
use crate::TransactionService;
use crate::export::Level;
use crate::export::Snapshot;
use crate::search::TransactionQuery;

/// What an export produces: the snapshot a plugin formats and, at working
/// level, the sidecar that apply resolves handles through.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Export {
    /// The ledger rendered for the exporter plugin.
    pub snapshot: Snapshot,
    /// The exported transactions by handle. `None` at archive level, where
    /// handles are full IDs.
    pub sidecar: Option<Sidecar>,
}

/// Builds export snapshots from a transaction query.
#[derive(Debug, Clone)]
pub struct RoundTripService {
    /// The SQLite connection pool, read for the database identity.
    pool: SqlitePool,
    /// Runs the query.
    transactions: TransactionService,
    /// Supplies the account tree.
    accounts: AccountService,
    /// Supplies commodity details.
    commodities: CommodityService,
    /// Supplies tag paths.
    tags: TagService,
    /// Supplies budgets and their revisions.
    budgets: BudgetService,
}

impl RoundTripService {
    /// Creates a service over `pool`.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            transactions: TransactionService::new(pool.clone()),
            accounts: AccountService::new(pool.clone()),
            commodities: CommodityService::new(pool.clone()),
            tags: TagService::new(pool.clone()),
            budgets: BudgetService::new(pool.clone()),
            pool,
        }
    }

    /// Exports the transactions `query` selects at `level`.
    ///
    /// A working export holds the selected transactions, the accounts and
    /// commodities they reference, short handles and no posting handles, plus
    /// a sidecar recording every selected transaction. An archive export holds
    /// every account, commodity and budget, archived included, the selected transactions
    /// under their full IDs with posting IDs as posting handles, and month-end
    /// balances for each account with activity. It has no sidecar.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] naming each transaction and key
    /// when a selected transaction or posting carries a user metadata key the
    /// round trip reserves (`bc`, or one starting `bc-`), and another
    /// [`crate::BcError`] on database or data failure.
    #[inline]
    pub async fn export(&self, query: &TransactionQuery, level: Level) -> BcResult<Export> {
        let transactions: Vec<Transaction> = self
            .transactions
            .search(query)
            .await?
            .into_iter()
            .map(|matched| matched.transaction)
            .collect();
        snapshot::refuse_reserved_keys(&transactions)?;
        let catalog = snapshot::Catalog::new(
            self.accounts.list_all().await?,
            crate::AccountResolver::load(&self.accounts).await?,
            self.commodities.list_all().await?,
            self.tags.forest().await?,
        );

        match level {
            Level::Working => {
                let ids: Vec<_> = transactions.iter().map(|tx| tx.id().clone()).collect();
                let handles = abbreviate(&ids);
                let snapshot = snapshot::working(&transactions, &handles, &catalog)?;
                let entries = transactions
                    .into_iter()
                    .map(|tx| {
                        let handle = handles
                            .get(tx.id())
                            .cloned()
                            .unwrap_or_else(|| tx.id().to_string());
                        SidecarEntry::new(handle, tx)
                    })
                    .collect();
                let sidecar =
                    Sidecar::new(database_id(&self.pool).await?, Timestamp::now(), entries);
                Ok(Export {
                    snapshot,
                    sidecar: Some(sidecar),
                })
            }
            Level::Archive => {
                let mut budgets = Vec::new();
                for budget in self.budgets.list_all().await? {
                    let revisions = self.budgets.revisions(budget.id()).await?;
                    budgets.push((budget, revisions));
                }
                Ok(Export {
                    snapshot: snapshot::archive(&transactions, &budgets, &catalog)?,
                    sidecar: None,
                })
            }
        }
    }
}
