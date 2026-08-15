# BorrowChecker — Design Specification

**Date:** 2026-03-20
**Updated:** 2026-06-14
**Status:** Approved
**Pun:** Rust's borrow checker + personal finance (borrowing money)

______________________________________________________________________

## 1. Overview

BorrowChecker is a distributable, open-source personal finance application written in Rust. It targets users who want the transparency and auditability of plain-text accounting tools (ledger, beancount) without the UX tax of writing transactions by hand, wrestling with CSV imports, or manually composing reports.

**Core principles:**

- No lock-in: data is always exportable to open formats
- Plain-text compatibility: ledger and beancount files are first-class citizens
- Extensible: a WASM plugin system lets the community add importers, processors, and reports
- Multiple surfaces: CLI for scripting and automation, Tauri GUI for interactive use

______________________________________________________________________

## 2. Goals

- Full read/write compatibility with ledger and beancount file formats
- SQLite as the internal storage engine (fast, reliable, portable)
- Append-only event log in the core (audit trail, undo/redo, future sync)
- Double-entry accounting enforced at the core level
- Import profiles: account-bound importer configurations that eliminate import ambiguity
- Zero-based budgeting as the default model, with category tracking as a fallback; expense account hierarchy is the category tree (as in ledger/beancount)
- Fortnightly and financial-year periods as first-class budget intervals
- WASM plugin system with explicit ABI versioning and a graceful deprecation/grace-period policy
- Transaction processor pipeline (generalisation of categorisation)
- CLI and Tauri GUI as the two primary surfaces
- Structured CLI output (`--json`) for scripting and automation

## 3. Non-Goals (v1)

- Cloud sync (designed for, built later — Milestone 11)
- Mobile app (stretch goal, Milestone 11)
- Investment/portfolio tracking (post-v1)
- Bank API integrations / Open Banking (post-v1; covered by importers in the meantime)
- Multi-user / shared accounts (post-v1)

______________________________________________________________________

## 4. Architecture

### 4.1 Cargo Workspace Layout

```
borrow-checker/
├── Cargo.toml                  # workspace root
├── crates/
│   ├── bc-models/              # shared domain types (accounts, transactions, budgets, etc.)
│   ├── bc-core/                # engine: event log, SQLite projections, business logic
│   ├── bc-config/              # configuration management (XDG + platform config hierarchy, settings loading)
│   ├── bc-otel/                # OpenTelemetry tracing setup
│   ├── bc-format-csv/          # CSV import (configurable column mapping)
│   ├── bc-format-ledger/       # Ledger read + write
│   ├── bc-format-beancount/    # Beancount read + write
│   ├── bc-format-ofx/          # OFX/QFX import
│   ├── bc-plugins/             # WASM host runtime + plugin ABI bridge
│   ├── bc-sdk/                 # plugin author SDK (published to crates.io separately)
│   ├── bc-cli/                 # CLI binary
│   └── bc-app/                 # Tauri GUI
└── plugins/                    # first-party example/bundled plugins
```

**Design philosophy — keep crates small and focused.** Each crate should have one clear purpose, a well-defined public API, and minimal dependencies. As the project grows it is expected and encouraged to introduce new crates or split existing ones. Utility crates will likely emerge as needed — e.g. `bc-config` (configuration management), `bc-otel` (OpenTelemetry tracing/metrics). Prefer creating a new crate over stuffing shared functionality into an existing one.

**Dependency relationships:**

- `bc-models` has no internal dependencies — it is the shared vocabulary for the whole workspace
- `bc-core` depends on `bc-models`; `bc-cli`, `bc-app` depend on `bc-core`
- `bc-format-*` crates depend on `bc-models` (domain types) and `bc-core` (import profiles, config)
- `bc-plugins` depends on `bc-core` (bridges WASM into the engine)
- `bc-sdk` is standalone — plugin authors only need it, not the full workspace
- `bc-ipc` is the serde wire contract between `bc-app` (native) and `bc-ui` (WASM). It stays a thin contract with no service dependencies; all conversion arrows point *toward* it. It gains an optional dependency on `bc-models` behind a `models` feature (native-only) so DTO↔domain `From`/`TryFrom` impls for basic scalar/enum/`Commodity` values can live in `bc-ipc` without leaking `bc-models` into the WASM build. Domain-walking conversions (account paths, tag resolution, `Transaction`/`AccountNode` assembly) live in `bc-core` as extension traits behind its opt-in `ipc` feature, not in `bc-ipc`. `bc-config`/`bc-plugins` similarly host their own DTO conversions behind an `ipc` feature.

### 4.2 Core Engine (`bc-core`)

The core owns two layers:

**Storage layer (SQLite via `sqlx`):**

| Table | Purpose |
| ------------------ | ---------------------------------------------------- |
| `events` | Append-only event log — never updated, never deleted |
| `accounts` | Projected read model |
| `transactions` | Projected read model |
| `balances` | Projected read model (table exists in M1 schema as a planned cache; M1 queries the `postings` table live — this cache will be populated in a later milestone for performance) |
| `budgets` | Projected read model — budget lines anchored to accounts _(delivered in Milestone 5)_ |
| `asset_valuations` | Projected read model — latest market value per ManualAsset account _(delivered in Milestone 5A)_ |
| `asset_depreciations` | Projected read model — depreciation history per ManualAsset account _(delivered in Milestone 5A)_ |
| `loan_terms` | Projected read model — loan terms per Receivable account _(delivered in Milestone 5A)_ |
| `import_profiles` | Account-bound importer configurations _(delivered in Milestone 2)_ |
| `meta` | Schema version, user preferences, last-sync cursor |

**Event vocabulary:**

```
AccountCreated / AccountUpdated / AccountArchived
TransactionCreated / TransactionAmended / TransactionVoided / TransactionReversed
TransactionPayeeChanged / TransactionDateChanged / TransactionDescriptionChanged
TransactionNoteChanged / TransactionTagsChanged / TransactionExtraDatesChanged
TransactionReconciled
PostingRecategorised / PostingAmountChanged / PostingNoteChanged / PostingSpreadChanged
PostingAdded / PostingRemoved
AssetValuationRecorded
DepreciationCalculated
LoanTermsSet
BudgetCreated / BudgetRevisionSet / BudgetRevisionRemoved / BudgetArchived
TransactionSourceAttached / TransactionSourceDetached
TransactionsMerged / TransactionUnmerged
ImportBatchDiscarded
```

**What the event log provides:**

- Undo/redo — walk the log backward/forward. `ImportBatchDiscarded` is
  deliberately excluded: it carries removal counts, not a snapshot of what was
  removed, so there is nothing to replay it against. Recovery from a discard is
  the pre-discard backup (see §4.6), not the event log.
- Full audit trail — every change is timestamped and sourced
- Time-travel queries — "what was my balance on 1 Jan?"
- Import idempotency — per-account source-reference deduplication
- Future sync — replicate events to a server or mobile device

**Double-entry accounting** is the model: a transaction is a set of postings that
should sum to zero per commodity, consistent with ledger/beancount semantics.
Balance is *derived and advisory*, not an admission requirement — see §4.4.

### 4.3 Account Model

Accounts are classified by `AccountType` (Asset, Liability, Equity, Income, Expense) — the canonical double-entry roots. The five variants are stable and unlikely to change; `#[non_exhaustive]` covers rare future additions.

**Hierarchy via `parent_id`:**

Accounts form an arbitrary-depth tree through an optional `parent_id: Option<AccountId>`. A root account (`parent_id = None`) is the authority for its `AccountType`; child accounts inherit their root's type (enforced in `bc-core` at creation time). The hierarchy supports:

- Institution grouping: `Assets > Bank > Savings, Checking`
- Virtual sub-accounts: `Assets > Bank > Offset > Mine, Partner, Shared`
- Rollups: summing a subtree gives the parent balance; virtual sub-accounts of a joint account should always sum to the real account's bank-statement balance
- Beancount/ledger export: the colon-separated path is derived by walking the ancestor chain

No two sibling accounts share a name (`idx_accounts_sibling_unique`, a `UNIQUE` index on `(parent_id, name)` with the parent folded through `COALESCE` so root accounts — whose `parent_id` is `NULL` — are compared too). This is what makes a colon-separated path resolve to at most one account during import (see §5.2). An archived account still owns its name: renaming or reusing it requires archiving or renaming that account first.

**Account kind** governs how a leaf account's balance is maintained:

| Kind | Description |
| ---------------- | ----------- |
| `DepositAccount` | Reconciles against a bank/card/brokerage statement. May have an import profile. Examples: checking, savings, credit card, investment portfolio. |
| `ManualAsset` | Manually-maintained real asset with no bank statement. Balance driven by valuation events. Examples: real property, vehicle, private equity stake. |
| `Receivable` | Money owed to you by a third party. Tracked via ordinary transactions (disbursement + repayments). May carry optional loan terms for amortization assistance. Examples: personal loan to a friend, loan to a trust. |
| `VirtualAllocation` | No independent existence. Subdivides a parent account's balance. Examples: earmarked sub-accounts within an offset account. |
| `Group` | Organisational node that holds no postings of its own. Created implicitly as a path ancestor when `account create` materialises a nested path, or explicitly via `--kind group`. Examples: `Assets`, `Assets:BankA`, `Expenses:Food`. |

Only `DepositAccount` accounts may have an import profile — enforced in `bc-core` at creation time.

**Cross-cutting labels via an entity-based tag model:**

The primary hierarchy can only express one grouping at a time. Cross-cutting concerns — ownership (mine / partner / shared), institution grouping across types, liquidity flags — are expressed as tag references on accounts.

Tags are first-class entities stored in a `tags` table with `id`, `name`, `parent_id` (self-referential for tag hierarchy), and `description`. `Account` holds `tag_ids: Vec<TagId>` — stable opaque references that survive renames. Human-readable paths are derived on demand via `TagForest::path_of(id) -> TagPath`: a `TagPath` is an ordered sequence of non-empty segments (`["institution", "commbank"]`) that serialises as a colon-joined string (`institution:commbank`).

The `account_tags` join table links accounts to their tags in the database.

Example tag paths: `institution:commbank`, `owner:mine`, `owner:shared`, `liquid`.

**Expense categorisation is the account hierarchy:**

Fine-grained expense categories (`Expenses:Food:Restaurants`, `Expenses:Health:Gym`, etc.) are represented as `Expense`-type accounts in the account tree, exactly as in ledger/beancount. There is no separate envelope or category entity. A `Budget` (see §7) is the mechanism for attaching allocation targets and period rules to any account; multiple budgets can exist per account (e.g., per-person sub-budgets on a shared expense account).

Cross-cutting expense views are handled via tags on postings and accounts, enabling queries like "all food spend regardless of context" or "all spending tagged `person:me`" — without duplicating the account hierarchy.

### 4.4 Transaction Model

A `Transaction` carries a canonical `date` (the sort key) and one narrative
field, `description`: the raw imported narration, usually the only text a bank
export provides. It is never edited after creation and is part of the
deduplication key, so importers can recognise a transaction they have already
seen.

**Everything else annotating a transaction is metadata.** `metadata: Metadata`
holds an ordered list of typed key-value entries, and `Posting` carries the
same field for leg-level annotation. Secondary dates a source supplies
(posted, value, settlement), a cleaned counterparty, a user's note — each is an
ordinary key, none holds a privileged position, and repeated keys are permitted
with insertion order as display order. Every key is registered globally against
one of seven value types (`text`, `number`, `boolean`, `date`, `timestamp`,
`amount`, `account`); a value that will not coerce to its key's type is stored
as text and flagged rather than rejected.

The line between a field and a key is what business logic reads: `date`,
`description`, `reconciliation`, `Posting::amount` and `cost` stay structural
because they carry invariants or drive computation. Beancount draws the same
line — `cost` and `price` are syntax, metadata is the escape hatch.

**Reconciliation is the only status axis.** `enum Reconciliation { Unreconciled, Flagged, Reconciled }`.
An earlier `Pending / Cleared / Voided` conflated three separate concerns:
finalisation is now *structural* (derived balance), voiding is a reversal link,
and reconciliation is what remains. `Flagged` is an attention marker.

**Balance is derived, never stored.** `balanced()` sums postings to zero per
commodity after resolving an elided leg. It is false when there are no concrete
legs, when the residual is non-zero for any commodity, or when two or more legs
are elided.

That derivation extends to *account balances*, not only `balanced()`. An elided
leg absorbs its transaction's residual — the negation of its sibling legs' sum —
and the balance engine resolves it on every read (`bc-core`'s `residual`
module), so it stays correct when a sibling changes. Nothing is stored.

The residual is a **per-commodity vector**: with concrete legs in several
commodities, each commodity's residual is contributed independently and no
rate is ever consulted (FX conversion is #233). `balanced()` is unchanged and
still reports false when more than one commodity remains, so a
multi-commodity residual is flagged while still counting toward balances —
warn, don't block.

Two or more elided legs cannot be written through the app — validation rejects
that shape (see **Storing is permissive** below). The balance engine still
handles it defensively, for a database hand-edited outside the app: such a
transaction has a residual that is real but not attributable to any single leg,
so it contributes to no balance at all. That is what `Residual::Ambiguous` and
`PostingAmount::Ambiguous` represent — a state the reader tolerates, not one
the writer can produce.

**Amount elision.** `Posting.amount: Option<Amount>` — `None` marks the leg that
absorbs the residual, exactly as in ledger and beancount. At most one leg per
transaction may be elided.

**Storing is permissive.** Validation rejects only structurally impossible
transactions: an empty posting list, two or more elided legs (the residual is
ambiguous), or a lone posting that is itself elided (no amount at all anywhere).
Everything else persists — including a fully-concrete unbalanced transaction.

This matters because a one-sided CSV import is the normal case, not an error: a
row saying `Assets:Bank:Checking -$50` with no counter-leg *must* persist so the
account balance moves and the UI can surface it for categorisation. Unbalanced
transactions therefore still count toward balances; `balanced()` is a quality
flag that gates *reconciliation* only, never creation. See §5.2 for how
importers produce these and §4.5 for how views filter them.

A transaction with some legs persisted and others still pending — an account
one of its document legs names does not exist yet — is a normal intermediate
state, not a defect. The elided leg is usually what makes this safe to leave
alone: it keeps its `amount` as `None`, so a later import pass can attach the
pending leg to the same transaction without rewriting anything already stored
(see §5.3). The exception is a row where the elided leg is the *only* one that
resolved; keeping it elided would discard the document's sole statement of
value, so the residual is materialised onto it, and a later pass leaves that
amount as it found it (#350).

Editing a transaction fully replaces its posting set, and import provenance
survives that replace: a modified leg keeps its source reference, and a deleted
leg leaves a tombstoned one (see §5.3), so a re-import neither duplicates the
legs that remain nor resurrects the ones that are gone.

**Tags** apply at both transaction and posting level. A transaction's tags flow
*down* to every posting — never the reverse — so `effective_tag_ids` for a
posting is the union of its own tags and its transaction's. This union is
computed, not materialised.

### 4.5 Query & Filtering (global filter)

One structured, Fava-style filter is shared app-wide: date range, account
subtree, tags, payee/narration text, amount magnitude, and reconciliation.
Dimensions combine with AND; values *within* the account and tag dimensions
combine with OR. Every view recomputes against it.

**The query never prunes.** `Service::search` returns whole transactions
annotated with which legs matched (`MatchedTransaction { transaction, matched_postings }`),
so a consumer decides its own presentation rather than receiving a
pre-truncated, possibly unbalanced transaction. Posting-scoped dimensions
(account, amount, posting tags) distinguish legs; transaction-scoped ones (date,
text, reconciliation, transaction tags) match the whole transaction.

**SQL is a candidate filter; Rust is the source of truth.** The generated SQL
narrows by coarse amount magnitude, producing a deliberate superset; exact
matching happens in Rust via `AmountQuery::matches`. This is not an
optimisation detail — it is what preserves commodity integrity. Comparing
magnitudes in SQL would let `over:USD50` match a BTC amount, so amounts are
never finally compared in SQL anywhere, including the budget actuals path.

Consumers interpret the shared filter through their own lens:

| View | Interpretation |
| ------------- | -------------------------------------------------------------------------- |
| Register | Intersection: the sidebar account is the scope, other dimensions refine it. A filter date bound overrides the period window and disables the period navigator. Non-matching legs are dimmed, never dropped |
| Balances | Transaction-membership: the filter selects a set of transactions; the figure sums *the viewed account's own legs* across them. A muted unfiltered figure is shown alongside for context |
| Sparklines | Same membership rule, bucketed. Filter dates re-anchor the span and drive bucket granularity |
| Budgets | Actuals-only lens: the filter narrows what counts toward actuals; targets never change and no budget is pruned. **The date dimension is ignored** — the period navigator is the sole driver, since a filter range does not align with budget period grids |

### 4.6 Backup & Restore (`bc-core`)

Snapshots are taken via SQLite `VACUUM INTO` to a temp file, then atomically renamed into place — a backup is a standalone file with no `-wal`/`-shm` sidecars.

**Kinds** (encoded in the filename suffix):

| Kind | Trigger |
| --------------- | -------------------------------------------------------------------------------------- |
| `manual` | User-initiated, from the CLI or the GUI Settings panel |
| `pre-migration` | Automatic, taken before applying schema migrations when `auto_pre_migration` is enabled and the database file already existed and was non-empty |
| `pre-restore` | Automatic safety snapshot taken just before a restore swap; deliberately skips rotation so it can never prune the very backup being restored |
| `pre-import` | Automatic, taken before an import run when `auto_pre_import` is enabled |
| `pre-discard` | Automatic, taken before an `import discard` run when `auto_pre_discard` is enabled |

**Retention** is a conservative union, configured in the `[backup]` section (`dir`, `retain_count` default 5, `retain_days` unset, `auto_pre_migration` default true, `auto_pre_import` default true, `auto_pre_discard` default true): a backup is kept if it is among the `retain_count` newest **or** newer than `retain_days`; it is pruned only if it satisfies neither. When both limits are unset, nothing is pruned. On disk, `retain_count = 0` is the sentinel for "unlimited" (an absent key falls back to the default of 5). Routine `pre-import` and `pre-discard` snapshots share this retention pool with manual and `pre-migration` backups, so a series of import or discard runs can crowd out older manual backups under the same union; per-kind retention is tracked as #344.

**Restore** validates the candidate first (copy to a temp directory, open it — which runs migrations — and run a sentinel query), then takes a `pre-restore` safety snapshot, then swaps the candidate in: the CLI closes the pool and swaps in-process; the GUI writes a restore-marker beside the database and relaunches, applying the swap at startup before any connection is opened. The swap itself clears stale `-wal`/`-shm` sidecars left by the replaced database and installs the candidate via a temp copy + atomic rename, so an interrupted restore leaves the live database untouched rather than corrupted.

______________________________________________________________________

## 5. Format Compatibility (`bc-format-*`)

### 5.1 Built-in Formats

| Format | v1 | Later |
| ------------------ | -------------- | ----- |
| Ledger | ✅ read + write | — |
| Beancount | ✅ read + write | — |
| CSV (configurable) | ✅ import | — |
| OFX/QFX | ✅ import | — |
| QIF | — | ✅ |
| CAMT.053 | — | ✅ |
| JSON/YAML (native) | — | ✅ |

Post-v1 built-in formats are delivered as additions to `bc-formats` (not as plugins), since they require no plugin ABI. Community-contributed bank-specific formats are delivered as plugins via `bc-sdk`.

### 5.2 Importer Trait

```rust
pub trait Importer {
    fn name(&self) -> &str;
    fn detect(&self, bytes: &[u8]) -> bool;  // format / profile-aware sniffing
    fn import(&self, config: &ImportConfig) -> Result<Vec<RawTransaction>, ImportError>;
    fn validate(&self, config: &ImportConfig) -> Result<(), ImportError>;
}
```

`validate` checks a config for internal coherence without touching the filesystem, so an incoherent profile can be rejected without running an import. It runs before every `import`, and may also be called on its own. It has **no default body**: an importer with no rules yet returns `Ok(())` explicitly, so that a delegating wrapper which forgets to forward it fails to compile instead of silently accepting everything.

The importer is a **pure parsing concern** — it converts bytes to `RawTransaction` values. Accounts live **on the postings**: each `RawTransaction` carries one or more `RawPosting` legs, and every leg names its own account **path** (e.g. `Assets:Bank:Checking`). Multi-account formats (Ledger, Beancount) name each leg's account directly; row-oriented formats (CSV, OFX) take every leg's account path from the importer's own config blob rather than from the file — one leg per row by default, though a CSV profile may configure further legs (a fee, the other side of a trade) that the same row also feeds. A leg's `amount` is optional — `None` marks an elided residual that balances the transaction. An optional `SourceLocation { display, uri }` on `RawTransaction` lets an importer name where a row came from (a file path and row number, an API response, …) for diagnostics; `display` is free-form and `uri` is an optional machine-addressable form.

Account **path → id** resolution happens later, in `bc-core` at persistence time, centralised in `AccountResolver`: it loads one snapshot of every account — archived included — per import run, then walks a path's segments down the parent/child tree. Matching is exact and case-sensitive, since Beancount capitalises its roots and Ledger permits spaces inside a segment; normalising would invent ambiguity rather than remove it. A path naming no account skips only that leg — never the rest of the row — and is never auto-created; the missing paths are collected into a deduplicated, sorted report so the user can create the accounts and re-run (see §5.3 for how a later run completes what an earlier one skipped).

> **Option C factory pattern** — foundations implemented in Milestone 2, full plugin registry deferred to Milestone 6. `ImporterFactory` (in `bc-core`) holds two fn pointers: `fn(&[u8]) -> bool` for stateless format-level detection and `fn() -> Box<dyn Importer>` for instance creation. `ImporterRegistry` stores a list of factories and provides `detect_format`, `create_for_name`, and `create_for_bytes`. Each format crate exposes a free `importer_factory()` function. The `Importer::detect(&self, ...)` method is retained for profile-aware detection after an instance is configured with a specific account's import profile.

### 5.3 Import Profiles

Import profiles live in `bc-core` and name a reusable importer plus its config:

```rust
struct ImportProfile {
    id: ProfileId,                   // newtype wrapper around TypeId (see ID convention below)
    name: String,                    // e.g. "Bank Savings"
    importer: String,                // e.g. "commbank-au"
    config: ImportConfig,            // column mappings, date formats, target account, etc.
    created_at: Timestamp,
}
```

Account binding is **not** a profile concern: it lives on each `RawPosting`
(see §5.2). A single-account profile's target account is carried inside its
opaque `config` blob and stamped onto the emitted leg.

**ID convention:** All ID types (`ProfileId`, `AccountId`, `TransactionId`, etc.) are newtype wrappers around a typed prefixed ID from the [`mti`](https://crates.io/crates/mti) crate. This produces human-readable, type-safe IDs like `profile_01h455vb4pex5vsknk084sn02q` — the prefix makes the type visible in logs and debug output, and the Rust newtype ensures IDs are never confused with each other at compile time. All ID types are defined in `bc-models`.

Multiple profiles can reference the same importer with different configuration. All CLI/TUI/GUI import operations work on profiles, not raw importers.

> **Deduplication:** Import idempotency is provided by per-**posting** source
> references (`transaction_sources`), not per-transaction. Every persisted leg
> carries a `posting_id` and a `SourceRef` scoped to its owning account,
> fingerprinted on `(date, narration, amount, reference)` with an occurrence
> ordinal — allocated per `(account, fingerprint)` — to disambiguate
> legitimately-identical rows; the `UNIQUE(account_id, fingerprint, occurrence)`
> key makes re-importing the same document a no-op. An elided leg fingerprints
> an *absent* amount (the value and commodity components render empty) rather
> than its resolved residual, because the residual is derived and would change
> if a sibling leg later changed — fingerprinting a computed value would make
> the dedup key itself unstable.
>
> A transaction's legs can therefore arrive across several import runs: one
> pass books the legs whose accounts already exist, and a later pass — after
> the missing accounts are created — attaches the rest to the same
> transaction rather than creating a duplicate. Before attaching, the run
> corroborates the candidate: every posting already on it must be explained by
> a leg of the document transaction being imported, and how it is explained
> turns on whether it carries provenance. A posting an import wrote is
> explained **by its reference** — the leg matching the `(account, fingerprint, occurrence)` the reference recorded. Every component comes from the
> reference rather than the posting, so an edit that corrects an amount or
> recategorises the leg moves the posting but never its reference, and the
> document's remaining legs can still arrive. A posting carrying no provenance
> is one the user wrote, in all likelihood the very leg an earlier pass could
> not resolve; it is explained **by adoption** — an unresolved leg on its
> account holding the same amount — and provenance is then recorded against
> that posting instead of a duplicate being inserted. A candidate with a
> posting explained neither way belongs to some other document: it is left
> alone and reported as a warning rather than risk grafting a leg onto the
> wrong transaction. Matching on references rather than on current amounts is
> also what keeps corroboration independent of the derived residual.
> Per-profile loosened fingerprints and transfer-leg merging remain deferred
> (see #266).
>
> Which of these two explanations applied is recorded on the reference itself,
> as `owns_posting`: true for a posting the import wrote, false for one it
> adopted. The column exists for discard (see below) — undoing a run must
> delete the postings it created but only detach its references from postings
> it merely adopted, and current-state matching alone cannot tell the two
> apart once the run is history.
>
> A source reference outlives the posting it names. Deleting a leg clears the
> reference's `posting_id` rather than deleting the reference, leaving a
> tombstone: a `NULL` `posting_id` records a leg the source document contained
> and the user has since removed. The tombstone still occupies its
> `(account_id, fingerprint, occurrence)` slot, so re-importing the same
> document does **not** recreate a leg the user deliberately deleted, and it
> keeps its original `account_id` even where an edit recategorised the posting
> — the reference describes the source document, not the edited state.
> `SourceService::detach` remains the explicit "forget this provenance"
> action, and does delete the row. The one case where a tombstone does not
> outlive the deletion that created it: discarding the batch that wrote it (see
> below) deletes the tombstone along with every other reference the batch
> owns, freeing its slot — the run is undone, so nothing is left for a
> re-import to guard against.
>
> Each import run is recorded in `import_batches` — the profile (if any), the
> importer, `started_at`, `finished_at`, `discarded_at`, and counts of new
> transactions, attached postings, and the two causes a posting is skipped for
> — an account path naming no existing account, and anything else — held side
> by side rather than as a total plus a subset of it. `finished_at`
> and the counts are set together when the run completes; a run that aborted
> before then has neither, so it is never misread as one that completed and
> did nothing. `import discard <batch-id>` (`bc-cli`; see §8.1) undoes a run:
> every posting it created is deleted along with its references (a tombstone
> included, per above), a posting it only adopted is detached but kept, and
> any transaction left holding no postings is deleted too, taking along
> whatever other batches' references happened to be riding on it. A surviving
> transaction's remaining legs are renumbered, since every other writer treats
> `postings.position` as contiguous from zero. Another batch's reference that
> merely adopted a deleted posting is reported separately from one swept away
> with its transaction: the first is left as a tombstone, keeping its slot,
> and only the second is gone. Discard
> means the run never happened, not that it is reverted — there is no
> undiscard. It takes a `pre-discard` snapshot (`backup.auto_pre_discard`, see
> §4.6) before writing, and records one `ImportBatchDiscarded` event carrying
> the removal counts; restoring that snapshot is the recovery path if a
> discard turns out to be a mistake.

______________________________________________________________________

## 6. Plugin System

### 6.1 Runtime

- **`bc-plugins`**: WASM host runtime using [wasmtime](https://wasmtime.dev/) directly, with the guest interface described in WIT and bound via `wit-bindgen`
- **`bc-sdk`**: Standalone crate published to crates.io. Plugin authors depend on this, compile to `wasm32-wasip2`, and distribute a single `.wasm` file.
- **Plugin discovery**: `~/.config/borrow-checker/plugins/` (configurable). A `plugins.toml` manifest lists enabled plugins and their configuration.

### 6.2 ABI Versioning

The SDK uses a **single integer ABI version**, separate from semver. Only breaking changes increment it; additive changes use capability negotiation (plugins query at runtime whether a host function exists).

**Support window policy:** A new ABI version is announced at release N. The previous version is deprecated and dropped no earlier than release N+2 — giving plugin authors at least one full release cycle to migrate.

| BorrowChecker | Supported ABIs | Notes |
| ------------- | -------------- | ------------------------- |
| 0.x | `[1]` | Initial release |
| 1.x | `[1, 2]` | v1 deprecated, v2 active |
| 2.x | `[2, 3]` | v1 dropped, v2 deprecated |

During the grace period the host loads deprecated-ABI plugins via a compatibility shim and warns the user at startup with a link to the migration guide.

**Before the first public release** this policy is not yet in force. There are no plugins outside this repository, so the WIT world may gain or change exported functions without incrementing `SDK_ABI`; the mitigation is simply that all first-party plugins are rebuilt in the same change. Note the consequence: a stale `.wasm` fails to instantiate and is skipped at load with a generic probe error rather than the ABI-mismatch diagnostic, because the host must instantiate a component before it can call `sdk_abi()`. Once the app is public, every such change requires a real ABI bump and the support window above.

### 6.3 Plugin Phases

**Phase 1 — Importers (Milestone 6, critical)**

Plugins implement the `Importer` trait. Registered by name; referenced in import profiles.

**Phase 2 — Transaction Processors (Milestone 8)**

A general-purpose pipeline that runs after import, before committing events. Each processor receives a `PendingTransaction` plus read-only context (account history, FX rates, user prefs) and returns a modified transaction or a review flag.

```rust
fn process(tx: PendingTransaction, ctx: &TransactionContext) -> ProcessorResult
```

Example processors: merchant normalisation, auto-categorisation, auto-split, FX enrichment, tax flagging, recurring detection, anomaly flagging, account auto-assignment. Processors declare a priority; pipeline order is deterministic and configurable.

**Phase 3 — Report Generators (Milestone 9)**

```rust
fn generate(query: ReportQuery, data: ReportData) -> ReportOutput
// ReportOutput = { title, series: Vec<DataSeries>, chart_hint: ChartType }
```

The host provides data; the plugin aggregates and shapes it. Chart rendering stays in the frontend — plugins return data, not pixels.

**Phase 4 — UI Extensions (Milestone 10, Tauri only)**

Plugins declare named pages. Tauri loads them as panels; plugin icons are auto-registered in the navigation rail. Requires Phase 3 to be meaningful.

______________________________________________________________________

## 7. Budgeting

### 7.1 Model

Default methodology is **zero-based budgeting** (every dollar assigned to a purpose). Users who don't want zero-based budgeting attach no allocation target to their accounts — they become plain category trackers. The data model is identical; it's a workflow preference.

**There is no separate envelope entity.** Budget categories are `Expense`-type accounts in the account tree (see §4.3). The `Budget` entity attaches allocation targets and period rules to an account:

```
Budget {
    id:             BudgetId
    account_id:     AccountId       // required — always anchored to an account
    tag_filter:     Option<TagId>   // postings matching this tag count against this budget;
                                    //   None = all postings to this account
    name:           Option<String>  // e.g. "Weekly repayment", "Person: me"
    target:         Option<Amount>  // None = tracking-only, no allocation target
    period:         BudgetPeriod    // see §7.2
    rollover:       RolloverPolicy  // carry forward / reset / cap at target
    created_at:     Timestamp
    archived_at:    Option<Timestamp>
}
```

**Multiple budgets per account** are allowed and expected. Examples:

- `Liabilities:Mortgage` — one budget for weekly repayments, another tracking accrued interest
- `Expenses:Haircuts` — one budget filtered to `#person:me` ($30/month), one to `#person:wife` ($60/month)
- Any account type may carry budgets; the restriction to `Expense`-type accounts is a workflow convention, not a data model constraint

**Rollup uses the account tree.** Parent accounts aggregate their children's actuals and budget totals upward automatically — no separate grouping entity is needed. `Expenses:Health` rolls up `Expenses:Health:Gym`, `Expenses:Health:Pharmacy`, etc.

**Budget assignment vs. reporting dimensions:**

Two orthogonal mechanisms exist for categorising spending:

- `posting.account_id` → **where** the money was categorised (the expense account IS the category; one account per posting; satisfies double-entry balance)
- `posting.tag_ids` / `account.tag_ids` → **how** to slice for reporting (multi-dimensional; many tags per entity; enables cross-cutting views like "all spending tagged `person:me`" or "all postings tagged `context:holiday`")

Example: a gym posting to `Expenses:Health:Gym`, tagged `person:me`. This counts against the gym account budget, and also appears in any "personal spending" report filtered by `person:me` — without double-counting.

**Posting-to-budget matching:**

A posting matches a `Budget` row when `posting.account_id` is the budget's account **or any descendant** in the account tree, and either `budget.tag_filter` is `None` or the posting carries that tag. The implementation uses a recursive CTE (`WITH RECURSIVE acct_tree`) to resolve all descendant accounts at query time. A budget on `Expenses:Health` therefore matches postings to `Expenses:Health:Gym` as well as directly to `Expenses:Health`. The subtree rule is what makes such a budget useful: `account create` materialises every ancestor of a path (§4.3), so an intermediate account like `Expenses:Health` commonly holds no postings of its own and draws its whole actual from its descendants.

Resolution:

- 0 matches: posting is tracking-only for this account — contributes to the account's actual total but no budget line
- 1 match: unambiguous
- 2+ matches: ambiguous — computed at query time and surfaced by the UI; the user must resolve (split the transaction, remove a conflicting tag, or explicitly nominate one budget)

A uniqueness constraint on `(account_id, tag_filter)` in the `budgets` table prevents two tracking-only budgets (both with `tag_filter = None`) from existing on the same account, which would create permanent unresolvable ambiguity.

Budget anchoring is permanent: a `Budget` cannot be re-anchored to a different account. Account restructuring (e.g., splitting `Expenses:Food` into sub-accounts) requires archiving affected budgets and creating replacements on the new accounts.

Conflict detection is a UI concern. The event log records raw postings; resolution is not enforced at the storage layer.

### 7.2 Budget Periods

| Period | Notes |
| ------------------ | ------------------------------------------------ |
| Weekly | Anchor: day of week |
| Fortnightly | Anchor: specific date, 14-day stride |
| Monthly | Calendar month |
| Quarterly | Jan/Apr/Jul/Oct or custom start month |
| Financial Quarter | FY-aligned quarter; anchor: configured financial year start month |
| Financial Year | Configurable start month/day — set once globally |
| Calendar Year | January 1 |
| Custom | N days / N weeks / N months |

**Fortnightly anchor:** set once globally (e.g. "my pay cycle starts 3 March 2026"). Every fortnight is derived as a 14-day stride from this anchor — no ambiguity.

**Financial year start** is a global preference, prompted during onboarding, with locale-based defaults:

| Locale | Default FY start |
| ------------------------ | ---------------- |
| 🇦🇺 Australia / 🇳🇿 NZ | 1 July |
| 🇬🇧 UK | 6 April |
| 🇺🇸 US (federal) | 1 October |
| 🇺🇸 US (personal) / Europe | 1 January |

**Mixed-period display:** all budgets normalise to a user-chosen display period (monthly by default). An annual `Car Registration` budget that accumulates monthly is displayed correctly alongside monthly grocery budgets.

______________________________________________________________________

## 8. Frontends

### 8.1 CLI (`bc-cli`)

Thin binary over `bc-core`. Commands:

```
borrow-checker account [list|create|archive]
borrow-checker transaction [list|add|amend|void]
borrow-checker asset [record-valuation|depreciate|set-loan-terms|amortization|book-value]
borrow-checker profile [create|list|show|edit|remove]
borrow-checker import run --profile <name> [--dry-run]
borrow-checker import list
borrow-checker import discard <batch-id>
borrow-checker export --format <ledger|beancount> --output <file>
borrow-checker report [net-worth|summary|categories]
borrow-checker budget [status|allocate|list]
borrow-checker plugin [install|list|remove]
borrow-checker completions <bash|elvish|fish|powershell|zsh>
```

Importers source their own files from the profile config (see §5.2), so `import run` takes no file argument and no account argument: each `RawPosting` names its own account path, resolved to an id in `bc-core` at persistence time (see §5.2, §5.3). `import` is a subcommand group: `run` executes a profile, `list` shows every run newest first with its outcome, and `discard <batch-id>` undoes one (see §5.3) — reported the same way `run` is, with `--json` covering all three.

`run --dry-run` resolves the profile and reports what it would do without writing: the account paths that would not resolve, the commodity codes that are not registered, the rows that would be skipped and why, the tags that would be created, and the per-account totals that would post. It is the same run with its writes diverted, not a second implementation, so it cannot drift from what `run` does. The report leads with what is broken rather than what would succeed, because it exists for profile tuning; `--json` covers it as it does the other three, minus the `batch_id` key, since a dry run opens no batch and so leaves nothing to `list` or `discard`.

Import profiles are created and edited from the CLI. `profile create` takes the
importer's opaque config as a TOML or JSON file (`--config <FILE>`, or `-` for
stdin); TOML is converted to JSON inside `bc-cli`, so `bc-core` and the plugin
ABI continue to see a single JSON blob. Profile names are unique and are the
identifier every surface uses. An unrecognised `--importer` is a warning, not
an error: the plugin may not be installed yet, and `import` errors at the point
of use.

`csv` and `json` native export are post-v1 additions (see §5.1 format compatibility table) and will extend the `--format` option when implemented.

All commands support `--json` for structured output. Shell completions are generated on demand via `borrow-checker completions <bash|elvish|fish|powershell|zsh>`.

Backup and restore (see §4.6) are exposed as CLI commands over the same `bc-core` service the GUI uses.

### 8.2 Tauri GUI (`bc-app`)

Layout: **icon rail + context-sensitive content**.

- Icon rail (left): Dashboard · Accounts · Budget · Reports · Plugins (plugin icons auto-append)
- **Dashboard** is the home screen: net worth, spend this month, budget remaining, recent transactions, budget health bars, quick-import button
- Accounts view: account tree (left panel) + transaction list + detail (right panel)
- Power users navigate directly via the account tree; new users land on the dashboard
- Settings → Backup panel: edit backup settings, trigger a manual backup, and restore from an existing snapshot (see §4.6)

______________________________________________________________________

## 9. Milestone Summary

> This table is a high-level design reference. Live tracking of outstanding work
> happens in [GitHub issues](https://github.com/JP-Ellis/borrow-checker/issues)
> (epics with sub-issues), not in a roadmap document.

| Milestone | Description | Depends on |
| --------- | ------------------------------------------------ | ---------- |
| 0 | Project foundation (workspace, CI, docs) | — |
| 1 | Core engine (`bc-core`, SQLite, event log) | 0 |
| 2 | Format compatibility (`bc-format-*` crates) | 1 |
| 3 | CLI (`bc-cli`) | 1, 2 |
| 5 | Budgeting (account-anchored budgets, tag-filtered sub-budgets, allocation, all periods) | 1 |
| 5A | Illiquid asset tracking (valuations, depreciation, loan terms) | 1, 5 |
| 6 | Plugin Phase 1: Importers | 2, 3 |
| 7 | Tauri GUI (`bc-app`) | 1, 2, 5 |
| 8 | Plugin Phase 2: Transaction Processors | 6 |
| 9 | Plugin Phase 3: Report Generators | 8 |
| 10 | Plugin Phase 4: UI Extensions | 7, 9 |
| 11 | Sync & multi-device (event replication, Android) | 10 |

______________________________________________________________________

## 10. Key Technical Decisions

| Decision | Choice | Rationale |
| ---------------------- | -------------------------------------------- | ---------------------------------------------------------------- |
| Storage | SQLite via `sqlx` | Portable, zero-server, fast for single-user workloads |
| Event log | Append-only SQLite table | Audit trail, undo/redo, future sync without full CQRS overhead |
| WASM runtime | wasmtime + WIT / `wit-bindgen` | Component-model interfaces; WASI preopens let importers read their own files |
| GUI framework | Tauri + Leptos (WASM) | Rust-native, small binary, real DOM for accessibility and charting |
| Filter exactness | Coarse in SQL, exact in Rust | SQL magnitude comparison cannot respect commodity; the superset is narrowed in `AmountQuery::matches` |
| Transaction admission | Permissive; balance is a derived flag | One-sided imports are the normal case and must persist to be categorised |
| Plugin ABI versioning | Integer ABI + N+2 grace period | Simple, explicit, protects the community ecosystem |
| Budget default | Zero-based; expense accounts are categories | Most intentional model; degrades gracefully to category tracking; round-trips cleanly with ledger/beancount |
| Importer/account split | Importer = parser, Profile = account binding | Clean separation; same parser serves multiple accounts |
| ID types | `mti` newtype wrappers (e.g. `profile_01h…`) | Type-safe, log-readable, no ID confusion across domain types |

**Rejected: Slint as the UI framework.** Evaluated mid-2026 with a working
proof-of-concept reproducing the accounts page. The DSL ergonomics and native
startup and binary size were genuine wins, but its web target renders to a
canvas rather than the DOM (weak accessibility, no browser devtools or CSS),
it has no mature charting library for the sparkline and budget visuals,
it offers no webview-plugin injection point equivalent to the `bc-ipc` seam,
and it is single-vendor licensed. Revisit only if those change; the near-term
alternative is better Leptos hot-reload, not a framework swap.
