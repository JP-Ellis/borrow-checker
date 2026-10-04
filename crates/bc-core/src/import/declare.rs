//! Applies an importer's account declarations before any posting resolves.
//!
//! The decisions are pure: [`plan_open`] and [`plan_close`] compare one
//! declaration against the account's stored state and return the steps it
//! asks for. [`apply`] drives them in source order for both sinks. A commit
//! writes each step and records it against the batch; a plan only updates the
//! run's account snapshot.

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;

use bc_models::Account;
use bc_models::AccountId;
use bc_models::AccountKind;
use bc_models::Commodity;
use bc_models::CommodityCode;
use bc_models::CommodityId;
use bc_models::ImportBatchId;
use jiff::civil::Date;

use crate::AccountClose;
use crate::AccountOpen;
use crate::AccountPath;
use crate::AccountResolver;
use crate::BcError;
use crate::BcResult;
use crate::CommodityResolver;
use crate::Declaration;
use crate::DeclaredField;
use crate::Diagnostic;
use crate::ImportBatchAccountRecord;
use crate::PathSpec;
use crate::Resolution;
use crate::SkipCause;
use crate::SourceLocation;
use crate::Warning;
use crate::account::Cascade;

// MARK: Driver

/// Where the driver's steps go.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Writes<'svc> {
    /// Write each step and record it against the batch.
    Commit {
        /// Import batch provenance service.
        batches: &'svc crate::ImportBatchService,
        /// The open batch.
        batch_id: &'svc ImportBatchId,
    },
    /// Write nothing: every step is assumed to apply.
    Plan,
}

/// What applying a run's declarations produced, merged into the run before
/// legs resolve.
#[derive(Debug, Default)]
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "`import_exec::run_with` merges each field into its own worklist"
)]
pub(crate) struct Declared {
    /// Account paths created, or that would be, sorted, ancestors included.
    pub(crate) created: Vec<String>,
    /// Conflicts and refusals, in source order.
    pub(crate) warnings: Vec<Warning>,
    /// Declarations or parts of them that could not apply, in source order.
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// Paths a `close` named that resolve to no account.
    pub(crate) unresolved_accounts: BTreeSet<String>,
    /// Codes an `open` named that resolve to no single commodity.
    pub(crate) unresolved_commodities: BTreeSet<String>,
}

/// Applies `declarations` in source order.
///
/// Every account created, or that would be, enters `resolver`, so legs into
/// it resolve in the same run. A second `open` or `close` of one path is
/// diagnosed and skipped.
///
/// # Arguments
///
/// * `writes` - Whether to write, and where to record what was written.
/// * `accounts` - Account service, read for stored state and written in a
///   commit.
/// * `commodities` - The run's commodity code resolver.
/// * `registry` - Every registered commodity, the snapshot `commodities` was
///   built from.
/// * `resolver` - The run's account snapshot, extended with each creation.
/// * `declarations` - Account declarations in source order.
///
/// # Returns
///
/// The created paths, warnings, diagnostics and worklist entries.
///
/// # Errors
///
/// Returns [`BcError`] on database failure. A declaration the account service
/// refuses is a warning or diagnostic, never an error.
pub(crate) async fn apply(
    writes: Writes<'_>,
    accounts: &crate::AccountService,
    commodities: &CommodityResolver,
    registry: &[Commodity],
    resolver: &mut AccountResolver,
    declarations: &[Declaration],
) -> BcResult<Declared> {
    let mut driver = Driver {
        writes,
        accounts,
        registry,
        resolver,
        state: HashMap::new(),
        out: Declared::default(),
    };
    let mut seen: HashSet<(&'static str, String)> = HashSet::new();

    for declaration in declarations {
        let (kind, stated, source) = match *declaration {
            Declaration::Open(ref open) => ("open", &open.account, &open.source_location),
            Declaration::Close(ref close) => ("close", &close.account, &close.source_location),
        };
        let location = location_of(source.as_ref());
        let path = match AccountPath::parse(stated) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(
                    location,
                    account = stated.as_str(),
                    %error,
                    "malformed account path; skipping this declaration"
                );
                driver.note(location, SkipCause::MalformedPath, stated.clone());
                continue;
            }
        };
        let rendered = path.to_string();
        if !seen.insert((kind, rendered.clone())) {
            driver.note(
                location,
                SkipCause::IgnoredDeclaration,
                format!("duplicate {kind} for {rendered}; the first in source order applies"),
            );
            continue;
        }

        match *declaration {
            Declaration::Open(ref open) => {
                driver
                    .open(open, path, &rendered, location, commodities)
                    .await?;
            }
            Declaration::Close(ref close) => {
                driver.close(close, &path, &rendered, location).await?;
            }
        }
    }

    let mut out = driver.out;
    out.created.sort_unstable();
    out.created.dedup();
    Ok(out)
}

/// Returns the display text of a declaration's location.
///
/// # Arguments
///
/// * `source` - Where the importer said the declaration came from.
///
/// # Returns
///
/// The display text, or `<unknown source>`.
fn location_of(source: Option<&SourceLocation>) -> &str {
    source.map_or("<unknown source>", |location| location.display.as_str())
}

/// Reports whether `error` is the account service refusing a declaration
/// rather than a failure of the database underneath it.
///
/// # Arguments
///
/// * `error` - What the service returned.
///
/// # Returns
///
/// `true` when the import may warn and continue.
fn is_refusal(error: &BcError) -> bool {
    matches!(
        *error,
        BcError::BadData(_)
            | BcError::InvalidInput(_)
            | BcError::NotFound(_)
            | BcError::AlreadyClosed(_)
            | BcError::AlreadyArchived(_)
    )
}

/// The state one [`apply`] call threads through its declarations.
struct Driver<'run> {
    /// Where steps go.
    writes: Writes<'run>,
    /// Account service.
    accounts: &'run crate::AccountService,
    /// Every registered commodity.
    registry: &'run [Commodity],
    /// The run's account snapshot.
    resolver: &'run mut AccountResolver,
    /// Each account a declaration has named, as the run now holds it: read
    /// from the database once, or built for an account the run created.
    /// Opening and closing dates follow each applied step. The commodity list
    /// does not: only a second `open` of the same path would read it, and that
    /// is skipped as a duplicate.
    state: HashMap<AccountId, Account>,
    /// What the run has produced so far.
    out: Declared,
}

impl Driver<'_> {
    /// Records a diagnostic.
    ///
    /// # Arguments
    ///
    /// * `location` - The declaration's location.
    /// * `cause` - Why it, or part of it, did not apply.
    /// * `detail` - The offending path, code or reason.
    fn note(&mut self, location: &str, cause: SkipCause, detail: String) {
        self.out.diagnostics.push(Diagnostic {
            location: location.to_owned(),
            cause,
            detail,
        });
    }

    /// Returns the account `id` as the run now holds it.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] if the account cannot be read.
    async fn stored(&mut self, id: &AccountId) -> BcResult<Account> {
        if let Some(account) = self.state.get(id) {
            return Ok(account.clone());
        }
        let account = self.accounts.find_by_id(id).await?;
        self.state.insert(id.clone(), account.clone());
        Ok(account)
    }

    /// Applies one `open`.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure.
    async fn open(
        &mut self,
        open: &AccountOpen,
        path: AccountPath,
        rendered: &str,
        location: &str,
        commodities: &CommodityResolver,
    ) -> BcResult<()> {
        let commodity_ids = self.commodity_ids(&open.commodities, location, commodities);
        let stored = match self.resolver.resolve(&path) {
            Resolution::Resolved { ref id, .. } => Some(self.stored(id).await?),
            Resolution::Missing { .. } => None,
        };
        let steps = plan_open(
            stored.as_ref(),
            path,
            rendered,
            open.date,
            commodity_ids,
            self.registry,
        );
        for step in steps {
            self.execute(step, rendered, location).await?;
        }
        Ok(())
    }

    /// Applies one `close`. A path naming no account joins the unresolved
    /// worklist; a `close` never creates.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure.
    async fn close(
        &mut self,
        close: &AccountClose,
        path: &AccountPath,
        rendered: &str,
        location: &str,
    ) -> BcResult<()> {
        let id = match self.resolver.resolve(path) {
            Resolution::Resolved { id, .. } => id,
            Resolution::Missing {
                resolved_prefix,
                missing_segment,
            } => {
                if self.out.unresolved_accounts.insert(rendered.to_owned()) {
                    tracing::warn!(
                        location,
                        account = rendered,
                        "close names no existing account; nothing to close"
                    );
                }
                self.note(
                    location,
                    SkipCause::UnresolvedAccount,
                    format!(
                        "{rendered} (resolved as far as '{resolved_prefix}', missing \
                         '{missing_segment}')"
                    ),
                );
                return Ok(());
            }
        };
        let stored = self.stored(&id).await?;
        for step in plan_close(&stored, rendered, close.date) {
            self.execute(step, rendered, location).await?;
        }
        Ok(())
    }

    /// Resolves an `open`'s codes to commodity ids, in order.
    ///
    /// A code resolves through `commodities` to its registered spelling, then
    /// to the exchange-less commodity of that code, or the only one. A code
    /// that resolves to nothing, or to several listings with none
    /// exchange-less, joins the worklist with a diagnostic.
    ///
    /// # Returns
    ///
    /// The ids that resolved, first occurrence kept.
    fn commodity_ids(
        &mut self,
        codes: &[String],
        location: &str,
        commodities: &CommodityResolver,
    ) -> Vec<CommodityId> {
        let mut ids: Vec<CommodityId> = Vec::new();
        for code in codes {
            let Some(canonical) = commodities.resolve(&CommodityCode::new(code.as_str())) else {
                self.out.unresolved_commodities.insert(code.clone());
                self.note(location, SkipCause::UnresolvedCommodity, code.clone());
                continue;
            };
            let listed: Vec<&Commodity> = self
                .registry
                .iter()
                .filter(|commodity| commodity.code() == canonical)
                .collect();
            let chosen = listed
                .iter()
                .find(|commodity| commodity.exchange().is_none())
                .or_else(|| listed.first().filter(|_| listed.len() == 1));
            let Some(commodity) = chosen else {
                self.out.unresolved_commodities.insert(canonical.to_owned());
                self.note(
                    location,
                    SkipCause::UnresolvedCommodity,
                    format!("{canonical} (listed on several exchanges, none without one)"),
                );
                continue;
            };
            if !ids.contains(commodity.id()) {
                ids.push(commodity.id().clone());
            }
        }
        ids
    }

    /// Carries out one step.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure.
    async fn execute(&mut self, step: Step, rendered: &str, location: &str) -> BcResult<()> {
        match step {
            Step::Conflict(warning) => {
                tracing::warn!(location, %warning, "declaration conflicts with the stored account");
                self.out.warnings.push(warning);
            }
            Step::Create {
                path,
                opened_on,
                commodity_ids,
            } => {
                self.create(path, opened_on, commodity_ids, rendered, location)
                    .await?;
            }
            Step::SetOpenedOn { id, to } => {
                if let Writes::Commit { batches, batch_id } = self.writes {
                    let result = self.accounts.set_opened_on(&id, Some(to)).await;
                    if self.refused(result, &id, rendered, location)? {
                        return Ok(());
                    }
                    let record = ImportBatchAccountRecord::builder()
                        .account_id(id.clone())
                        .opened_on(to)
                        .build();
                    batches.record_accounts(batch_id, &[record]).await?;
                }
                if let Some(account) = self.state.get_mut(&id) {
                    account.set_opened_on(Some(to));
                }
            }
            Step::SetCommodities { id, to } => {
                if let Writes::Commit { batches, batch_id } = self.writes {
                    let result = self.accounts.set_commodities(&id, &to).await;
                    if self.refused(result, &id, rendered, location)? {
                        return Ok(());
                    }
                    let record = ImportBatchAccountRecord::builder()
                        .account_id(id)
                        .commodities(to)
                        .build();
                    batches.record_accounts(batch_id, &[record]).await?;
                }
            }
            Step::Close { id, on } => {
                if let Writes::Commit { batches, batch_id } = self.writes {
                    let result = self.accounts.close(&id, on, Cascade::Reject).await;
                    if self.refused(result, &id, rendered, location)? {
                        return Ok(());
                    }
                    let record = ImportBatchAccountRecord::builder()
                        .account_id(id.clone())
                        .closed_on(on)
                        .build();
                    batches.record_accounts(batch_id, &[record]).await?;
                }
                if let Some(account) = self.state.get_mut(&id) {
                    account.set_closed_on(Some(on));
                }
            }
        }
        Ok(())
    }

    /// Turns a service refusal into a [`Warning::DeclarationNotApplied`].
    ///
    /// # Arguments
    ///
    /// * `result` - What the service returned.
    /// * `id` - The account the step named.
    /// * `rendered` - Its path.
    /// * `location` - The declaration's location.
    ///
    /// # Returns
    ///
    /// `true` when the service refused the step.
    ///
    /// # Errors
    ///
    /// Returns the service's error when it is not a refusal.
    fn refused(
        &mut self,
        result: BcResult<()>,
        id: &AccountId,
        rendered: &str,
        location: &str,
    ) -> BcResult<bool> {
        match result {
            Ok(()) => Ok(false),
            Err(error) if is_refusal(&error) => {
                tracing::warn!(location, account = rendered, %error, "declaration not applied");
                self.out.warnings.push(Warning::DeclarationNotApplied {
                    account_id: id.clone(),
                    account_path: rendered.to_owned(),
                    reason: error.to_string(),
                });
                Ok(true)
            }
            Err(error) => Err(error),
        }
    }

    /// Creates `path`'s missing segments, or decides that it would.
    ///
    /// The account tree's own creation rules are checked first, against the
    /// run's snapshot, so a plan diagnoses the paths a commit would be refused:
    /// a root segment naming no account type, or a missing segment under a
    /// closed or archived account.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure.
    async fn create(
        &mut self,
        path: AccountPath,
        opened_on: Date,
        commodity_ids: Vec<CommodityId>,
        rendered: &str,
        location: &str,
    ) -> BcResult<()> {
        let existing = self.existing_prefix(&path);
        let segments = path.segments();

        let account_type = match self.creation_rules(&existing, segments).await? {
            Ok(account_type) => account_type,
            Err(reason) => {
                self.refuse_create(rendered, location, &reason);
                return Ok(());
            }
        };

        let (minted, created) = match self.writes {
            Writes::Commit { batches, batch_id } => {
                let spec = PathSpec::builder()
                    .path(path.clone())
                    .opened_on(opened_on)
                    .commodity_ids(commodity_ids.clone())
                    .build();
                let created = match self.accounts.create_paths(&[spec]).await {
                    Ok(created) => created,
                    Err(error) if is_refusal(&error) => {
                        self.refuse_create(rendered, location, &error.to_string());
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                };
                let records: Vec<ImportBatchAccountRecord> = created
                    .minted
                    .iter()
                    .map(|id| {
                        ImportBatchAccountRecord::builder()
                            .account_id(id.clone())
                            .created(true)
                            .build()
                    })
                    .collect();
                batches.record_accounts(batch_id, &records).await?;
                (created.minted, created.created)
            }
            Writes::Plan => {
                let created: Vec<String> = prefixes(segments)
                    .into_iter()
                    .skip(existing.len())
                    .collect();
                let minted = created.iter().map(|_| AccountId::new()).collect();
                (minted, created)
            }
        };

        let mut parent = existing.last().cloned();
        for (index, (segment, id)) in segments
            .iter()
            .zip(existing.iter().chain(&minted))
            .enumerate()
            .skip(existing.len())
        {
            let is_leaf = index.saturating_add(1) == segments.len();
            let account = Account::builder()
                .id(id.clone())
                .name(segment.clone())
                .account_type(account_type)
                .kind(if is_leaf {
                    AccountKind::DepositAccount
                } else {
                    AccountKind::Group
                })
                .maybe_parent_id(parent.clone())
                .commodities(if is_leaf {
                    commodity_ids.clone()
                } else {
                    Vec::new()
                })
                .maybe_opened_on(is_leaf.then_some(opened_on))
                .build();
            self.state.insert(id.clone(), account);
            parent = Some(id.clone());
        }

        let ids: Vec<AccountId> = existing.into_iter().chain(minted).collect();
        self.resolver.insert(&path, &ids);
        self.out.created.extend(created);
        Ok(())
    }

    /// Checks the account tree's creation rules for `path`'s missing segments.
    ///
    /// # Arguments
    ///
    /// * `existing` - The ids of the leading segments that exist, root first.
    /// * `segments` - Every segment of the path.
    ///
    /// # Returns
    ///
    /// The account type the new segments take, or why the tree refuses them.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] if a stored account cannot be read.
    async fn creation_rules(
        &mut self,
        existing: &[AccountId],
        segments: &[String],
    ) -> BcResult<Result<bc_models::AccountType, String>> {
        let account_type = if let Some(root) = existing.first() {
            self.stored(root).await?.account_type()
        } else {
            let root = segments.first().map_or("", String::as_str);
            let Some(derived) = crate::account::derive_account_type(root) else {
                return Ok(Err(format!(
                    "cannot derive an account type from root segment '{root}'"
                )));
            };
            derived
        };
        if let Some(parent_id) = existing.last() {
            let parent = self.stored(parent_id).await?;
            if parent.archived_at().is_some() {
                return Ok(Err(format!(
                    "cannot create under archived account '{}'",
                    parent.name()
                )));
            }
            if let Some(closed_on) = parent.closed_on() {
                return Ok(Err(format!(
                    "cannot create under account '{}', which closed on {closed_on}",
                    parent.name()
                )));
            }
        }
        Ok(Ok(account_type))
    }

    /// Diagnoses an `open` whose account the tree refuses to create.
    ///
    /// # Arguments
    ///
    /// * `rendered` - The declared path.
    /// * `location` - The declaration's location.
    /// * `reason` - Why the tree refuses it.
    fn refuse_create(&mut self, rendered: &str, location: &str, reason: &str) {
        tracing::warn!(
            location,
            account = rendered,
            reason,
            "declared account not created"
        );
        self.note(
            location,
            SkipCause::IgnoredDeclaration,
            format!("{rendered}: {reason}"),
        );
    }

    /// Returns the ids of `path`'s leading segments the snapshot holds, root
    /// first.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to walk.
    ///
    /// # Returns
    ///
    /// One id per segment up to the first missing one.
    fn existing_prefix(&self, path: &AccountPath) -> Vec<AccountId> {
        let mut ids = Vec::new();
        for prefix in prefixes(path.segments()) {
            let Ok(parsed) = AccountPath::parse(&prefix) else {
                break;
            };
            match self.resolver.resolve(&parsed) {
                Resolution::Resolved { id, .. } => ids.push(id),
                Resolution::Missing { .. } => break,
            }
        }
        ids
    }
}

/// Renders every leading prefix of a path, shortest first.
///
/// # Arguments
///
/// * `segments` - The path's segments, root first.
///
/// # Returns
///
/// One colon-joined prefix per segment, ending with the whole path.
fn prefixes(segments: &[String]) -> Vec<String> {
    let mut walked: Vec<&str> = Vec::with_capacity(segments.len());
    segments
        .iter()
        .map(|segment| {
            walked.push(segment);
            walked.join(":")
        })
        .collect()
}

// MARK: Decisions

/// What one declaration asks of one account, given what is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// Create the account, and every missing ancestor, with these fields.
    Create {
        /// The path to materialise.
        path: AccountPath,
        /// The leaf's opening date.
        opened_on: Date,
        /// The leaf's commodities, first is the default.
        commodity_ids: Vec<CommodityId>,
    },
    /// Fill an empty opening date.
    SetOpenedOn {
        /// The account to fill.
        id: AccountId,
        /// The declared date.
        to: Date,
    },
    /// Fill an empty commodity list.
    SetCommodities {
        /// The account to fill.
        id: AccountId,
        /// The declared commodities, in order.
        to: Vec<CommodityId>,
    },
    /// Close an open account.
    Close {
        /// The account to close.
        id: AccountId,
        /// The declared closing date.
        on: Date,
    },
    /// The declaration disagrees with a stored value, which is kept.
    Conflict(Warning),
}

/// Decides the steps for an `open` against the account's stored state.
///
/// Each field is compared independently. An empty field is filled, an equal
/// one is left alone, and a different one is kept with a conflict warning. An
/// empty `commodity_ids` states nothing about the commodities, so the stored
/// list is neither compared nor changed.
///
/// # Arguments
///
/// * `stored` - The account the path resolves to, or `None` when it resolves
///   to nothing.
/// * `path` - The declared path.
/// * `path_str` - The declared path, rendered.
/// * `date` - The declared opening date.
/// * `commodity_ids` - The declared commodities that resolved, in order.
/// * `registry` - Every registered commodity, for rendering a conflict.
///
/// # Returns
///
/// The steps, in field order: opening date first, then commodities.
pub(crate) fn plan_open(
    stored: Option<&Account>,
    path: AccountPath,
    path_str: &str,
    date: Date,
    commodity_ids: Vec<CommodityId>,
    registry: &[Commodity],
) -> Vec<Step> {
    let Some(account) = stored else {
        return vec![Step::Create {
            path,
            opened_on: date,
            commodity_ids,
        }];
    };
    let mut steps = Vec::new();

    match account.opened_on() {
        None => steps.push(Step::SetOpenedOn {
            id: account.id().clone(),
            to: date,
        }),
        Some(stored_on) if stored_on == date => {}
        Some(stored_on) => steps.push(conflict(
            account,
            path_str,
            DeclaredField::OpenedOn,
            stored_on.to_string(),
            date.to_string(),
        )),
    }

    if commodity_ids.is_empty() {
        return steps;
    }
    let held = account.commodities();
    if held.is_empty() {
        steps.push(Step::SetCommodities {
            id: account.id().clone(),
            to: commodity_ids,
        });
    } else if held.iter().collect::<HashSet<_>>() != commodity_ids.iter().collect::<HashSet<_>>() {
        steps.push(conflict(
            account,
            path_str,
            DeclaredField::Commodities,
            render_codes(held, registry),
            render_codes(&commodity_ids, registry),
        ));
    }
    steps
}

/// Decides the steps for a `close` against the account's stored state.
///
/// # Arguments
///
/// * `stored` - The account the path resolves to. A path resolving to
///   nothing is the caller's unresolved-account case and never reaches here.
/// * `path_str` - The declared path, rendered.
/// * `date` - The declared closing date.
///
/// # Returns
///
/// [`Step::Close`] for an open account, nothing for an equal date, and a
/// conflict for a different one.
pub(crate) fn plan_close(stored: &Account, path_str: &str, date: Date) -> Vec<Step> {
    match stored.closed_on() {
        None => vec![Step::Close {
            id: stored.id().clone(),
            on: date,
        }],
        Some(stored_on) if stored_on == date => Vec::new(),
        Some(stored_on) => vec![conflict(
            stored,
            path_str,
            DeclaredField::ClosedOn,
            stored_on.to_string(),
            date.to_string(),
        )],
    }
}

/// Builds the conflict step for one field.
///
/// # Arguments
///
/// * `account` - The account whose stored value is kept.
/// * `path_str` - Its rendered path.
/// * `field` - The field that disagrees.
/// * `stored` - The stored value, rendered.
/// * `declared` - The declared value, rendered.
///
/// # Returns
///
/// A [`Step::Conflict`] carrying a [`Warning::DeclarationConflict`].
fn conflict(
    account: &Account,
    path_str: &str,
    field: DeclaredField,
    stored: String,
    declared: String,
) -> Step {
    Step::Conflict(Warning::DeclarationConflict {
        account_id: account.id().clone(),
        account_path: path_str.to_owned(),
        field,
        stored,
        declared,
    })
}

/// Renders commodity ids as their codes, joined by `", "`.
///
/// # Arguments
///
/// * `ids` - The ids to render, in order.
/// * `registry` - Every registered commodity.
///
/// # Returns
///
/// The codes; an id the registry does not hold renders as the id itself.
fn render_codes(ids: &[CommodityId], registry: &[Commodity]) -> String {
    ids.iter()
        .map(|id| {
            registry
                .iter()
                .find(|commodity| commodity.id() == id)
                .map_or_else(|| id.to_string(), |commodity| commodity.code().to_owned())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::sync::LazyLock;

    use bc_models::AccountType;
    use jiff::Timestamp;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    static AUD: LazyLock<CommodityId> = LazyLock::new(CommodityId::new);
    static XTS: LazyLock<CommodityId> = LazyLock::new(CommodityId::new);

    fn aud() -> CommodityId {
        AUD.clone()
    }

    fn xts() -> CommodityId {
        XTS.clone()
    }

    fn registry() -> Vec<Commodity> {
        vec![
            Commodity::builder()
                .id(aud())
                .code("AUD")
                .decimals(2)
                .build(),
            Commodity::builder()
                .id(xts())
                .code("XTS")
                .decimals(2)
                .build(),
        ]
    }

    fn d(year: i16, month: i8, day: i8) -> Date {
        jiff::civil::date(year, month, day)
    }

    /// The stored state of `Assets:Bank:Checking`, built through the real
    /// `Account` builder.
    #[derive(Default)]
    struct Acct {
        opened_on: Option<Date>,
        closed_on: Option<Date>,
        commodities: Vec<CommodityId>,
        archived: bool,
    }

    impl Acct {
        fn opened_on(mut self, date: Date) -> Self {
            self.opened_on = Some(date);
            self
        }

        fn closed_on(mut self, date: Date) -> Self {
            self.closed_on = Some(date);
            self
        }

        fn commodity_ids(mut self, ids: Vec<CommodityId>) -> Self {
            self.commodities = ids;
            self
        }

        fn archived(mut self) -> Self {
            self.archived = true;
            self
        }

        fn build(self) -> Account {
            Account::builder()
                .name("Checking")
                .account_type(AccountType::Asset)
                .parent_id(AccountId::new())
                .commodities(self.commodities)
                .maybe_opened_on(self.opened_on)
                .maybe_closed_on(self.closed_on)
                .maybe_archived_at(self.archived.then(Timestamp::now))
                .build()
        }
    }

    fn acct() -> Acct {
        Acct::default()
    }

    /// One declared `open` of `Assets:Bank:Checking`.
    struct OpenFixture {
        path: &'static str,
        date: Date,
        codes: Vec<&'static str>,
    }

    impl OpenFixture {
        fn path(&self) -> AccountPath {
            AccountPath::parse(self.path).expect("a valid path")
        }

        fn ids(&self) -> Vec<CommodityId> {
            self.codes
                .iter()
                .map(|code| match *code {
                    "AUD" => aud(),
                    "XTS" => xts(),
                    other => panic!("no fixture commodity {other}"),
                })
                .collect()
        }
    }

    fn open(date: Date, codes: &[&'static str]) -> OpenFixture {
        OpenFixture {
            path: "Assets:Bank:Checking",
            date,
            codes: codes.to_vec(),
        }
    }

    /// Renders step kinds, a conflict by its field, joined by `,`.
    fn describe(steps: &[Step]) -> String {
        steps
            .iter()
            .map(|step| match *step {
                Step::Create { .. } => "Create".to_owned(),
                Step::SetOpenedOn { .. } => "SetOpenedOn".to_owned(),
                Step::SetCommodities { .. } => "SetCommodities".to_owned(),
                Step::Close { .. } => "Close".to_owned(),
                Step::Conflict(Warning::DeclarationConflict { field, .. }) => {
                    format!("Conflict({field})")
                }
                Step::Conflict(ref other) => format!("Conflict({other})"),
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    #[rstest]
    #[case::missing_creates(None, open(d(2019, 3, 1), &["AUD"]), "Create")]
    #[case::null_opened_on_fills(Some(acct().build()), open(d(2019, 3, 1), &[]), "SetOpenedOn")]
    #[case::equal_opened_on_noop(
        Some(acct().opened_on(d(2019, 3, 1)).build()),
        open(d(2019, 3, 1), &[]),
        ""
    )]
    #[case::different_opened_on_conflicts(
        Some(acct().opened_on(d(2018, 1, 1)).build()),
        open(d(2019, 3, 1), &[]),
        "Conflict(opened_on)"
    )]
    #[case::empty_list_fills(
        Some(acct().opened_on(d(2019, 3, 1)).build()),
        open(d(2019, 3, 1), &["AUD"]),
        "SetCommodities"
    )]
    #[case::same_set_other_order_noop(
        Some(acct().opened_on(d(2019, 3, 1)).commodity_ids(vec![xts(), aud()]).build()),
        open(d(2019, 3, 1), &["AUD", "XTS"]),
        ""
    )]
    #[case::different_set_conflicts(
        Some(acct().opened_on(d(2019, 3, 1)).commodity_ids(vec![aud()]).build()),
        open(d(2019, 3, 1), &["XTS"]),
        "Conflict(commodities)"
    )]
    #[case::declared_empty_leaves_list(
        Some(acct().opened_on(d(2019, 3, 1)).commodity_ids(vec![aud()]).build()),
        open(d(2019, 3, 1), &[]),
        ""
    )]
    #[case::archived_fills(Some(acct().archived().build()), open(d(2019, 3, 1), &[]), "SetOpenedOn")]
    #[case::closed_fills(
        Some(acct().closed_on(d(2020, 1, 1)).build()),
        open(d(2019, 3, 1), &[]),
        "SetOpenedOn"
    )]
    #[case::both_fields_fill(Some(acct().build()), open(d(2019, 3, 1), &["AUD"]), "SetOpenedOn,SetCommodities")]
    fn open_steps(
        #[case] stored: Option<Account>,
        #[case] decl: OpenFixture,
        #[case] expected: &str,
    ) {
        let steps = plan_open(
            stored.as_ref(),
            decl.path(),
            "Assets:Bank:Checking",
            decl.date,
            decl.ids(),
            &registry(),
        );
        assert_eq!(describe(&steps), expected);
    }

    #[rstest]
    #[case::null_closes(acct().build(), d(2020, 1, 1), "Close")]
    #[case::equal_noop(acct().closed_on(d(2020, 1, 1)).build(), d(2020, 1, 1), "")]
    #[case::different_conflicts(
        acct().closed_on(d(2019, 12, 31)).build(),
        d(2020, 1, 1),
        "Conflict(closed_on)"
    )]
    fn close_steps(#[case] stored: Account, #[case] date: Date, #[case] expected: &str) {
        let steps = plan_close(&stored, "Assets:Bank:Checking", date);
        assert_eq!(describe(&steps), expected);
    }

    /// A commodity conflict names both lists by code, so the warning reads
    /// without a lookup.
    #[test]
    fn a_commodity_conflict_renders_codes() {
        let account = acct()
            .opened_on(d(2019, 3, 1))
            .commodity_ids(vec![aud(), xts()])
            .build();
        let steps = plan_open(
            Some(&account),
            AccountPath::parse("Assets:Bank:Checking").expect("a valid path"),
            "Assets:Bank:Checking",
            d(2019, 3, 1),
            vec![xts()],
            &registry(),
        );
        let [
            Step::Conflict(Warning::DeclarationConflict {
                ref stored,
                ref declared,
                ..
            }),
        ] = steps[..]
        else {
            panic!("expected one commodity conflict, got {steps:?}");
        };
        assert_eq!((stored.as_str(), declared.as_str()), ("AUD, XTS", "XTS"));
    }
}
