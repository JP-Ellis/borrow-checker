//! Transaction delete and import-provenance commands.
#![expect(
    clippy::module_name_repetitions,
    reason = "command names are the IPC contract"
)]

use bc_core::ipc::RejectedRowExt as _;
use bc_core::ipc::TransactionProvenanceExt as _;
use bc_models::AccountId;

use crate::AppState;

/// Widens a count for the wire.
fn to_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Parses an account id argument.
fn parse_account_id(raw: &str) -> Result<AccountId, bc_ipc::BcError> {
    raw.parse::<AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account id: {e}")))
}

/// Deletes a transaction, keeping or forgetting its import provenance.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for an unparsable id and
/// [`bc_ipc::BcError::NotFound`] for an unknown transaction.
pub async fn delete_transaction(
    state: &AppState,
    args: bc_ipc::commands::DeleteTransactionArgs,
) -> Result<bc_ipc::DeleteOutcome, bc_ipc::BcError> {
    let bc_ipc::commands::DeleteTransactionArgs {
        id,
        forget_provenance,
        ..
    } = args;
    let tx_id = id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid id: {e}")))?;
    let mode = if forget_provenance {
        bc_core::DeleteMode::ForgetProvenance
    } else {
        bc_core::DeleteMode::KeepProvenance
    };
    let outcome = state.transactions.delete(&tx_id, mode).await?;
    Ok(bc_ipc::DeleteOutcome::new(
        to_u64(outcome.references_kept),
        to_u64(outcome.references_forgotten),
    ))
}

/// Summarises the import provenance a transaction carries, naming each
/// account by its path.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for an unparsable id.
pub async fn transaction_provenance(
    state: &AppState,
    args: bc_ipc::commands::TransactionProvenanceArgs,
) -> Result<bc_ipc::TransactionProvenance, bc_ipc::BcError> {
    let bc_ipc::commands::TransactionProvenanceArgs { id, .. } = args;
    let tx_id = id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid id: {e}")))?;
    let summary = state.sources.summary(&tx_id).await?;
    let resolver = bc_core::AccountResolver::load(&state.accounts).await?;
    Ok(bc_ipc::TransactionProvenance::from_summary(
        &summary, &resolver,
    ))
}

/// Lists rejected statement rows, optionally for one account, naming each
/// account by its path.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for an unparsable account id.
pub async fn list_rejected_sources(
    state: &AppState,
    args: bc_ipc::commands::ListRejectedSourcesArgs,
) -> Result<Vec<bc_ipc::RejectedRow>, bc_ipc::BcError> {
    let bc_ipc::commands::ListRejectedSourcesArgs { account, .. } = args;
    let account_id = account.as_deref().map(parse_account_id).transpose()?;
    let rows = state.sources.rejected(account_id.as_ref()).await?;
    let resolver = bc_core::AccountResolver::load(&state.accounts).await?;
    Ok(rows
        .iter()
        .map(|row| bc_ipc::RejectedRow::from_core(row, &resolver))
        .collect())
}

/// Releases rejected statement rows, returning how many references were released.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a target that parses as
/// neither id, and for a reference that still backs a live posting or names
/// a leg of a deleted transaction. Returns [`bc_ipc::BcError::NotFound`] for
/// an unknown reference and for a transaction id with no rejected rows,
/// including a live transaction's id. No target is released on error.
pub async fn release_rejected_sources(
    state: &AppState,
    args: bc_ipc::commands::ReleaseRejectedSourcesArgs,
) -> Result<u64, bc_ipc::BcError> {
    let bc_ipc::commands::ReleaseRejectedSourcesArgs { targets, .. } = args;
    let parsed = targets
        .iter()
        .map(|raw| raw.parse::<bc_core::ReleaseTarget>())
        .collect::<Result<Vec<_>, _>>()?;
    let released = state.sources.release(&parsed).await?;
    Ok(to_u64(released))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::commands;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::SourceRef;
    use bc_models::SourceRefId;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;
    use serde_json::Value;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    /// A ledger with `Assets:Everyday` and `Assets:Savings`, and one
    /// transaction moving 50 AUD between them.
    struct Ledger {
        /// Keeps the database alive.
        _dir: TempDir,
        state: AppState,
        everyday: AccountId,
        savings: AccountId,
        assets: AccountId,
        tx: TransactionId,
        postings: Vec<(PostingId, AccountId, Amount)>,
    }

    /// Creates a deposit account under an optional parent.
    async fn account(state: &AppState, name: &str, parent: Option<&AccountId>) -> AccountId {
        state
            .accounts
            .create()
            .name(name)
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .maybe_parent_id(parent)
            .call()
            .await
            .expect("create account")
    }

    /// Opens a fresh ledger holding one hand-entered transaction.
    async fn ledger() -> Ledger {
        let dir = tempfile::tempdir().expect("tempdir");
        // SAFETY: nextest runs each test in its own process, and this runs
        // before the test opens the database or spawns any thread that reads
        // the environment.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config")) }
        let mut settings = bc_config::Settings::default();
        settings.set_db_path(dir.path().join("ledger.db"));
        settings.set_backup_dir(dir.path().join("backups"));
        let state = AppState::open(&settings).await.expect("open");

        let assets = account(&state, "Assets", None).await;
        let everyday = account(&state, "Everyday", Some(&assets)).await;
        let savings = account(&state, "Savings", Some(&assets)).await;
        let aud = |value| Amount::new(value, CommodityCode::new("AUD"));
        let postings = vec![
            (PostingId::new(), savings.clone(), aud(dec!(50.00))),
            (PostingId::new(), everyday.clone(), aud(dec!(-50.00))),
        ];
        let draft = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 3, 1))
            .description("Transfer to savings")
            .postings(
                postings
                    .iter()
                    .map(|(id, account_id, amount)| {
                        Posting::builder()
                            .id(id.clone())
                            .account_id(account_id.clone())
                            .amount(amount.clone())
                            .build()
                    })
                    .collect(),
            )
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(jiff::Timestamp::now())
            .build();
        let tx = state
            .transactions
            .create(draft)
            .await
            .expect("create transaction")
            .into_inner();

        Ledger {
            _dir: dir,
            state,
            everyday,
            savings,
            assets,
            tx,
            postings,
        }
    }

    impl Ledger {
        /// Attaches an import reference to every posting, as an import would.
        async fn import_every_leg(&self) {
            for (posting, account_id, amount) in &self.postings {
                let source = SourceRef::builder()
                    .id(SourceRefId::new())
                    .transaction_id(self.tx.clone())
                    .posting_id(Some(posting.clone()))
                    .account_id(account_id.clone())
                    .date(date(2026, 3, 1))
                    .narration("TRANSFER")
                    .amount(Some(amount.clone()))
                    .reference(None)
                    .occurrence(0)
                    .import_batch_id(None)
                    .owns_posting(true)
                    .created_at(jiff::Timestamp::now())
                    .build();
                self.state.sources.attach(&source).await.expect("attach");
            }
        }

        /// Dispatches `cmd` and decodes its answer.
        async fn call<T>(&self, cmd: &str, args: Value) -> T
        where
            T: serde::de::DeserializeOwned,
        {
            let value = crate::dispatch(&self.state, cmd, args)
                .await
                .expect("command succeeds");
            serde_json::from_value(value).expect("decode answer")
        }

        /// Deletes the transaction, keeping or forgetting its provenance.
        async fn delete(&self, forget_provenance: bool) -> bc_ipc::DeleteOutcome {
            self.call(
                commands::DELETE_TRANSACTION,
                json!({ "id": self.tx.to_string(), "forget_provenance": forget_provenance }),
            )
            .await
        }

        /// Lists rejected rows, optionally for one account.
        async fn rejected(&self, account: Option<&AccountId>) -> Vec<bc_ipc::RejectedRow> {
            self.call(
                commands::LIST_REJECTED_SOURCES,
                json!({ "account": account.map(ToString::to_string) }),
            )
            .await
        }
    }

    #[tokio::test]
    async fn a_hand_entry_has_no_provenance() {
        let ledger = ledger().await;
        let provenance: bc_ipc::TransactionProvenance = ledger
            .call(
                commands::TRANSACTION_PROVENANCE,
                json!({ "id": ledger.tx.to_string() }),
            )
            .await;
        assert_eq!(provenance, bc_ipc::TransactionProvenance::new(0, vec![]));
    }

    #[tokio::test]
    async fn an_import_names_its_accounts_by_path() {
        let ledger = ledger().await;
        ledger.import_every_leg().await;
        let provenance: bc_ipc::TransactionProvenance = ledger
            .call(
                commands::TRANSACTION_PROVENANCE,
                json!({ "id": ledger.tx.to_string() }),
            )
            .await;
        assert_eq!(
            provenance,
            bc_ipc::TransactionProvenance::new(
                2,
                vec!["Assets:Savings".to_owned(), "Assets:Everyday".to_owned()]
            )
        );
    }

    #[rstest]
    #[case::keep(false, bc_ipc::DeleteOutcome::new(2, 0))]
    #[case::forget(true, bc_ipc::DeleteOutcome::new(0, 2))]
    #[tokio::test]
    async fn deleting_an_import_reports_its_references(
        #[case] forget_provenance: bool,
        #[case] expected: bc_ipc::DeleteOutcome,
    ) {
        let ledger = ledger().await;
        ledger.import_every_leg().await;
        assert_eq!(ledger.delete(forget_provenance).await, expected);
    }

    #[tokio::test]
    async fn a_kept_delete_lists_one_rejected_transaction() {
        let ledger = ledger().await;
        ledger.import_every_leg().await;
        ledger.delete(false).await;

        let rows = ledger.rejected(None).await;

        let [
            bc_ipc::RejectedRow::Transaction {
                deleted_transaction_id,
                legs,
            },
        ] = rows.as_slice()
        else {
            panic!("expected one rejected transaction, got {rows:?}");
        };
        assert_eq!(*deleted_transaction_id, ledger.tx.to_string());
        let mut seen: Vec<(&str, Option<bc_ipc::Amount>)> = legs
            .iter()
            .map(|leg| (leg.account.as_str(), leg.amount.clone()))
            .collect();
        seen.sort_by(|a, b| a.0.cmp(b.0));
        assert_eq!(
            seen,
            vec![
                (
                    "Assets:Everyday",
                    Some(bc_ipc::Amount::new(dec!(-50.00), "AUD"))
                ),
                (
                    "Assets:Savings",
                    Some(bc_ipc::Amount::new(dec!(50.00), "AUD"))
                ),
            ]
        );
    }

    #[tokio::test]
    async fn the_account_filter_keeps_rows_with_a_leg_on_it() {
        let ledger = ledger().await;
        ledger.import_every_leg().await;
        ledger.delete(false).await;

        assert_eq!(ledger.rejected(Some(&ledger.everyday)).await.len(), 1);
        assert_eq!(ledger.rejected(Some(&ledger.savings)).await.len(), 1);
        assert_eq!(ledger.rejected(Some(&ledger.assets)).await, vec![]);
    }

    #[tokio::test]
    async fn a_forgotten_delete_lists_nothing() {
        let ledger = ledger().await;
        ledger.import_every_leg().await;
        ledger.delete(true).await;

        assert_eq!(ledger.rejected(None).await, vec![]);
    }
}
