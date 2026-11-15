//! Command handlers for account and transaction operations.
#![expect(
    clippy::module_name_repetitions,
    reason = "command names are the IPC contract"
)]

use core::num::NonZeroUsize;
use std::collections::HashMap;
use std::collections::HashSet;

use bc_core::ipc::AccountDetailExt as _;
use bc_core::ipc::AccountNodeExt as _;
use bc_core::ipc::AuditEntryExt as _;
use bc_core::ipc::TransactionExt as _;

use crate::AppState;

// MARK: Scope helper

/// Resolves the account set a command operates on.
///
/// # Arguments
///
/// * `state` - Shared application state.
/// * `id` - The selected account.
/// * `include_descendants` - When set, the account plus its active subtree.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] if the subtree lookup fails.
async fn scope_ids(
    state: &AppState,
    id: &bc_models::AccountId,
    include_descendants: bool,
) -> Result<Vec<bc_models::AccountId>, bc_ipc::BcError> {
    if !include_descendants {
        return Ok(vec![id.clone()]);
    }
    state
        .accounts
        .subtree_ids(id)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
}

// MARK: Display context

/// The active accounts and tag forest a transaction needs for display.
struct DisplayContext {
    /// Active accounts, in the order the service lists them.
    accounts: Vec<bc_models::Account>,
    /// Every tag, for resolving tag ids to paths.
    forest: bc_models::TagForest,
}

impl DisplayContext {
    /// Loads the active accounts and the tag forest.
    ///
    /// # Arguments
    ///
    /// * `state` - Shared application state.
    ///
    /// # Errors
    ///
    /// Returns [`bc_ipc::BcError::Internal`] if either lookup fails.
    async fn load(state: &AppState) -> Result<Self, bc_ipc::BcError> {
        let accounts = state
            .accounts
            .list_active()
            .await
            .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
        let forest = state
            .tags
            .forest()
            .await
            .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
        Ok(Self { accounts, forest })
    }

    /// Returns a converter from a domain transaction to its display form.
    ///
    /// The account map is built once, here, and shared by every call.
    ///
    /// # Returns
    ///
    /// A function converting one transaction.
    fn transaction_converter(
        &self,
    ) -> impl Fn(&bc_models::Transaction) -> bc_ipc::Transaction + '_ {
        let account_map = self
            .accounts
            .iter()
            .map(|a| (a.id().to_string(), a))
            .collect::<std::collections::HashMap<_, _>>();
        move |tx| bc_ipc::Transaction::from_model_with_accounts(tx, &account_map, &self.forest)
    }
}

// MARK: Command handlers

/// List all accounts as a tree of nodes.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the service call fails.
pub async fn list_accounts(state: &AppState) -> Result<Vec<bc_ipc::AccountNode>, bc_ipc::BcError> {
    let ctx = DisplayContext::load(state).await?;

    let totals = state
        .balance_engine
        .account_totals()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let balances: HashMap<_, _> = totals.defaults().collect();
    let rollups: HashMap<_, _> = totals
        .rollups()
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?
        .collect();

    let nodes = ctx
        .accounts
        .iter()
        .map(|account| {
            let balance = balances.get(account.id()).map(bc_ipc::Amount::from);
            let default_code = balance.as_ref().map(|b| b.currency_code.clone());
            let rollup = rollups
                .get(account.id())
                .map_or_default(|b| ordered_amounts(b, default_code.as_deref()));
            bc_ipc::AccountNode::from_model(account, &ctx.forest, balance).with_rollup(rollup)
        })
        .collect::<Vec<_>>();

    Ok(nodes)
}

// MARK: Account management

/// Parses an account ID from the wire.
///
/// # Arguments
///
/// * `raw` - The ID string.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if `raw` is not an account ID.
fn parse_account_id(raw: &str) -> Result<bc_models::AccountId, bc_ipc::BcError> {
    raw.parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account id '{raw}': {e}")))
}

/// Resolves commodity codes to registered commodity IDs, keeping their order.
///
/// # Arguments
///
/// * `state` - Shared application state.
/// * `codes` - The codes to resolve.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] naming the first unregistered
/// code, or [`bc_ipc::BcError::Internal`] if the registry fails to load.
async fn resolve_commodity_codes(
    state: &AppState,
    codes: &[String],
) -> Result<Vec<bc_models::CommodityId>, bc_ipc::BcError> {
    if codes.is_empty() {
        return Ok(Vec::new());
    }
    let registry = state
        .commodities
        .list_all()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    let by_code: HashMap<&str, &bc_models::CommodityId> =
        registry.iter().map(|c| (c.code(), c.id())).collect();
    codes
        .iter()
        .map(|code| {
            by_code
                .get(code.as_str())
                .map(|id| (*id).clone())
                .ok_or_else(|| bc_ipc::BcError::Validation(format!("unknown commodity '{code}'")))
        })
        .collect()
}

/// Resolves tag ID strings to existing tag IDs, keeping their order.
///
/// # Arguments
///
/// * `state` - Shared application state.
/// * `ids` - The tag ID strings.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] naming the first ID that does not
/// parse or names no tag, or [`bc_ipc::BcError::Internal`] if the hierarchy
/// fails to load.
async fn resolve_tag_ids(
    state: &AppState,
    ids: &[String],
) -> Result<Vec<bc_models::TagId>, bc_ipc::BcError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let forest = state
        .tags
        .forest()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    ids.iter()
        .map(|raw| {
            let id = raw
                .parse::<bc_models::TagId>()
                .map_err(|e| bc_ipc::BcError::Validation(format!("invalid tag id '{raw}': {e}")))?;
            if forest.path_of(&id).is_none() {
                return Err(bc_ipc::BcError::Validation(format!(
                    "unknown tag id '{raw}'"
                )));
            }
            Ok(id)
        })
        .collect()
}

/// Wraps a core failure as [`bc_ipc::BcError::Internal`].
#[expect(
    clippy::needless_pass_by_value,
    reason = "`map_err` hands the error over by value"
)]
fn internal(error: bc_core::BcError) -> bc_ipc::BcError {
    bc_ipc::BcError::Internal(error.to_string())
}

/// Everything an account detail needs beyond the account itself.
struct DetailContext {
    /// Every account, archived included, in name order.
    accounts: Vec<bc_models::Account>,
    /// The event count of each account a detail may be built for, read
    /// before the accounts' fields.
    counts: HashMap<bc_models::AccountId, u64>,
    /// Every tag, for resolving tag IDs to paths.
    forest: bc_models::TagForest,
    /// Commodity code by ID.
    codes: HashMap<bc_models::CommodityId, String>,
}

impl DetailContext {
    /// Loads every account with its event count, the tag forest and the
    /// commodity registry.
    ///
    /// The counts are read before the fields they stand for. A write between
    /// the two reads leaves a count older than the fields, which a later save
    /// refuses as stale; the reverse order would pair old fields with a newer
    /// count and let that save overwrite the write. An account created
    /// between the reads has no count and is left out.
    ///
    /// # Arguments
    ///
    /// * `state` - Shared application state.
    ///
    /// # Errors
    ///
    /// Returns [`bc_ipc::BcError::Internal`] if a lookup fails.
    async fn load(state: &AppState) -> Result<Self, bc_ipc::BcError> {
        let mut counts = HashMap::new();
        for account in state.accounts.list_all().await.map_err(internal)? {
            let count = state
                .accounts
                .event_count(account.id())
                .await
                .map_err(internal)?;
            counts.insert(account.id().clone(), count);
        }
        let mut accounts = state.accounts.list_all().await.map_err(internal)?;
        accounts.retain(|a| counts.contains_key(a.id()));
        Self::finish(state, accounts, counts).await
    }

    /// Loads what the detail of `id` needs: its event count, every account
    /// for its path and parent chain, the tag forest and the commodity
    /// registry.
    ///
    /// The count is read before the fields, as in [`Self::load`]. An account
    /// created between the two reads has a count of zero, older than its
    /// fields, so a save on it is refused as stale.
    ///
    /// # Arguments
    ///
    /// * `state` - Shared application state.
    /// * `id` - The account to describe.
    ///
    /// # Errors
    ///
    /// Returns [`bc_ipc::BcError::Internal`] if a lookup fails.
    async fn load_one(
        state: &AppState,
        id: &bc_models::AccountId,
    ) -> Result<Self, bc_ipc::BcError> {
        let count = state.accounts.event_count(id).await.map_err(internal)?;
        let accounts = state.accounts.list_all().await.map_err(internal)?;
        let counts = HashMap::from([(id.clone(), count)]);
        Self::finish(state, accounts, counts).await
    }

    /// Completes a context over `accounts` and `counts` with the tag forest
    /// and the commodity registry.
    ///
    /// # Arguments
    ///
    /// * `state` - Shared application state.
    /// * `accounts` - The accounts, read after `counts`.
    /// * `counts` - The event counts, read first.
    ///
    /// # Errors
    ///
    /// Returns [`bc_ipc::BcError::Internal`] if a lookup fails.
    async fn finish(
        state: &AppState,
        accounts: Vec<bc_models::Account>,
        counts: HashMap<bc_models::AccountId, u64>,
    ) -> Result<Self, bc_ipc::BcError> {
        let forest = state.tags.forest().await.map_err(internal)?;
        let codes = state
            .commodities
            .list_all()
            .await
            .map_err(internal)?
            .into_iter()
            .map(|c| (c.id().clone(), c.code().to_owned()))
            .collect();
        Ok(Self {
            accounts,
            counts,
            forest,
            codes,
        })
    }

    /// Builds the detail of each account in `accounts`, in their order.
    ///
    /// # Arguments
    ///
    /// * `accounts` - The accounts to describe, drawn from `self.accounts`.
    ///
    /// # Errors
    ///
    /// Returns [`bc_ipc::BcError::Internal`] if an account has no loaded
    /// event count.
    fn details(
        &self,
        accounts: &[&bc_models::Account],
    ) -> Result<Vec<bc_ipc::AccountDetail>, bc_ipc::BcError> {
        let account_map: HashMap<String, &bc_models::Account> = self
            .accounts
            .iter()
            .map(|a| (a.id().to_string(), a))
            .collect();
        let mut out = Vec::with_capacity(accounts.len());
        for account in accounts {
            let event_count = *self.counts.get(account.id()).ok_or_else(|| {
                bc_ipc::BcError::Internal(format!("no event count loaded for {}", account.id()))
            })?;
            out.push(bc_ipc::AccountDetail::from_model(
                account,
                &account_map,
                &self.forest,
                &self.codes,
                event_count,
            ));
        }
        Ok(out)
    }
}

/// Builds the detail of one account.
///
/// # Arguments
///
/// * `state` - Shared application state.
/// * `id` - The account.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::NotFound`] if no account has `id`, or
/// [`bc_ipc::BcError`] if a lookup fails.
async fn detail_of(
    state: &AppState,
    id: &bc_models::AccountId,
) -> Result<bc_ipc::AccountDetail, bc_ipc::BcError> {
    let ctx = DetailContext::load_one(state, id).await?;
    let account = ctx
        .accounts
        .iter()
        .find(|a| a.id() == id)
        .ok_or_else(|| bc_ipc::BcError::NotFound(id.to_string()))?;
    ctx.details(&[account])?
        .pop()
        .ok_or_else(|| bc_ipc::BcError::Internal(format!("no detail built for {id}")))
}

/// Lists every account, archived included, with its editable fields, in the
/// order the sidebar tree shows them.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if a lookup fails.
pub async fn list_account_details(
    state: &AppState,
) -> Result<Vec<bc_ipc::AccountDetail>, bc_ipc::BcError> {
    let ctx = DetailContext::load(state).await?;
    let ordered = bc_core::ipc::accounts_in_tree_order(&ctx.accounts);
    ctx.details(&ordered)
}

/// Fetches one account with its editable fields.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a malformed ID,
/// [`bc_ipc::BcError::NotFound`] for an unknown one, or [`bc_ipc::BcError`]
/// if a lookup fails.
pub async fn get_account_detail(
    state: &AppState,
    args: bc_ipc::commands::GetAccountDetailArgs,
) -> Result<bc_ipc::AccountDetail, bc_ipc::BcError> {
    let bc_ipc::commands::GetAccountDetailArgs { id, .. } = args;
    detail_of(state, &parse_account_id(&id)?).await
}

/// Creates an account and returns its stored detail.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a malformed ID, an unknown
/// commodity code or tag, or a request core refuses (an invalid name, a
/// parent of another type, acquisition fields on an account that is not a
/// manual asset); [`bc_ipc::BcError::NotFound`] for an unknown parent; or
/// [`bc_ipc::BcError`] if a write or lookup fails.
pub async fn create_account(
    state: &AppState,
    args: bc_ipc::commands::CreateAccountArgs,
) -> Result<bc_ipc::AccountDetail, bc_ipc::BcError> {
    let bc_ipc::commands::CreateAccountArgs {
        name,
        parent_id,
        account_type,
        kind,
        description,
        commodity_codes,
        tag_ids: raw_tag_ids,
        opened_on,
        acquisition: raw_acquisition,
        ..
    } = args;
    let parent = parent_id.as_deref().map(parse_account_id).transpose()?;
    let commodity_ids = resolve_commodity_codes(state, &commodity_codes).await?;
    let tag_ids = resolve_tag_ids(state, &raw_tag_ids).await?;
    let acquisition = raw_acquisition.as_ref().map(bc_core::Acquisition::from);

    let id = state
        .accounts
        .create()
        .name(&name)
        .account_type(account_type.into())
        .kind(kind.into())
        .maybe_description(description.as_deref())
        .maybe_parent_id(parent.as_ref())
        .commodity_ids(&commodity_ids)
        .tag_ids(&tag_ids)
        .maybe_acquisition_date(acquisition.as_ref().and_then(|a| a.date))
        .maybe_acquisition_cost(acquisition.as_ref().and_then(|a| a.cost))
        .maybe_depreciation_policy(
            acquisition
                .as_ref()
                .and_then(|a| a.depreciation_policy.as_ref()),
        )
        .maybe_opened_on(opened_on)
        .call()
        .await?;
    detail_of(state, &id).await
}

/// Applies a patch to an account whose event count is unchanged, and returns
/// its fresh detail.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Conflict`] if the account's event count no
/// longer equals `expected_events`; [`bc_ipc::BcError::Validation`] for a
/// malformed ID, an unknown commodity code or tag, or a patch core refuses;
/// [`bc_ipc::BcError::NotFound`] for an unknown account or parent; or
/// [`bc_ipc::BcError`] if a write or lookup fails.
pub async fn update_account(
    state: &AppState,
    args: bc_ipc::commands::UpdateAccountArgs,
) -> Result<bc_ipc::AccountDetail, bc_ipc::BcError> {
    let bc_ipc::commands::UpdateAccountArgs {
        id: raw_id,
        expected_events,
        patch,
        ..
    } = args;
    let id = parse_account_id(&raw_id)?;
    let parent = patch
        .parent_id
        .map(|p| p.as_deref().map(parse_account_id).transpose())
        .transpose()?;
    let commodity_ids = match patch.commodity_codes {
        Some(codes) => Some(resolve_commodity_codes(state, &codes).await?),
        None => None,
    };
    let tag_ids = match patch.tag_ids {
        Some(ids) => Some(resolve_tag_ids(state, &ids).await?),
        None => None,
    };
    let core_patch = bc_core::AccountPatch::builder()
        .maybe_name(patch.name)
        .maybe_parent(parent)
        .maybe_description(patch.description)
        .maybe_tag_ids(tag_ids)
        .maybe_commodity_ids(commodity_ids)
        .maybe_opened_on(patch.opened_on)
        .maybe_closed_on(patch.closed_on)
        .maybe_acquisition(patch.acquisition.as_ref().map(bc_core::Acquisition::from))
        .build();

    state
        .accounts
        .update(&id, expected_events, core_patch)
        .await?;
    detail_of(state, &id).await
}

/// Closes an account, with its open descendants when `cascade`.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a malformed ID, an open
/// descendant under a non-cascading close, a closing date before the opening
/// date, or an account already closed; [`bc_ipc::BcError::NotFound`] for an
/// unknown account; or [`bc_ipc::BcError`] if the write fails.
pub async fn close_account(
    state: &AppState,
    args: bc_ipc::commands::CloseAccountArgs,
) -> Result<(), bc_ipc::BcError> {
    let bc_ipc::commands::CloseAccountArgs {
        id: raw_id,
        on,
        cascade,
        ..
    } = args;
    let id = parse_account_id(&raw_id)?;
    state.accounts.close(&id, on, cascade_of(cascade)).await?;
    Ok(())
}

/// Reopens a closed account.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a malformed ID, an account
/// that is not closed, or one whose parent is closed;
/// [`bc_ipc::BcError::NotFound`] for an unknown account; or
/// [`bc_ipc::BcError`] if the write fails.
pub async fn reopen_account(
    state: &AppState,
    args: bc_ipc::commands::ReopenAccountArgs,
) -> Result<(), bc_ipc::BcError> {
    let id = parse_account_id(&args.id)?;
    state.accounts.reopen(&id).await?;
    Ok(())
}

/// Archives an account, with its active descendants when `cascade`.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a malformed ID, an active
/// descendant under a non-cascading archive, or an account already archived;
/// [`bc_ipc::BcError::NotFound`] for an unknown account; or
/// [`bc_ipc::BcError`] if the write fails.
pub async fn archive_account(
    state: &AppState,
    args: bc_ipc::commands::ArchiveAccountArgs,
) -> Result<(), bc_ipc::BcError> {
    let bc_ipc::commands::ArchiveAccountArgs {
        id: raw_id,
        cascade,
        ..
    } = args;
    let id = parse_account_id(&raw_id)?;
    state.accounts.archive(&id, cascade_of(cascade)).await?;
    Ok(())
}

/// Unarchives an account.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a malformed ID, an account
/// that is not archived, or one whose parent is archived;
/// [`bc_ipc::BcError::NotFound`] for an unknown account; or
/// [`bc_ipc::BcError`] if the write fails.
pub async fn unarchive_account(
    state: &AppState,
    args: bc_ipc::commands::UnarchiveAccountArgs,
) -> Result<(), bc_ipc::BcError> {
    let id = parse_account_id(&args.id)?;
    state.accounts.unarchive(&id).await?;
    Ok(())
}

/// Maps the wire flag to a core cascade mode.
const fn cascade_of(cascade: bool) -> bc_core::Cascade {
    if cascade {
        bc_core::Cascade::Into
    } else {
        bc_core::Cascade::Reject
    }
}

/// Reports what closing an account on a date would do. Informational only:
/// `close_account` enforces every rule itself.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a malformed ID,
/// [`bc_ipc::BcError::NotFound`] for an unknown account, or
/// [`bc_ipc::BcError`] if a lookup fails.
pub async fn preview_close_account(
    state: &AppState,
    args: bc_ipc::commands::PreviewCloseAccountArgs,
) -> Result<bc_ipc::ClosePreviewDto, bc_ipc::BcError> {
    let bc_ipc::commands::PreviewCloseAccountArgs { id: raw_id, on, .. } = args;
    let id = parse_account_id(&raw_id)?;
    let preview = state.accounts.preview_close(&id, on).await?;
    let accounts = state.accounts.list_all().await?;
    let refs = bc_core::ipc::AccountRefs::new(&accounts);
    let to_ref = |acct: &bc_models::AccountId| refs.get(acct);
    Ok(bc_ipc::ClosePreviewDto::builder()
        .cascade(
            preview
                .cascade
                .iter()
                .map(|(acct, _)| to_ref(acct))
                .collect(),
        )
        .balances(
            preview
                .balances
                .iter()
                .map(|(acct, amounts)| {
                    (
                        to_ref(acct),
                        amounts.iter().map(bc_ipc::Amount::from).collect(),
                    )
                })
                .collect(),
        )
        .postings_after(preview.postings_after)
        .build())
}

/// Reports what archiving an account would do. Informational only:
/// `archive_account` enforces every rule itself.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a malformed ID,
/// [`bc_ipc::BcError::NotFound`] for an unknown account, or
/// [`bc_ipc::BcError`] if a lookup fails.
pub async fn preview_archive_account(
    state: &AppState,
    args: bc_ipc::commands::PreviewArchiveAccountArgs,
) -> Result<bc_ipc::ArchivePreviewDto, bc_ipc::BcError> {
    let id = parse_account_id(&args.id)?;
    let preview = state.accounts.preview_archive(&id).await?;
    let accounts = state.accounts.list_all().await?;
    let refs = bc_core::ipc::AccountRefs::new(&accounts);
    Ok(bc_ipc::ArchivePreviewDto::builder()
        .cascade(
            preview
                .cascade
                .iter()
                .map(|(acct, _)| refs.get(acct))
                .collect(),
        )
        .no_close_date(preview.no_close_date)
        .build())
}

/// Flattens `balances` into IPC amounts, moving `default_code` to the front.
///
/// Without a default commodity (e.g. a root with no own postings, only a
/// roll-up), the remainder is sorted by commodity code rather than left in
/// first-seen order: a root's own `balance` is `None` in that case, so there
/// is no default to anchor on, and first-seen order over a `HashMap`-derived
/// [`bc_models::Balances`] is not guaranteed stable across fetches — a
/// sorted tiebreak keeps the sidebar figure and sparkline currency from
/// flipping between calls.
///
/// # Arguments
///
/// * `balances` - Per-commodity totals in first-seen order.
/// * `default_code` - The account's own default commodity, if any.
fn ordered_amounts(
    balances: &bc_models::Balances,
    default_code: Option<&str>,
) -> Vec<bc_ipc::Amount> {
    let mut out: Vec<bc_ipc::Amount> = balances
        .iter()
        .map(|(code, value)| bc_ipc::Amount::new(value, code))
        .collect();
    if let Some(code) = default_code {
        if let Some(pos) = out.iter().position(|a| a.currency_code == code)
            && pos != 0
        {
            let first = out.remove(pos);
            out.insert(0, first);
        }
    } else {
        out.sort_unstable_by(|a, b| a.currency_code.cmp(&b.currency_code));
    }
    out
}

/// List transactions for the given account within a date window.
///
/// # Arguments
///
/// * `account_id` - The account ID to filter by.
/// * `date_from` - The inclusive start of the date window.
/// * `date_until` - The exclusive end of the date window.
/// * `state` - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the service call fails or the ID is invalid.
pub async fn list_transactions(
    state: &AppState,
    args: bc_ipc::commands::ListTransactionsArgs,
) -> Result<Vec<bc_ipc::Transaction>, bc_ipc::BcError> {
    let bc_ipc::commands::ListTransactionsArgs {
        account_id,
        date_from,
        date_until,
        ..
    } = args;
    let id = account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;

    let ctx = DisplayContext::load(state).await?;
    let to_ipc = ctx.transaction_converter();

    let txs = state
        .transactions
        .list_for_account_in_range(&id, date_from, date_until)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(txs.map(|tx| to_ipc(&tx)).collect())
}

/// Loads one transaction by ID, in the register's display form.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a malformed ID,
/// [`bc_ipc::BcError::NotFound`] if it does not exist, or
/// [`bc_ipc::BcError::Internal`] on a service failure.
pub async fn get_transaction(
    state: &AppState,
    args: bc_ipc::commands::GetTransactionArgs,
) -> Result<bc_ipc::Transaction, bc_ipc::BcError> {
    let bc_ipc::commands::GetTransactionArgs { id, .. } = args;
    let tx_id = id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid transaction id: {e}")))?;
    let tx = state.transactions.find_by_id(&tx_id).await?;
    let ctx = DisplayContext::load(state).await?;
    let to_ipc = ctx.transaction_converter();
    Ok(to_ipc(&tx))
}

/// Create a new transaction.
///
/// # Arguments
///
/// * `tx` - The new transaction data.
/// * `state` - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the service call fails, a field fails
/// validation, or an account ID cannot be parsed.
pub async fn create_transaction(
    state: &AppState,
    args: bc_ipc::commands::CreateTransactionArgs,
) -> Result<bc_ipc::CreatedTransaction, bc_ipc::BcError> {
    let bc_ipc::commands::CreateTransactionArgs { tx, .. } = args;
    let reconciliation = bc_models::Reconciliation::from(tx.reconciliation);

    let mut postings = Vec::with_capacity(tx.postings.len());
    for p in &tx.postings {
        let account_id = p.account_id.parse::<bc_models::AccountId>().map_err(|e| {
            bc_ipc::BcError::Validation(format!("invalid account_id '{}': {e}", p.account_id))
        })?;
        let tag_ids = parse_tag_ids(&p.tags)?;
        let posting = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(account_id)
            .maybe_amount(p.amount.as_ref().map(bc_models::Amount::from))
            .maybe_price(p.price.as_ref().map(bc_models::Quote::from))
            .maybe_cost(p.cost.as_ref().map(bc_models::Cost::from))
            .metadata(metadata_from(&p.metadata)?)
            .tag_ids(tag_ids)
            .maybe_spread_from(p.spread_from)
            .maybe_spread_until(p.spread_until)
            .build();
        postings.push(posting);
    }

    let tx_tag_ids = parse_tag_ids(&tx.tags)?;

    let model_tx = bc_models::Transaction::builder()
        .id(bc_models::TransactionId::new())
        .date(tx.date)
        .description(tx.description)
        .metadata(metadata_from(&tx.metadata)?)
        .postings(postings)
        .tag_ids(tx_tag_ids)
        .reconciliation(reconciliation)
        .created_at(jiff::Timestamp::now())
        .build();

    let wanted: Vec<&bc_ipc::TagInfo> = tx
        .tags
        .iter()
        .chain(tx.postings.iter().flat_map(|p| &p.tags))
        .collect();
    ensure_tags_exist(&state.tags, &wanted).await?;

    let warned = state
        .transactions
        .create(model_tx)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(bc_ipc::CreatedTransaction::new(
        warned.value.to_string(),
        warned.warnings.iter().map(Into::into).collect(),
    ))
}

/// Converts an IPC edit into the model the service writes.
///
/// # Arguments
///
/// * `tx` - The desired transaction state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for an unparsable ID (tag IDs
/// included) or another domain rule violation. No tag lookup happens here.
fn model_from_edit(
    tx: &bc_ipc::EditTransaction,
) -> Result<bc_models::Transaction, bc_ipc::BcError> {
    let tx_id = tx
        .id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid transaction id: {e}")))?;
    let reconciliation = bc_models::Reconciliation::from(tx.reconciliation);

    let mut postings = Vec::with_capacity(tx.postings.len());
    for p in &tx.postings {
        let account_id = p
            .account_id
            .parse::<bc_models::AccountId>()
            .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account id: {e}")))?;
        let posting_id = match &p.id {
            Some(s) => s
                .parse::<bc_models::PostingId>()
                .map_err(|e| bc_ipc::BcError::Validation(format!("invalid posting id: {e}")))?,
            None => bc_models::PostingId::new(),
        };
        let tag_ids = parse_tag_ids(&p.tags)?;
        let posting = bc_models::Posting::builder()
            .id(posting_id)
            .account_id(account_id)
            .maybe_amount(p.amount.as_ref().map(bc_models::Amount::from))
            .maybe_price(p.price.as_ref().map(bc_models::Quote::from))
            .maybe_cost(p.cost.as_ref().map(bc_models::Cost::from))
            .metadata(metadata_from(&p.metadata)?)
            .tag_ids(tag_ids)
            .maybe_spread_from(p.spread_from)
            .maybe_spread_until(p.spread_until)
            .build();
        postings.push(posting);
    }

    let tag_ids = parse_tag_ids(&tx.tags)?;

    Ok(bc_models::Transaction::builder()
        .id(tx_id)
        .date(tx.date)
        .description(tx.description.clone())
        .metadata(metadata_from(&tx.metadata)?)
        .postings(postings)
        .reconciliation(reconciliation)
        .tag_ids(tag_ids)
        .created_at(jiff::Timestamp::now())
        .build())
}

/// Applies a desired transaction state (decomposed-event edit) only if the
/// transaction is still in the state the editor loaded it in.
///
/// # Arguments
///
/// * `tx` - The desired transaction state.
/// * `base` - The transaction as the editor loaded it.
/// * `state` - The shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for unparsable IDs, an unknown tag
/// in `tx` or domain rule violations, [`bc_ipc::BcError::NotFound`] if the
/// transaction does not exist, [`bc_ipc::BcError::Conflict`] if the
/// transaction changed since `base` was loaded, or
/// [`bc_ipc::BcError::Internal`] for unexpected failures.
pub async fn edit_transaction(
    state: &AppState,
    args: bc_ipc::commands::EditTransactionArgs,
) -> Result<Vec<bc_ipc::WriteWarning>, bc_ipc::BcError> {
    let bc_ipc::commands::EditTransactionArgs { tx, base, .. } = args;
    let base_model = model_from_edit(&base)?;
    let updated = model_from_edit(&tx)?;
    // A base tag that no longer exists is not on the stored transaction (FK),
    // so the diff reports it as a Conflict; only new draft tags are checked.
    let known: HashSet<&str> = base
        .tags
        .iter()
        .chain(base.postings.iter().flat_map(|p| &p.tags))
        .map(|t| t.id.as_str())
        .collect();
    let added: Vec<&bc_ipc::TagInfo> = tx
        .tags
        .iter()
        .chain(tx.postings.iter().flat_map(|p| &p.tags))
        .filter(|t| !known.contains(t.id.as_str()))
        .collect();
    ensure_tags_exist(&state.tags, &added).await?;

    let warned = state
        .transactions
        .edit_if_unchanged(&base_model, updated)
        .await?;
    Ok(warned.warnings.iter().map(Into::into).collect())
}

/// Sets a transaction's reconciliation state.
///
/// # Arguments
///
/// * `id`             - The transaction ID to update.
/// * `reconciliation` - The desired reconciliation state.
/// * `state`          - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if `id` is malformed or the
/// transaction does not balance, [`bc_ipc::BcError::NotFound`] if no such
/// transaction exists, or [`bc_ipc::BcError::Internal`] if the update fails.
pub async fn set_reconciliation(
    state: &AppState,
    args: bc_ipc::commands::SetReconciliationArgs,
) -> Result<(), bc_ipc::BcError> {
    let bc_ipc::commands::SetReconciliationArgs {
        id, reconciliation, ..
    } = args;
    let tx_id = id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid transaction id: {e}")))?;
    Ok(state
        .transactions
        .reconcile(&tx_id, bc_models::Reconciliation::from(reconciliation))
        .await?)
}

/// Reverses a transaction, returning the new reversal transaction's id.
///
/// # Arguments
///
/// * `id`    - The transaction ID to reverse.
/// * `state` - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if `id` is malformed or no such transaction exists.
pub async fn reverse_transaction(
    state: &AppState,
    args: bc_ipc::commands::ReverseTransactionArgs,
) -> Result<String, bc_ipc::BcError> {
    let bc_ipc::commands::ReverseTransactionArgs { id, .. } = args;
    let tx_id = id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid id: {e}")))?;

    let reversal_id = state
        .transactions
        .reverse(&tx_id)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(reversal_id.to_string())
}

/// Returns inflow, outflow, internal and balance stats for `account_id` over
/// an explicit date window.
///
/// # Arguments
///
/// * `account_id` - The account to query.
/// * `commodity`  - Optional commodity code override. Defaults to the account's first commodity.
/// * `include_descendants` - Fold the account's active subtree into the stats.
/// * `date_from`  - The inclusive start of the date window.
/// * `date_until` - The exclusive end of the date window.
/// * `filter`     - Active global filter, or `None` for the unfiltered fast path. When
///   `Some`, the stats are recomputed against it and the real (unfiltered)
///   opening/closing are attached for reference.
/// * `state`      - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the account ID is malformed,
/// [`bc_ipc::BcError::Query`] if the filter's query does not parse or resolve,
/// and [`bc_ipc::BcError::Internal`] if the balance or scope lookups fail.
/// The filtered stats propagate a core error through
/// `From<bc_core::BcError> for bc_ipc::BcError`: `BadData` becomes
/// [`bc_ipc::BcError::Validation`], and a database failure becomes
/// [`bc_ipc::BcError::Internal`].
pub async fn get_account_stats(
    state: &AppState,
    args: bc_ipc::commands::GetAccountStatsArgs,
) -> Result<bc_ipc::AccountStats, bc_ipc::BcError> {
    let bc_ipc::commands::GetAccountStatsArgs {
        account_id,
        commodity,
        include_descendants,
        date_from,
        date_until,
        filter,
        ..
    } = args;
    let id = account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;

    let commodity_code = match commodity {
        Some(c) => c,
        None => state
            .balance_engine
            .default_commodity_for(&id)
            .await
            .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?
            .unwrap_or_default(),
    };

    let ids = scope_ids(state, &id, include_descendants).await?;
    let first_activity = state
        .transactions
        .earliest_activity_date_for_set(&ids)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    // Real (unfiltered) window stats — cheap SQL, always computed so the
    // filtered branch can attach them for reference.
    let real = state
        .balance_engine
        .account_period_stats_for_set(&ids, &commodity_code, date_from, date_until)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let Some(active_filter) = filter else {
        return Ok(bc_ipc::AccountStats::new(
            bc_ipc::Amount::from(&real.inflow),
            bc_ipc::Amount::from(&real.outflow),
            bc_ipc::Amount::from(&real.internal),
            bc_ipc::Amount::from(&real.net),
            bc_ipc::Amount::from(&real.opening),
            bc_ipc::Amount::from(&real.closing),
            real.tx_count,
        )
        .with_first_activity(first_activity));
    };

    let query = state.transactions.query_from_filter(active_filter).await?;
    let filtered = state
        .transactions
        .filtered_period_stats(&ids, &commodity_code, &query, date_from, date_until)
        .await?;

    Ok(bc_ipc::AccountStats::new(
        bc_ipc::Amount::from(&filtered.inflow),
        bc_ipc::Amount::from(&filtered.outflow),
        bc_ipc::Amount::from(&filtered.internal),
        bc_ipc::Amount::from(&filtered.net),
        bc_ipc::Amount::from(&filtered.opening),
        bc_ipc::Amount::from(&filtered.closing),
        filtered.tx_count,
    )
    .with_real_balances(
        bc_ipc::Amount::from(&real.opening),
        bc_ipc::Amount::from(&real.closing),
    )
    .with_first_activity(first_activity))
}

/// Runs a structured transaction search.
///
/// # Arguments
///
/// * `filter` - The structured filter to apply.
/// * `state` - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Query`] if the filter's query does not parse or
/// resolve, or [`bc_ipc::BcError::Internal`] if a service call fails.
pub async fn search_transactions(
    state: &AppState,
    args: bc_ipc::commands::SearchTransactionsArgs,
) -> Result<Vec<bc_ipc::FilteredTransaction>, bc_ipc::BcError> {
    let bc_ipc::commands::SearchTransactionsArgs { filter, .. } = args;
    let query = state.transactions.query_from_filter(filter).await?;

    let ctx = DisplayContext::load(state).await?;
    let to_ipc = ctx.transaction_converter();

    let matched = state.transactions.search(&query).await?;

    Ok(matched
        .into_iter()
        .map(|m| {
            bc_ipc::FilteredTransaction::new(
                to_ipc(&m.transaction),
                m.matched_postings.iter().map(ToString::to_string).collect(),
            )
        })
        .collect())
}

/// Fetches one page of the register for an account, with running balances.
///
/// # Arguments
///
/// * `request` - Filter, scope, cursor and limit.
/// * `state` - Shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for a malformed account or cursor
/// id, [`bc_ipc::BcError::Query`] if the filter's query does not parse or
/// resolve, [`bc_ipc::BcError::Internal`] if the account or tag lookups fail,
/// and the IPC mapping of the core error (`BadData` becomes `Validation`) if
/// the register query itself fails.
pub async fn register_page(
    state: &AppState,
    args: bc_ipc::commands::RegisterPageArgs,
) -> Result<bc_ipc::RegisterPage, bc_ipc::BcError> {
    let bc_ipc::commands::RegisterPageArgs { request, .. } = args;
    let id = request
        .account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;
    let scope = scope_ids(state, &id, request.include_descendants).await?;
    let cursor = request
        .cursor
        .as_ref()
        .map(|c| {
            c.id.parse::<bc_models::TransactionId>()
                .map(|tx_id| bc_core::search::RegisterCursor {
                    date: c.date,
                    id: tx_id,
                })
                .map_err(|e| bc_ipc::BcError::Validation(format!("invalid cursor id: {e}")))
        })
        .transpose()?;
    let query = state.transactions.query_from_filter(request.filter).await?;

    let ctx = DisplayContext::load(state).await?;
    let to_ipc = ctx.transaction_converter();

    let page = state
        .transactions
        .register_page(&query, &scope, cursor.as_ref(), request.limit)
        .await?;

    Ok(bc_ipc::RegisterPage::new(
        page.rows
            .into_iter()
            .map(|r| {
                bc_ipc::RegisterRow::new(
                    to_ipc(&r.transaction),
                    r.matched_postings.iter().map(ToString::to_string).collect(),
                    r.balance_after.as_ref().map(bc_ipc::Amount::from),
                    r.filtered_sum_after.as_ref().map(bc_ipc::Amount::from),
                )
            })
            .collect(),
        page.total,
        page.next_cursor
            .map(|c| bc_ipc::RegisterCursor::new(c.date, c.id.to_string())),
    ))
}

// MARK: Metadata helpers

/// Converts a list of metadata entry DTOs into a domain metadata list.
///
/// The order is the display order and is preserved. Each entry's `mismatched`
/// flag is discarded: the write path derives it against the key's registered
/// type, so an incoming entry does not get to assert it.
///
/// # Arguments
///
/// * `entries` - The entries as they arrived over IPC.
///
/// # Returns
///
/// The domain list, in input order.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if a key fails validation or an
/// account-valued entry carries an unparsable account id.
fn metadata_from(entries: &[bc_ipc::MetaEntryDto]) -> Result<bc_models::Metadata, bc_ipc::BcError> {
    entries.iter().map(bc_models::MetaEntry::try_from).collect()
}

// MARK: Tag helpers

/// Parses the IDs of wire tags; each tag's `path` is display-only and ignored.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if an ID does not parse.
fn parse_tag_ids(tags: &[bc_ipc::TagInfo]) -> Result<Vec<bc_models::TagId>, bc_ipc::BcError> {
    tags.iter()
        .map(|t| {
            t.id.parse::<bc_models::TagId>()
                .map_err(|e| bc_ipc::BcError::Validation(format!("invalid tag id '{}': {e}", t.id)))
        })
        .collect()
}

/// Checks that every tag names an existing tag by ID.
///
/// Each tag's `path` is a display snapshot used only in the error message.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] naming the first unknown tag, or
/// [`bc_ipc::BcError::Internal`] if the hierarchy fails to load.
async fn ensure_tags_exist(
    tags: &bc_core::TagService,
    wanted: &[&bc_ipc::TagInfo],
) -> Result<(), bc_ipc::BcError> {
    if wanted.is_empty() {
        return Ok(());
    }
    let forest = tags
        .forest()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    for t in wanted {
        let id = t
            .id
            .parse::<bc_models::TagId>()
            .map_err(|e| bc_ipc::BcError::Validation(format!("invalid tag id '{}': {e}", t.id)))?;
        if forest.path_of(&id).is_none() {
            return Err(bc_ipc::BcError::Validation(format!(
                "unknown tag '{}' (id {})",
                t.path, t.id
            )));
        }
    }
    Ok(())
}

// MARK: Sparkline helpers

/// Formats a bucket start date as a sparkline X-axis label.
///
/// Week, month and quarter labels carry a two-digit year (`Q1 '26`) on the
/// first bucket and wherever the year changes from `prev`, the previous
/// bucket's start; weeks use the ISO week-numbering year.
fn spark_label(
    start: jiff::civil::Date,
    prev: Option<jiff::civil::Date>,
    period: &bc_models::Period,
) -> String {
    let year_suffix = |year_of: fn(jiff::civil::Date) -> i16| {
        if prev.is_some_and(|p| year_of(p) == year_of(start)) {
            String::new()
        } else {
            #[expect(
                clippy::integer_division_remainder_used,
                clippy::modulo_arithmetic,
                reason = "year % 100 gives the 2-digit year; years are positive for realistic dates"
            )]
            let two_digit = year_of(start) % 100;
            format!(" '{two_digit:02}")
        }
    };
    match period {
        bc_models::Period::Monthly => {
            let months = [
                "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
            ];
            #[expect(
                clippy::as_conversions,
                clippy::cast_sign_loss,
                reason = "month() returns i8 in 1–12; casting to usize is safe"
            )]
            let idx = start.month() as usize;
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "idx is 1–12; subtracting 1 gives 0–11, always in bounds"
            )]
            #[expect(
                clippy::indexing_slicing,
                reason = "idx - 1 is 0–11; array has exactly 12 elements"
            )]
            let month = months[idx - 1];
            format!("{month}{}", year_suffix(jiff::civil::Date::year))
        }
        bc_models::Period::Weekly => {
            let week = start.iso_week_date().week();
            format!("w{week:02}{}", year_suffix(|d| d.iso_week_date().year()))
        }
        bc_models::Period::Quarterly | bc_models::Period::FinancialQuarter { .. } => {
            // Map month to Q1–Q4 (calendar quarters)
            #[expect(
                clippy::as_conversions,
                clippy::cast_sign_loss,
                reason = "month() returns i8 in 1–12; casting to u8 is safe"
            )]
            let month_u8 = start.month() as u8;
            #[expect(
                clippy::arithmetic_side_effects,
                clippy::integer_division,
                clippy::integer_division_remainder_used,
                reason = "month_u8 is 1–12; arithmetic maps to quarter 1–4 without overflow"
            )]
            let q = (month_u8 - 1) / 3 + 1;
            format!("Q{q}{}", year_suffix(jiff::civil::Date::year))
        }
        bc_models::Period::CalendarYear => format!("{}", start.year()),
        bc_models::Period::FinancialYear { .. } => {
            #[expect(
                clippy::integer_division_remainder_used,
                clippy::modulo_arithmetic,
                reason = "year % 100 gives 2-digit year; i16 year is always positive for realistic dates"
            )]
            let two_digit = start.year() % 100;
            format!("FY{two_digit:02}")
        }
        bc_models::Period::Daily
        | bc_models::Period::Fortnightly { .. }
        | bc_models::Period::Custom { .. } => start.to_string(),
        _ => {
            tracing::warn!(period = ?period, "unknown Period variant in spark_label; using start date as label");
            start.to_string()
        }
    }
}

// MARK: Audit helpers

/// Loads the audit trail for a transaction.
///
/// # Arguments
///
/// * `id` - The transaction's ID.
/// * `state` - The shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for an unparsable ID, or
/// [`bc_ipc::BcError::Internal`] if the lookup fails.
pub async fn get_transaction_audit(
    state: &AppState,
    args: bc_ipc::commands::GetTransactionAuditArgs,
) -> Result<Vec<bc_ipc::AuditEntry>, bc_ipc::BcError> {
    let bc_ipc::commands::GetTransactionAuditArgs { id, .. } = args;
    let tx_id = id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid transaction id: {e}")))?;
    let trail = state
        .transactions
        .audit_trail(&tx_id)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "bc_core::Event is #[non_exhaustive]; catch-all arm required for exhaustiveness against future variants"
    )]
    let source_account_ids: std::collections::HashSet<_> = trail
        .iter()
        .filter_map(|(_, event)| match event {
            bc_core::Event::TransactionSourceAttached { account_id, .. } => {
                Some(account_id.clone())
            }
            _ => None,
        })
        .collect();

    let mut account_names = std::collections::HashMap::new();
    for account_id in source_account_ids.into_iter().collect::<Vec<_>>() {
        let account = state
            .accounts
            .find_by_id(&account_id)
            .await
            .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
        account_names.insert(account_id, account.name().to_owned());
    }

    Ok(trail
        .iter()
        .map(|(ts, event)| bc_ipc::AuditEntry::from_event(*ts, event, &account_names))
        .collect())
}

/// Returns period-bucketed cash-flow data for a sparkline chart.
///
/// Defaults to 6 monthly buckets ending with the current month.
///
/// # Arguments
///
/// * `account_id` - The account to query.
/// * `commodity`  - Optional commodity code. Defaults to the account's first commodity.
/// * `include_descendants` - Fold the account's active subtree into the buckets.
/// * `count`      - Optional bucket count (default 6).
/// * `period`     - Optional bucket period (default Monthly).
/// * `as_of`      - Optional reference date; the most recent bucket contains this
///   date. Defaults to today.
/// * `filter`     - Active global filter, or `None` for the unfiltered fast path.
/// * `state`      - Shared application state.
///
/// # Panics
///
/// Never panics in practice — the internal `NonZeroUsize::new(6)` is a
/// compile-time constant and can never be zero.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the account ID is malformed,
/// [`bc_ipc::BcError::Query`] if the filter's query does not parse or resolve,
/// and [`bc_ipc::BcError::Internal`] if the balance or scope lookups fail.
/// The filtered buckets propagate a core error through
/// `From<bc_core::BcError> for bc_ipc::BcError`: `BadData` becomes
/// [`bc_ipc::BcError::Validation`], and a database failure becomes
/// [`bc_ipc::BcError::Internal`].
pub async fn get_account_sparkline(
    state: &AppState,
    args: bc_ipc::commands::GetAccountSparklineArgs,
) -> Result<Vec<bc_ipc::SparkPoint>, bc_ipc::BcError> {
    let bc_ipc::commands::GetAccountSparklineArgs {
        account_id,
        commodity,
        include_descendants,
        count,
        period,
        as_of,
        filter,
        ..
    } = args;
    let id = account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;

    let commodity_code = match commodity {
        Some(c) => c,
        None => state
            .balance_engine
            .default_commodity_for(&id)
            .await
            .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?
            .unwrap_or_default(),
    };

    let ids = scope_ids(state, &id, include_descendants).await?;

    let bucket_count = count
        .and_then(|c| NonZeroUsize::new(usize::try_from(c).unwrap_or(0)))
        .unwrap_or_else(|| {
            #[expect(
                clippy::expect_used,
                reason = "6 is a non-zero constant; this can never panic"
            )]
            NonZeroUsize::new(6).expect("6 > 0")
        });

    let model_period = period.unwrap_or(bc_models::Period::Monthly);

    let anchor = as_of.unwrap_or_else(|| jiff::Zoned::now().date());

    let buckets = match filter {
        None => state
            .balance_engine
            .posting_buckets_for_set(&ids, &commodity_code, &model_period, bucket_count, anchor)
            .await
            .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?,
        Some(active_filter) => {
            let query = state.transactions.query_from_filter(active_filter).await?;
            state
                .transactions
                .filtered_posting_buckets(
                    &ids,
                    &commodity_code,
                    &query,
                    &model_period,
                    bucket_count,
                    anchor,
                )
                .await?
        }
    };

    let mut prev = None;
    let points = buckets
        .into_iter()
        .map(|b| {
            let label = spark_label(b.start, prev.replace(b.start), &model_period);
            bc_ipc::SparkPoint::new(
                label,
                bc_ipc::Amount::from(&b.inflow),
                bc_ipc::Amount::from(&b.outflow),
            )
        })
        .collect::<Vec<_>>();

    Ok(points)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use core::ops::Neg as _;

    use bc_ipc::commands;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;
    use serde_json::json;
    use tempfile::TempDir;

    use crate::AppState;

    #[test]
    fn spark_label_names_a_daily_bucket_by_its_date() {
        let start = jiff::civil::date(2026, 2, 28);
        assert_eq!(
            super::spark_label(start, None, &bc_models::Period::Daily),
            "2026-02-28"
        );
    }

    #[rstest]
    #[case::iso_week_differs_from_day_of_year(
        jiff::civil::date(2026, 9, 28),
        Some(jiff::civil::date(2026, 9, 21)),
        "w40"
    )]
    #[case::iso_week_one_starts_in_previous_december(
        jiff::civil::date(2024, 12, 30),
        Some(jiff::civil::date(2024, 12, 23)),
        "w01 '25"
    )]
    #[case::first_bucket_carries_year(jiff::civil::date(2026, 2, 23), None, "w09 '26")]
    fn spark_label_numbers_weeks_by_iso_week(
        #[case] start: jiff::civil::Date,
        #[case] prev: Option<jiff::civil::Date>,
        #[case] expected: &str,
    ) {
        assert_eq!(
            super::spark_label(start, prev, &bc_models::Period::Weekly),
            expected
        );
    }

    #[rstest]
    #[case::first_bucket(jiff::civil::date(2025, 7, 1), None, "Q3 '25")]
    #[case::same_year(
        jiff::civil::date(2025, 10, 1),
        Some(jiff::civil::date(2025, 7, 1)),
        "Q4"
    )]
    #[case::year_boundary(
        jiff::civil::date(2026, 1, 1),
        Some(jiff::civil::date(2025, 10, 1)),
        "Q1 '26"
    )]
    fn spark_label_dates_a_quarter_on_first_tick_and_year_change(
        #[case] start: jiff::civil::Date,
        #[case] prev: Option<jiff::civil::Date>,
        #[case] expected: &str,
    ) {
        assert_eq!(
            super::spark_label(start, prev, &bc_models::Period::Quarterly),
            expected
        );
    }

    #[rstest]
    #[case::first_bucket(jiff::civil::date(2025, 12, 1), None, "dec '25")]
    #[case::same_year(
        jiff::civil::date(2025, 12, 1),
        Some(jiff::civil::date(2025, 11, 1)),
        "dec"
    )]
    #[case::year_boundary(
        jiff::civil::date(2026, 1, 1),
        Some(jiff::civil::date(2025, 12, 1)),
        "jan '26"
    )]
    fn spark_label_dates_a_month_on_first_tick_and_year_change(
        #[case] start: jiff::civil::Date,
        #[case] prev: Option<jiff::civil::Date>,
        #[case] expected: &str,
    ) {
        assert_eq!(
            super::spark_label(start, prev, &bc_models::Period::Monthly),
            expected
        );
    }

    /// Builds a [`bc_models::Balances`] from `(commodity, amount)` pairs, added
    /// in order so the first pair is first-seen.
    fn balances_of(pairs: &[(&str, &str)]) -> bc_models::Balances {
        let mut balances = bc_models::Balances::new();
        for (code, value) in pairs {
            let amount = bc_models::Amount::new(
                value
                    .parse()
                    .expect("test amount literal is a valid decimal"),
                *code,
            );
            balances
                .try_add(&amount)
                .expect("test amounts do not overflow");
        }
        balances
    }

    #[test]
    fn ordered_amounts_moves_the_default_code_to_the_front() {
        let balances = balances_of(&[("AUD", "100.00"), ("USD", "50.00"), ("EUR", "25.00")]);
        let out = super::ordered_amounts(&balances, Some("USD"));
        assert_eq!(
            out.iter()
                .map(|a| a.currency_code.as_str())
                .collect::<Vec<_>>(),
            vec!["USD", "AUD", "EUR"],
            "the default commodity moves to the front; the rest keep their relative order"
        );
    }

    #[test]
    fn ordered_amounts_is_a_no_op_when_the_default_is_already_first() {
        let balances = balances_of(&[("AUD", "100.00"), ("USD", "50.00")]);
        let out = super::ordered_amounts(&balances, Some("AUD"));
        assert_eq!(
            out.iter()
                .map(|a| a.currency_code.as_str())
                .collect::<Vec<_>>(),
            vec!["AUD", "USD"],
            "already-first default code leaves first-seen order untouched"
        );
    }

    #[test]
    fn ordered_amounts_sorts_by_code_without_a_default() {
        let balances = balances_of(&[("USD", "50.00"), ("AUD", "100.00"), ("EUR", "25.00")]);
        let out = super::ordered_amounts(&balances, None);
        assert_eq!(
            out.iter()
                .map(|a| a.currency_code.as_str())
                .collect::<Vec<_>>(),
            vec!["AUD", "EUR", "USD"],
            "no default code means the remainder is sorted, not left in \
             first-seen order, so a root with no own balance does not flip \
             between fetches"
        );
    }

    #[test]
    fn ordered_amounts_of_empty_balances_is_empty() {
        let balances = bc_models::Balances::new();
        assert_eq!(super::ordered_amounts(&balances, Some("AUD")), vec![]);
        assert_eq!(super::ordered_amounts(&balances, None), vec![]);
    }

    #[test]
    fn metadata_from_preserves_order_and_drops_the_flag() {
        let entries = vec![
            bc_ipc::MetaEntryDto::flagged("payee", "Generic Grocer"),
            bc_ipc::MetaEntryDto::new("note", bc_ipc::MetaValueDto::Text("weekly shop".to_owned())),
        ];
        assert!(
            entries.first().is_some_and(|e| e.mismatched),
            "the first entry arrives claiming to be flagged"
        );

        let meta = super::metadata_from(&entries).expect("valid entries convert");
        assert_eq!(
            meta.iter().map(|e| e.key().as_str()).collect::<Vec<_>>(),
            vec!["payee", "note"],
            "input order is the display order"
        );
        assert!(
            meta.iter().all(|e| !e.mismatched()),
            "the write path derives the flag; an incoming entry does not assert it"
        );
    }

    #[test]
    fn metadata_from_rejects_an_invalid_key() {
        let entries = vec![bc_ipc::MetaEntryDto::new(
            "1nvoice",
            bc_ipc::MetaValueDto::Boolean(true),
        )];
        let err = super::metadata_from(&entries).expect_err("a key must start with a letter");
        assert!(
            matches!(err, bc_ipc::BcError::Validation(ref m) if m.contains("1nvoice")),
            "the failure names the offending key, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn ensure_tags_exist_errors_on_unknown_tag() {
        let pool = bc_core::open_db("sqlite::memory:").await.expect("db");
        let tags = bc_core::TagService::new(pool);
        let ghost = bc_ipc::TagInfo::new(bc_models::TagId::new().to_string(), "ghost");
        let err = super::ensure_tags_exist(&tags, &[&ghost])
            .await
            .expect_err("unknown tag must error");
        assert!(
            matches!(err, bc_ipc::BcError::Validation(ref m) if m.contains("'ghost'") && m.contains(&ghost.id)),
            "{err:?}"
        );
    }

    #[test]
    fn parse_tag_ids_ignores_the_path() {
        let id = bc_models::TagId::new();
        let parsed =
            super::parse_tag_ids(&[bc_ipc::TagInfo::new(id.to_string(), "::not a path::")])
                .expect("the path is never parsed");
        assert_eq!(parsed, vec![id]);
    }

    // MARK: Account management fixtures

    /// A fresh state on a database inside `dir`, with config lookup isolated.
    async fn open_state(dir: &TempDir) -> AppState {
        // SAFETY: nextest runs each test in its own process, before any thread
        // reads the environment.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config")) }
        let mut settings = bc_config::Settings::default();
        settings.set_db_path(dir.path().join("ledger.db"));
        settings.set_backup_dir(dir.path().join("backups"));
        AppState::open(&settings).await.expect("open")
    }

    /// Creates a deposit account directly through the core service.
    async fn account(
        state: &AppState,
        name: &str,
        ty: bc_models::AccountType,
        parent: Option<&bc_models::AccountId>,
    ) -> bc_models::AccountId {
        state
            .accounts
            .create()
            .name(name)
            .account_type(ty)
            .kind(bc_models::AccountKind::DepositAccount)
            .maybe_parent_id(parent)
            .call()
            .await
            .expect(name)
    }

    /// Runs `cmd` through dispatch and decodes its reply as `T`.
    async fn run<T>(
        state: &AppState,
        cmd: &str,
        args: serde_json::Value,
    ) -> Result<T, bc_ipc::BcError>
    where
        T: serde::de::DeserializeOwned,
    {
        crate::dispatch(state, cmd, args)
            .await
            .map(|v| serde_json::from_value(v).expect("reply decodes"))
    }

    // MARK: Account management commands

    #[tokio::test]
    async fn list_account_details_includes_archived() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let expenses = account(&state, "Expenses", bc_models::AccountType::Expense, None).await;
        let assets = account(&state, "Assets", bc_models::AccountType::Asset, None).await;
        let savings = account(
            &state,
            "Savings",
            bc_models::AccountType::Asset,
            Some(&assets),
        )
        .await;
        let old = account(
            &state,
            "Old Bank",
            bc_models::AccountType::Asset,
            Some(&assets),
        )
        .await;
        let food = account(
            &state,
            "Food",
            bc_models::AccountType::Expense,
            Some(&expenses),
        )
        .await;
        state
            .accounts
            .archive(&old, bc_core::Cascade::Reject)
            .await
            .expect("archive");

        let details: Vec<bc_ipc::AccountDetail> =
            run(&state, commands::LIST_ACCOUNT_DETAILS, json!({}))
                .await
                .expect("list");

        assert_eq!(
            details
                .iter()
                .map(|d| (d.path.as_str(), d.archived))
                .collect::<Vec<_>>(),
            vec![
                ("Assets", false),
                ("Assets :: Old Bank", true),
                ("Assets :: Savings", false),
                ("Expenses", false),
                ("Expenses :: Food", false),
            ],
            "every account, archived included, roots by type then children by name"
        );
        assert_eq!(
            details.iter().map(|d| d.id.clone()).collect::<Vec<_>>(),
            [assets, old, savings, expenses, food]
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn get_account_detail_unknown_id_is_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let ghost = bc_models::AccountId::new().to_string();

        let result: Result<bc_ipc::AccountDetail, _> =
            run(&state, commands::GET_ACCOUNT_DETAIL, json!({ "id": ghost })).await;

        assert_eq!(result, Err(bc_ipc::BcError::NotFound(ghost)));
    }

    #[tokio::test]
    async fn create_account_round_trips_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let assets = account(&state, "Assets", bc_models::AccountType::Asset, None).await;
        let tag = state
            .tags
            .create_path(&"asset:vehicle".parse().expect("tag path"))
            .await
            .expect("tag");

        let created: bc_ipc::AccountDetail = run(
            &state,
            commands::CREATE_ACCOUNT,
            json!({
                "name": "Car",
                "parent_id": assets.to_string(),
                "account_type": "Asset",
                "kind": "ManualAsset",
                "description": "Hatchback",
                "commodity_codes": ["USD", "AUD"],
                "tag_ids": [tag.to_string()],
                "opened_on": "2026-01-15",
                "acquisition": {
                    "date": "2026-01-15",
                    "cost": "12345.00",
                    "depreciation_policy": { "type": "straight_line", "annual_rate": "0.25" },
                },
            }),
        )
        .await
        .expect("create");

        assert_eq!(created.name, "Car");
        assert_eq!(created.path, "Assets :: Car");
        assert_eq!(created.parent_id, Some(assets.to_string()));
        assert_eq!(created.account_type, bc_ipc::AccountType::Asset);
        assert_eq!(created.kind, bc_ipc::AccountKindDto::ManualAsset);
        assert_eq!(created.description.as_deref(), Some("Hatchback"));
        assert_eq!(created.commodity_codes, vec!["USD", "AUD"]);
        assert_eq!(
            created.tags,
            vec![bc_ipc::TagInfo::new(tag.to_string(), "asset:vehicle")]
        );
        assert_eq!(created.opened_on, Some(jiff::civil::date(2026, 1, 15)));
        assert_eq!(created.closed_on, None);
        assert!(!created.archived);
        assert_eq!(
            created.acquisition,
            Some(
                bc_ipc::AcquisitionDto::builder()
                    .date(jiff::civil::date(2026, 1, 15))
                    .cost(dec!(12345.00))
                    .depreciation_policy(bc_ipc::DepreciationPolicyDto::StraightLine {
                        annual_rate: dec!(0.25),
                    })
                    .build()
            )
        );
        assert!(created.event_count > 0, "creation records an event");

        let fetched: bc_ipc::AccountDetail = run(
            &state,
            commands::GET_ACCOUNT_DETAIL,
            json!({ "id": created.id }),
        )
        .await
        .expect("get");
        assert_eq!(fetched, created, "the returned detail is the stored one");
    }

    #[tokio::test]
    async fn update_account_returns_the_fresh_detail() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let assets = account(&state, "Assets", bc_models::AccountType::Asset, None).await;
        let bank = account(&state, "Bank", bc_models::AccountType::Asset, Some(&assets)).await;
        let before: bc_ipc::AccountDetail = run(
            &state,
            commands::GET_ACCOUNT_DETAIL,
            json!({ "id": bank.to_string() }),
        )
        .await
        .expect("get");

        let after: bc_ipc::AccountDetail = run(
            &state,
            commands::UPDATE_ACCOUNT,
            json!({
                "id": bank.to_string(),
                "expected_events": before.event_count,
                "patch": {
                    "name": "Everyday",
                    "parent_id": null,
                    "commodity_codes": ["AUD"],
                    "description": "Main account",
                },
            }),
        )
        .await
        .expect("update");

        assert_eq!(after.path, "Everyday");
        assert_eq!(after.parent_id, None);
        assert_eq!(after.commodity_codes, vec!["AUD"]);
        assert_eq!(after.description.as_deref(), Some("Main account"));
        assert!(
            after.event_count > before.event_count,
            "the reply carries the new base"
        );
    }

    #[tokio::test]
    async fn update_account_stale_count_is_conflict() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let bank = account(&state, "Bank", bc_models::AccountType::Asset, None).await;
        let count = state.accounts.event_count(&bank).await.expect("count");

        let result: Result<bc_ipc::AccountDetail, _> = run(
            &state,
            commands::UPDATE_ACCOUNT,
            json!({
                "id": bank.to_string(),
                "expected_events": count + 1,
                "patch": { "name": "Renamed" },
            }),
        )
        .await;

        assert!(
            matches!(result, Err(bc_ipc::BcError::Conflict(_))),
            "{result:?}"
        );
    }

    #[rstest]
    #[case::update(true)]
    #[case::create(false)]
    #[tokio::test]
    async fn unknown_commodity_code_is_validation(#[case] update: bool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let bank = account(&state, "Bank", bc_models::AccountType::Asset, None).await;
        let count = state.accounts.event_count(&bank).await.expect("count");

        let (cmd, args) = if update {
            (
                commands::UPDATE_ACCOUNT,
                json!({
                    "id": bank.to_string(),
                    "expected_events": count,
                    "patch": { "commodity_codes": ["AUD", "XYZ"] },
                }),
            )
        } else {
            (
                commands::CREATE_ACCOUNT,
                json!({
                    "name": "Other",
                    "parent_id": null,
                    "account_type": "Asset",
                    "kind": "DepositAccount",
                    "description": null,
                    "commodity_codes": ["XYZ"],
                    "tag_ids": [],
                    "opened_on": null,
                    "acquisition": null,
                }),
            )
        };
        let result: Result<bc_ipc::AccountDetail, _> = run(&state, cmd, args).await;

        assert!(
            matches!(result, Err(bc_ipc::BcError::Validation(ref m)) if m.contains("XYZ")),
            "{result:?}"
        );
    }

    #[rstest]
    #[case::update(true)]
    #[case::create(false)]
    #[tokio::test]
    async fn unknown_tag_id_is_validation(#[case] update: bool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let bank = account(&state, "Bank", bc_models::AccountType::Asset, None).await;
        let count = state.accounts.event_count(&bank).await.expect("count");
        let ghost = bc_models::TagId::new().to_string();

        let (cmd, args) = if update {
            (
                commands::UPDATE_ACCOUNT,
                json!({
                    "id": bank.to_string(),
                    "expected_events": count,
                    "patch": { "tag_ids": [ghost] },
                }),
            )
        } else {
            (
                commands::CREATE_ACCOUNT,
                json!({
                    "name": "Other",
                    "parent_id": null,
                    "account_type": "Asset",
                    "kind": "DepositAccount",
                    "description": null,
                    "commodity_codes": [],
                    "tag_ids": [ghost],
                    "opened_on": null,
                    "acquisition": null,
                }),
            )
        };
        let result: Result<bc_ipc::AccountDetail, _> = run(&state, cmd, args).await;

        assert!(
            matches!(result, Err(bc_ipc::BcError::Validation(ref m)) if m.contains(&ghost)),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn list_accounts_still_excludes_archived() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let assets = account(&state, "Assets", bc_models::AccountType::Asset, None).await;
        let old = account(
            &state,
            "Old Bank",
            bc_models::AccountType::Asset,
            Some(&assets),
        )
        .await;
        state
            .accounts
            .archive(&old, bc_core::Cascade::Reject)
            .await
            .expect("archive");

        let nodes: Vec<bc_ipc::AccountNode> = run(&state, commands::LIST_ACCOUNTS, json!({}))
            .await
            .expect("list");

        assert_eq!(
            nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>(),
            vec![assets.to_string()],
            "the sidebar never shows an archived account"
        );
    }

    /// Posts `amount` AUD to `to` on `on`, balanced against `from`.
    async fn transfer(
        state: &AppState,
        from: &bc_models::AccountId,
        to: &bc_models::AccountId,
        on: jiff::civil::Date,
        amount: rust_decimal::Decimal,
    ) {
        let leg = |account: &bc_models::AccountId, value: rust_decimal::Decimal| {
            bc_models::Posting::builder()
                .id(bc_models::PostingId::new())
                .account_id(account.clone())
                .amount(bc_models::Amount::new(
                    value,
                    bc_models::CommodityCode::new("AUD"),
                ))
                .build()
        };
        let tx = bc_models::Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(on)
            .description("Groceries")
            .postings(vec![leg(to, amount), leg(from, amount.neg())])
            .reconciliation(bc_models::Reconciliation::Unreconciled)
            .created_at(jiff::Timestamp::now())
            .build();
        state
            .transactions
            .create(tx)
            .await
            .expect("seed transaction");
    }

    /// A bank with one open child, `Assets` > `Bank` > `Checking`.
    async fn bank_with_child(state: &AppState) -> (bc_models::AccountId, bc_models::AccountId) {
        let assets = account(state, "Assets", bc_models::AccountType::Asset, None).await;
        let bank = account(state, "Bank", bc_models::AccountType::Asset, Some(&assets)).await;
        let checking = account(
            state,
            "Checking",
            bc_models::AccountType::Asset,
            Some(&bank),
        )
        .await;
        (bank, checking)
    }

    async fn detail(state: &AppState, id: &bc_models::AccountId) -> bc_ipc::AccountDetail {
        run(
            state,
            commands::GET_ACCOUNT_DETAIL,
            json!({ "id": id.to_string() }),
        )
        .await
        .expect("detail")
    }

    #[tokio::test]
    async fn close_account_without_cascade_on_open_child_is_validation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let (bank, checking) = bank_with_child(&state).await;

        let result: Result<(), _> = run(
            &state,
            commands::CLOSE_ACCOUNT,
            json!({ "id": bank.to_string(), "on": "2026-10-31", "cascade": false }),
        )
        .await;

        assert!(
            matches!(result, Err(bc_ipc::BcError::Validation(_))),
            "{result:?}"
        );
        assert_eq!(detail(&state, &bank).await.closed_on, None);
        assert_eq!(detail(&state, &checking).await.closed_on, None);
    }

    #[tokio::test]
    async fn close_account_with_cascade_closes_children() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let (bank, checking) = bank_with_child(&state).await;
        let on = jiff::civil::date(2026, 10, 31);

        let () = run(
            &state,
            commands::CLOSE_ACCOUNT,
            json!({ "id": bank.to_string(), "on": "2026-10-31", "cascade": true }),
        )
        .await
        .expect("close");

        assert_eq!(detail(&state, &bank).await.closed_on, Some(on));
        assert_eq!(detail(&state, &checking).await.closed_on, Some(on));

        let () = run(
            &state,
            commands::REOPEN_ACCOUNT,
            json!({ "id": bank.to_string() }),
        )
        .await
        .expect("reopen");
        assert_eq!(detail(&state, &bank).await.closed_on, None);

        let again: Result<(), _> = run(
            &state,
            commands::REOPEN_ACCOUNT,
            json!({ "id": bank.to_string() }),
        )
        .await;
        assert!(
            matches!(again, Err(bc_ipc::BcError::Validation(_))),
            "{again:?}"
        );
    }

    #[tokio::test]
    async fn preview_close_account_reports_balance_and_late_postings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let (bank, checking) = bank_with_child(&state).await;
        let income = account(&state, "Salary", bc_models::AccountType::Income, None).await;
        transfer(
            &state,
            &income,
            &checking,
            jiff::civil::date(2026, 10, 1),
            dec!(100),
        )
        .await;
        transfer(
            &state,
            &income,
            &checking,
            jiff::civil::date(2026, 11, 2),
            dec!(25),
        )
        .await;

        let preview: bc_ipc::ClosePreviewDto = run(
            &state,
            commands::PREVIEW_CLOSE_ACCOUNT,
            json!({ "id": bank.to_string(), "on": "2026-10-31" }),
        )
        .await
        .expect("preview");

        let checking_ref =
            bc_ipc::AccountRef::new(checking.to_string(), "Assets :: Bank :: Checking");
        assert_eq!(preview.cascade, vec![checking_ref.clone()]);
        assert_eq!(
            preview.balances,
            vec![(checking_ref, vec![bc_ipc::Amount::new(dec!(100), "AUD")])]
        );
        assert_eq!(preview.postings_after, 1);
    }

    #[tokio::test]
    async fn archive_then_unarchive_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let (bank, checking) = bank_with_child(&state).await;

        let preview: bc_ipc::ArchivePreviewDto = run(
            &state,
            commands::PREVIEW_ARCHIVE_ACCOUNT,
            json!({ "id": bank.to_string() }),
        )
        .await
        .expect("preview");
        assert_eq!(
            preview.cascade,
            vec![bc_ipc::AccountRef::new(
                checking.to_string(),
                "Assets :: Bank :: Checking"
            )]
        );
        assert!(preview.no_close_date);

        let blocked: Result<(), _> = run(
            &state,
            commands::ARCHIVE_ACCOUNT,
            json!({ "id": bank.to_string(), "cascade": false }),
        )
        .await;
        assert!(
            matches!(blocked, Err(bc_ipc::BcError::Validation(_))),
            "{blocked:?}"
        );

        let () = run(
            &state,
            commands::ARCHIVE_ACCOUNT,
            json!({ "id": bank.to_string(), "cascade": true }),
        )
        .await
        .expect("archive");
        assert!(detail(&state, &bank).await.archived);
        assert!(detail(&state, &checking).await.archived);

        let () = run(
            &state,
            commands::UNARCHIVE_ACCOUNT,
            json!({ "id": bank.to_string() }),
        )
        .await
        .expect("unarchive");
        assert!(!detail(&state, &bank).await.archived);
        assert!(
            detail(&state, &checking).await.archived,
            "unarchiving the parent leaves the child archived"
        );

        let again: Result<(), _> = run(
            &state,
            commands::UNARCHIVE_ACCOUNT,
            json!({ "id": bank.to_string() }),
        )
        .await;
        assert!(
            matches!(again, Err(bc_ipc::BcError::Validation(_))),
            "{again:?}"
        );
    }

    // MARK: Account patch and lifecycle edges

    /// Patches `id` at its current event count.
    async fn patch(
        state: &AppState,
        id: &str,
        patch: serde_json::Value,
    ) -> Result<bc_ipc::AccountDetail, bc_ipc::BcError> {
        let base: bc_ipc::AccountDetail =
            run(state, commands::GET_ACCOUNT_DETAIL, json!({ "id": id }))
                .await
                .expect("base");
        run(
            state,
            commands::UPDATE_ACCOUNT,
            json!({ "id": id, "expected_events": base.event_count, "patch": patch }),
        )
        .await
    }

    /// Creates `Assets` > `Car`, a manual asset carrying every editable field.
    async fn car(state: &AppState) -> bc_ipc::AccountDetail {
        let assets = account(state, "Assets", bc_models::AccountType::Asset, None).await;
        let tag = state
            .tags
            .create_path(&"asset:vehicle".parse().expect("tag path"))
            .await
            .expect("tag");
        run(
            state,
            commands::CREATE_ACCOUNT,
            json!({
                "name": "Car",
                "parent_id": assets.to_string(),
                "account_type": "Asset",
                "kind": "ManualAsset",
                "description": "Hatchback",
                "commodity_codes": ["AUD"],
                "tag_ids": [tag.to_string()],
                "opened_on": "2026-01-15",
                "acquisition": { "date": "2026-01-15", "cost": "12345.00" },
            }),
        )
        .await
        .expect("create")
    }

    #[tokio::test]
    async fn update_account_name_only_leaves_other_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let before = car(&state).await;

        let after = patch(&state, &before.id, json!({ "name": "Van" }))
            .await
            .expect("rename");

        let mut expected = before.clone();
        expected.name = "Van".to_owned();
        expected.path = "Assets :: Van".to_owned();
        expected.event_count = after.event_count;
        assert_eq!(after, expected);
    }

    #[tokio::test]
    async fn update_account_moves_under_a_parent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let assets = account(&state, "Assets", bc_models::AccountType::Asset, None).await;
        let bank = account(&state, "Bank", bc_models::AccountType::Asset, None).await;

        let after = patch(
            &state,
            &bank.to_string(),
            json!({ "parent_id": assets.to_string() }),
        )
        .await
        .expect("move");

        assert_eq!(after.parent_id, Some(assets.to_string()));
        assert_eq!(after.path, "Assets :: Bank");
    }

    #[tokio::test]
    async fn update_account_replaces_tags() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let before = car(&state).await;
        let fleet = state
            .tags
            .create_path(&"asset:fleet".parse().expect("tag path"))
            .await
            .expect("tag");

        let after = patch(
            &state,
            &before.id,
            json!({ "tag_ids": [fleet.to_string()] }),
        )
        .await
        .expect("retag");

        assert_eq!(
            after.tags,
            vec![bc_ipc::TagInfo::new(fleet.to_string(), "asset:fleet")]
        );
    }

    #[tokio::test]
    async fn update_account_corrects_a_closing_date() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let bank = account(&state, "Bank", bc_models::AccountType::Asset, None).await;
        let () = run(
            &state,
            commands::CLOSE_ACCOUNT,
            json!({ "id": bank.to_string(), "on": "2026-10-31", "cascade": false }),
        )
        .await
        .expect("close");

        let after = patch(
            &state,
            &bank.to_string(),
            json!({ "closed_on": "2026-09-30" }),
        )
        .await
        .expect("correct");

        assert_eq!(after.closed_on, Some(jiff::civil::date(2026, 9, 30)));
    }

    #[tokio::test]
    async fn update_account_replaces_an_acquisition() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let before = car(&state).await;

        let after = patch(
            &state,
            &before.id,
            json!({
                "acquisition": {
                    "date": "2026-02-01",
                    "cost": "9999.00",
                    "depreciation_policy": { "type": "straight_line", "annual_rate": "0.20" },
                },
            }),
        )
        .await
        .expect("replace");

        assert_eq!(
            after.acquisition,
            Some(
                bc_ipc::AcquisitionDto::builder()
                    .date(jiff::civil::date(2026, 2, 1))
                    .cost(dec!(9999.00))
                    .depreciation_policy(bc_ipc::DepreciationPolicyDto::StraightLine {
                        annual_rate: dec!(0.20),
                    })
                    .build()
            )
        );
        let fetched: bc_ipc::AccountDetail = run(
            &state,
            commands::GET_ACCOUNT_DETAIL,
            json!({ "id": before.id }),
        )
        .await
        .expect("get");
        assert_eq!(fetched, after, "the replacement is stored");
    }

    #[tokio::test]
    async fn update_account_acquisition_on_a_deposit_account_is_validation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let bank = account(&state, "Bank", bc_models::AccountType::Asset, None).await;

        let result = patch(
            &state,
            &bank.to_string(),
            json!({ "acquisition": { "date": "2026-02-01", "cost": "100.00" } }),
        )
        .await;

        assert!(
            matches!(result, Err(bc_ipc::BcError::Validation(_))),
            "{result:?}"
        );
    }

    /// A good name with a closing date on an open account: core refuses the
    /// closing date, and the name must not land either.
    #[tokio::test]
    async fn update_account_with_one_bad_field_changes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let before = car(&state).await;

        let result = patch(
            &state,
            &before.id,
            json!({ "name": "Van", "closed_on": "2026-09-30" }),
        )
        .await;

        assert!(
            matches!(result, Err(bc_ipc::BcError::Validation(_))),
            "{result:?}"
        );
        let id: bc_models::AccountId = before.id.parse().expect("id");
        assert_eq!(detail(&state, &id).await, before);
    }

    #[tokio::test]
    async fn close_account_before_its_opening_is_validation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let created = car(&state).await;

        let result: Result<(), _> = run(
            &state,
            commands::CLOSE_ACCOUNT,
            json!({ "id": created.id, "on": "2026-01-14", "cascade": false }),
        )
        .await;

        assert!(
            matches!(result, Err(bc_ipc::BcError::Validation(_))),
            "{result:?}"
        );
        let id: bc_models::AccountId = created.id.parse().expect("id");
        assert_eq!(detail(&state, &id).await.closed_on, None);
    }

    #[tokio::test]
    async fn unarchive_account_under_an_archived_parent_is_validation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let (bank, checking) = bank_with_child(&state).await;
        let () = run(
            &state,
            commands::ARCHIVE_ACCOUNT,
            json!({ "id": bank.to_string(), "cascade": true }),
        )
        .await
        .expect("archive");

        let result: Result<(), _> = run(
            &state,
            commands::UNARCHIVE_ACCOUNT,
            json!({ "id": checking.to_string() }),
        )
        .await;

        assert!(
            matches!(result, Err(bc_ipc::BcError::Validation(_))),
            "{result:?}"
        );
        assert!(detail(&state, &checking).await.archived);
    }

    #[rstest]
    #[case::get(commands::GET_ACCOUNT_DETAIL, json!({}))]
    #[case::update(
        commands::UPDATE_ACCOUNT,
        json!({ "expected_events": 1_u64, "patch": { "name": "Renamed" } })
    )]
    #[case::close(commands::CLOSE_ACCOUNT, json!({ "on": "2026-10-31", "cascade": false }))]
    #[case::reopen(commands::REOPEN_ACCOUNT, json!({}))]
    #[case::archive(commands::ARCHIVE_ACCOUNT, json!({ "cascade": false }))]
    #[case::unarchive(commands::UNARCHIVE_ACCOUNT, json!({}))]
    #[tokio::test]
    async fn malformed_account_id_is_validation(
        #[case] cmd: &str,
        #[case] mut args: serde_json::Value,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        args.as_object_mut()
            .expect("object")
            .insert("id".to_owned(), json!("not-an-id"));

        let result = crate::dispatch(&state, cmd, args).await;

        assert!(
            matches!(result, Err(bc_ipc::BcError::Validation(_))),
            "{result:?}"
        );
    }
}
