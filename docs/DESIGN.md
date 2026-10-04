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
- Multiple surfaces: CLI for scripting and automation, Tauri GUI for interactive use, and a self-hosted web server for the same GUI in a browser

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
- CLI, Tauri GUI and web server as the primary surfaces
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
│   ├── bc-expr/                # amount expression evaluator (Beancount grammar)
│   ├── bc-plugins/             # WASM host runtime + plugin ABI bridge
│   ├── bc-sdk/                 # plugin author SDK (published to crates.io separately)
│   ├── bc-sdk-macros/          # proc-macro support for bc-sdk
│   ├── bc-seed/                # seeded E2E fixture and synthetic benchmark ledgers
│   ├── bc-cli/                 # CLI binary
│   ├── bc-ipc/                 # serde wire contract between the GUI hosts and bc-ui
│   ├── bc-service/             # GUI command bodies, shared by bc-app and bc-server
│   ├── bc-ui/                  # Leptos frontend (WASM)
│   ├── bc-app/                 # Tauri GUI
│   └── bc-server/              # web server: bc-ui and the RPC commands over HTTP
└── plugins/                    # first-party format plugins: beancount, csv, ledger, ofx
```

**Design philosophy — keep crates small and focused.** Each crate should have one clear purpose, a well-defined public API, and minimal dependencies. As the project grows it is expected and encouraged to introduce new crates or split existing ones. Utility crates will likely emerge as needed — e.g. `bc-config` (configuration management), `bc-otel` (OpenTelemetry tracing/metrics). Prefer creating a new crate over stuffing shared functionality into an existing one.

**Dependency relationships:**

- `bc-models` has no internal dependencies — it is the shared vocabulary for the whole workspace
- `bc-core` depends on `bc-models`; `bc-cli` and `bc-service` depend on `bc-core`
- `bc-app` and `bc-server` are hosts over `bc-service`; each forwards a GUI command to `bc_service::dispatch`
- The first-party format plugins under `plugins/` depend on `bc-sdk`, and the Beancount plugin on `bc-expr`
- `bc-plugins` depends on `bc-core` (bridges WASM into the engine)
- `bc-sdk` is standalone — plugin authors only need it, not the full workspace
- `bc-ipc` is the serde wire contract between the hosts (`bc-app`, `bc-server`) and `bc-ui` (WASM). Its `http` feature selects the transport: Tauri `invoke` without it, `fetch` to `bc-server` with it. It stays a thin contract with no service dependencies; all conversion arrows point *toward* it. It gains an optional dependency on `bc-models` behind a `models` feature (native-only) so DTO↔domain `From`/`TryFrom` impls for basic scalar/enum/`Commodity` values can live in `bc-ipc` without leaking `bc-models` into the WASM build. Domain-walking conversions (account paths, tag resolution, `Transaction`/`AccountNode` assembly) live in `bc-core` as extension traits behind its opt-in `ipc` feature, not in `bc-ipc`. `bc-config`/`bc-plugins` similarly host their own DTO conversions behind an `ipc` feature.

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
AccountCreated / AccountUpdated / AccountArchived / AccountClosed / AccountReopened / AccountOpenedOnChanged / AccountCommoditiesChanged
TransactionCreated / TransactionVoided / TransactionReversed
TransactionDateChanged / TransactionDescriptionChanged
TransactionTagsChanged / TransactionMetadataChanged
TransactionReconciled
PostingRecategorised / PostingAmountChanged / PostingMetadataChanged / PostingTagsChanged / PostingSpreadChanged / PostingAnnotationChanged
PostingAdded / PostingRemoved
MetadataKeyRegistered / MetadataKeyRetyped / MetadataKeyRenamed / MetadataKeyDeleted
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

**Double-entry accounting** is the model: a transaction is a set of postings whose
weights should sum to zero per commodity, consistent with ledger/beancount semantics.
Balance is *derived and advisory*, not an admission requirement — see §4.4.

### 4.3 Account Model

Accounts are classified by `AccountType` (Asset, Liability, Equity, Income, Expense) — the canonical double-entry roots. The five variants are stable and unlikely to change; `#[non_exhaustive]` covers rare future additions.

**Hierarchy via `parent_id`:**

Accounts form an arbitrary-depth tree through an optional `parent_id: Option<AccountId>`. A root account (`parent_id = None`) is the authority for its `AccountType`; child accounts inherit their root's type when a path is materialised by `account create`, which derives the type from the root segment and applies it to every segment it creates. `Service::create` checks the requested type against the parent's and rejects a contradiction, so both entry points agree. The hierarchy supports:

- Institution grouping: `Assets > Bank > Savings, Checking`
- Virtual sub-accounts: `Assets > Bank > Offset > Mine, Partner, Shared`
- Rollups: summing a subtree gives the parent balance; virtual sub-accounts of a joint account should always sum to the real account's bank-statement balance
- Beancount/ledger export: the colon-separated path is derived by walking the ancestor chain

No two sibling accounts share a name (`idx_accounts_sibling_unique`, a `UNIQUE` index on `(parent_id, name)` with the parent folded through `COALESCE` so root accounts — whose `parent_id` is `NULL` — are compared too). This is what makes a colon-separated path resolve to at most one account during import (see §5.2). An archived account still owns its name: renaming or reusing it requires archiving or renaming that account first.

An account carries two optional business dates, `opened_on` and `closed_on`. Neither is `archived_at`, which controls visibility in active lists and says nothing about when an account existed: an account closed years ago still belongs in reports covering the years it was open. A transaction dated outside a declared life warns and is written — a date outside an account's bounds is more often a wrong declaration than a wrong transaction, and refusing the posting would lose data to fix metadata. A closed account may not have an open descendant, and an archived account may not have an unarchived one — the two guards read the column they are named for, so archiving a subtree says nothing about whether it is closed. `close` and `archive` reject by default, naming the descendants that block them, and cascade on request.

**Account kind** governs how a leaf account's balance is maintained:

| Kind | Description |
| ---------------- | ----------- |
| `DepositAccount` | Reconciles against a bank/card/brokerage statement. May have an import profile. Examples: checking, savings, credit card, investment portfolio. |
| `ManualAsset` | Manually-maintained real asset with no bank statement. Balance driven by valuation events. Examples: real property, vehicle, private equity stake. |
| `Receivable` | Money owed to you by a third party. Tracked via ordinary transactions (disbursement + repayments). May carry optional loan terms for amortization assistance. Examples: personal loan to a friend, loan to a trust. |
| `VirtualAllocation` | No independent existence. Subdivides a parent account's balance. Examples: earmarked sub-accounts within an offset account. |
| `Group` | Organisational node that holds no postings of its own. Created implicitly as a path ancestor when `account create` materialises a nested path, or explicitly via `--kind group`. Examples: `Assets`, `Assets:BankA`, `Expenses:Food`. |

An import profile carries a name, an importer identifier and a config blob; it holds no account reference, and nothing restricts which account kind a profile is applied to. `DepositAccount` is the kind the import workflow expects, by convention rather than by a check.

**Cross-cutting labels via an entity-based tag model:**

The primary hierarchy can only express one grouping at a time. Cross-cutting concerns — ownership (mine / partner / shared), institution grouping across types, liquidity flags — are expressed as tag references on accounts.

Tags are first-class entities stored in a `tags` table with `id`, `name`, `parent_id` (self-referential for tag hierarchy), and `description`. `Account` holds `tag_ids: Vec<TagId>` — stable opaque references that survive renames. Human-readable paths are derived on demand via `TagForest::path_of(id) -> TagPath`: a `TagPath` is an ordered sequence of non-empty segments (`["institution", "commbank"]`) that serialises as a colon-joined string (`institution:commbank`).

Across IPC every tag is a `TagInfo { id, path }`: reads fill `path` from the forest, and writes read only `id`, so a rename never invalidates what a client holds.

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
`description`, `reconciliation`, `Posting::amount`, `price` and `cost` stay structural
because they carry invariants or drive computation. Beancount draws the same
line — `cost` and `price` are syntax, metadata is the escape hatch.

**Reconciliation is the only status axis.** `enum Reconciliation { Unreconciled, Flagged, Reconciled }`.
An earlier `Pending / Cleared / Voided` conflated three separate concerns:
finalisation is now *structural* (derived balance), voiding is a reversal link,
and reconciliation is what remains. `Flagged` is an attention marker.

**Balance is derived, never stored.** `balanced()` sums posting weights to
zero per commodity after resolving an elided leg. It is false when there are no
concrete legs, when the residual is non-zero for any commodity, when two or
more legs are elided, or when a commodity's running total overflows
`Decimal`'s range.

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

**Stale edits are refused.** `edit_transaction` carries the base the editor
loaded; if the stored transaction has moved on, the save fails with a
conflict and the draft stays on screen. It is the one blocking check outside
unrepresentable states.

**A leg is weighed, not just summed.** `Posting::price` (`@`/`@@`) and
`Posting::cost` (`{}`/`{{}}`) are each a `Quote`, kept in the form the source
stated — per unit or total — because neither converts to the other exactly.
For balancing and the residual a leg contributes its *weight*
(`Posting::weight`): at cost if a cost is set, else at price if a price is
set, else the amount itself, exactly Beancount's rule. So
`4.00 USD @@ 6.37 AUD` against `-6.37 AUD` balances, and `-2 ETH @ 300 AUD`
funds an elided gains leg in AUD. Account balances still move by the amount:
the ETH account holds ETH. A quote in the leg's own commodity is weighed as
written and warned about (`Warning::QuoteInOwnCommodity`). A cost is stored on
the leg and weighed; nothing reads it back as inventory — lot booking is its
own feature.

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
subtree, tags, description text, amount magnitude, reconciliation, and balance
status. Dimensions combine with AND; values *within* the account and tag
dimensions combine with OR. Every view recomputes against it.

Free text matches a transaction's description alone. Payee lives in metadata,
which the text dimension does not reach; searching metadata keys is its own
syntax, tracked in #429.

**The query never prunes.** `Service::search` returns whole transactions
annotated with which legs matched (`MatchedTransaction { transaction, matched_postings }`),
so a consumer decides its own presentation rather than receiving a
pre-truncated, possibly unbalanced transaction. Posting-scoped dimensions
(account, amount, posting tags) distinguish legs; transaction-scoped ones (date,
text, reconciliation, balance status, transaction tags) match the whole
transaction.

**SQL is a candidate filter; Rust is the source of truth.** The generated SQL
narrows by coarse amount magnitude, producing a deliberate superset; exact
matching happens in Rust via `AmountQuery::matches`. This is not an
optimisation detail — it is what preserves commodity integrity. Comparing
magnitudes in SQL would let `over:USD50` match a BTC amount, so amounts are
never finally compared in SQL anywhere, including the budget actuals path.

Balance status has no SQL form at all, since amounts are TEXT. Any query
using it hydrates every candidate and asks `Transaction::balanced()`, the
same verdict the IPC DTO carries for the row's unbalanced pill.

Consumers interpret the shared filter through their own lens:

| View | Interpretation |
| ------------- | -------------------------------------------------------------------------- |
| Register | Intersection: the sidebar account is the scope, other dimensions refine it. A filter date bound overrides the period window and disables the period navigator. Non-matching legs are dimmed, never dropped |
| Balances | Transaction-membership: the filter selects a set of transactions; the figure sums *the viewed account's own legs* across them. A muted unfiltered figure is shown alongside for context |
| Sparklines | Same membership rule, bucketed. Filter dates re-anchor the span and drive bucket granularity |
| Budgets | Actuals-only lens: the filter narrows what counts toward actuals; targets never change and no budget is pruned. **The date dimension is ignored** — the period navigator is the sole driver, since a filter range does not align with budget period grids. **Balance status is ignored too**: actuals assume double entry, which an unbalanced transaction violates |

### 4.6 Backup & Restore (`bc-core`)

Snapshots are taken via SQLite `VACUUM INTO` to a temp file, then atomically renamed into place — a backup is a standalone file with no `-wal`/`-shm` sidecars.

**Pools.** Each database carries a ledger ID, a `ledger_…` TypeID stored under the `ledger-id` key in `meta` and minted on first open. Managed backups live in `{dir}/{ledger-id}/` as `{stamp}.{kind}.sqlite`; listing, rotation and restore candidates come only from the open database's pool, so ledgers sharing a backup directory never see or prune each other's backups. A moved or renamed file keeps its ID and its pool. A restore keeps the open database's ID whatever the candidate carries, so restoring any backup, legacy or foreign, keeps the database in this ledger's pool. Files of the form `{stamp}.{kind}.sqlite` directly in `{dir}` are unattributed: nothing lists, rotates or deletes them, and moving one into a ledger's pool directory adopts it.

**Kinds** (encoded in the filename suffix):

| Kind | Trigger |
| --------------- | -------------------------------------------------------------------------------------- |
| `manual` | User-initiated, from the CLI or the GUI Settings panel |
| `pre-migration` | Automatic, taken before applying schema migrations when `auto-pre-migration` is enabled and the database file already existed and was non-empty |
| `pre-restore` | Automatic safety snapshot taken just before a restore swap; the snapshot step skips rotation so it can never prune a `pre-restore` backup being restored, and the next ordinary backup reconciles the count |
| `pre-import` | Automatic, taken before a `sync` sweep's first write — once per sweep, not once per profile, and not at all when nothing parses — when `auto-pre-import` is enabled |
| `pre-discard` | Automatic, taken before an `import discard` run when `auto-pre-discard` is enabled |

**Retention** is configured in the `[backup]` section (`dir`, `retain-count` default 5, `retain-days` unset, `auto-pre-migration` default true, `auto-pre-import` default true, `auto-pre-discard` default true). Each automatic kind (`pre-migration`, `pre-import`, `pre-discard`, `pre-restore`) is rotated against its own backups under a conservative union: a backup is kept if it is among the `retain-count` newest of its kind **or** newer than `retain-days`; it is pruned only if it satisfies neither. A burst of imports therefore cannot evict a `pre-migration` snapshot. `manual` backups are never pruned; only an explicit delete removes one. When both limits are unset, nothing is pruned. On disk, `retain-count = 0` is the sentinel for "unlimited" (an absent key falls back to the default of 5).

**Delete and rekey.** `backup delete <file-name>` (and the GUI's per-row delete) removes one backup from the open ledger's pool; it accepts only a bare `{stamp}.{kind}.sqlite` name, so nothing outside the pool is reachable. `backup list` shows the pool. A hand-made copy of a database file carries its source's ledger ID and shares its pool; run `backup rekey` on the copy to give it a fresh ID. Rekey refuses while another process holds the database, and moves no files.

**Restore** validates the candidate first (copy to a temp directory, open it — which runs migrations — and run a sentinel query), then takes a `pre-restore` safety snapshot, then swaps the candidate in: the CLI closes the pool and swaps in-process; the GUI writes a restore-marker beside the database and relaunches, applying the swap at startup before any connection is opened. The swap copies the candidate to a temp file, stamps the open database's ledger ID into it (migrating a candidate that has no `meta` table), clears stale `-wal`/`-shm` sidecars left by the replaced database and installs the copy by atomic rename, so an interrupted restore leaves the live database untouched rather than corrupted.

### 4.7 Configuration (`bc-config`)

**Configuration** layers built-in defaults, `~/.config/borrow-checker/config.toml`,
`./borrow-checker.toml` and `BC_*` environment variables, in rising priority.
Keys are kebab-case (`display-commodity`, `[db] path`, `[backup] retain-count`);
snake_case is accepted, and one file spelling a key both ways is an error. An
environment variable puts `__` between a table and its key: `BC_DB__PATH`,
`BC_BACKUP__RETAIN_COUNT`; the single-underscore `BC_BACKUP_DIR` is an error
naming its replacement. A relative path in a file resolves against the
directory of that file's canonical path, so a symlinked user config anchors
paths beside its target; a relative path from the environment or `--db-path`
resolves against the working directory. A config that fails to load stops the
app and the CLI instead of falling back to defaults.

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
    fn import(&self, config: &ImportConfig) -> Result<Vec<Directive>, ImportError>;
    fn validate(&self, config: &ImportConfig) -> Result<(), ImportError>;
}
```

`validate` checks a config for internal coherence without touching the filesystem, so an incoherent profile can be rejected without running an import. It runs before every `import`, and may also be called on its own. It has **no default body**: an importer with no rules yet returns `Ok(())` explicitly, so that a delegating wrapper which forgets to forward it fails to compile instead of silently accepting everything.

The importer is a **pure parsing concern** — it converts bytes to an ordered list of `Directive` values: a transaction (`RawTransaction`), an `open` or a `close`, in source order, each with its own optional `SourceLocation`. Accounts live **on the postings**: each `RawTransaction` carries one or more `RawPosting` legs, and every leg names its own account **path** (e.g. `Assets:Bank:Checking`). Multi-account formats (Ledger, Beancount) name each leg's account directly; row-oriented formats (CSV, OFX) take every leg's account path from the importer's own config blob rather than from the file — one leg per row by default, though a CSV profile may configure further legs (a fee, the other side of a trade) that the same row also feeds. A leg's `amount` is optional — `None` marks an elided residual that balances the transaction. The optional `SourceLocation { display, uri }` each directive carries lets an importer name where a row or declaration came from (a file path and row number, an API response, …) for diagnostics; `display` is free-form and `uri` is an optional machine-addressable form.

Account **path → id** resolution happens later, in `bc-core` at persistence time, centralised in `AccountResolver`: it loads one snapshot of every account — archived included — per import run, then walks a path's segments down the parent/child tree. Matching is exact and case-sensitive, since Beancount capitalises its roots and Ledger permits spaces inside a segment; normalising would invent ambiguity rather than remove it. A path naming no account skips only that leg — never the rest of the row — and is never auto-created; the missing paths are collected into a deduplicated, sorted report so the user can create the accounts and re-run (see §5.3 for how a later run completes what an earlier one skipped).

**Declarations.** An `open` (date, account path, commodity codes) or a `close` (date, account path) states a fact about an account. `run_with` applies the declarations before it resolves tags or posting legs, so a leg into an account opened in the same file resolves in the same run, and the stored `opened_on` and `closed_on` are in place when the posting checks run. An `open` naming a missing account creates it, with its ancestors as groups. On an existing account it fills each empty field (`opened_on`, the commodity list) and leaves a stored value alone: a differing value raises `Warning::DeclarationConflict` naming the field, and a stored list that holds the same commodities in another order is no conflict. An `open` with no commodities says nothing about the stored list. A `close` of a missing account is an unresolved path and never creates one. A `close` on an open account closes it with `Cascade::Reject`; a refusal from the account service (an open descendant, a date before `opened_on`) becomes `Warning::DeclarationNotApplied` and the account stays open. A commodity code that does not resolve to exactly one commodity joins the unresolved commodities and the rest of the `open` still applies; declarations never create commodities. The declarations apply in date order, since a Beancount ledger is date-ordered whatever its layout. Within one date, opens apply before closes, and closes apply deepest path first, so a parent closing on its child's date is not blocked by that child. Ties keep source order. A second declaration for one path in a run is a diagnostic, and the first in that order applies. A dry run applies the same rules with its writes diverted, and reports the accounts it would create. Re-running an import writes nothing and warns about nothing, since every field compares equal.

A commodity code crossing the ABI is **final**. An importer applies its own aliases (Beancount's and CSV's `commodity_aliases`, resolved at the directive's date) before it emits a code, and the host resolves the code as given.

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
> The key identifies the statement *row*, not its content. It takes only what
> distinguishes one row from another on a statement — date, narration, amount,
> reference — and leaves out everything a document may also state about the
> leg: price, cost basis, metadata, tags. A source is treated as immutable
> institution data that later exports repeat; corrections belong in the app,
> where an edited posting keeps its reference and is recognised on the next
> re-import. A source whose content changed between imports is therefore not
> an amendment: a row whose identity is unchanged dedups and keeps the stored
> posting as the user left it, and a row whose identity changed is a new row.
> Loosening the key per profile (#272) fits this reading; tightening it would
> only turn recognised rows into duplicates.
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
> and only the second is gone. Every tag the run created (recorded in
> `import_batch_tags`, ancestors included) is deleted unless something else
> has since named it — a membership added by hand or by a later run, a budget
> filter, or a child tag that stays — in which case it is kept and counted
> separately. Declarations reverse the same way (recorded in
> `import_batch_accounts`): an account the run created, ancestors included, is
> deleted deepest first unless something still names it — a posting, a child
> account, a tag, a valuation, a budget, a source or a metadata value — or it
> has been edited since the run, in which case it is kept and counted
> separately. An edit is an account event beyond the count the batch recorded
> after its own last write to that account; a field the run filled on an
> existing account is cleared only while the stored value still equals the
> filled one, so a later edit survives, and a filled `closed_on` stays when the
> account's parent is closed. Discard
> means the run never happened, not that it is reverted — there is no
> undiscard. It is refused outright when a later, undiscarded batch owns a
> live leg on a transaction this batch owns a live leg on: removing this
> batch's leg would leave the later one describing money from nowhere, so the
> error names the later batches, newest first, as the order to discard them
> in. A refused discard writes nothing and takes no snapshot. Otherwise it
> takes a `pre-discard` snapshot (`backup.auto-pre-discard`, see
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

**Before the first public release** this policy is not yet in force. ABI 0 is pre-public and may change; ABI 1 will be the first stable ABI, and the table above begins there. There are no plugins outside this repository, so the WIT world may gain or change exported functions without incrementing `SDK_ABI`; the mitigation is simply that all first-party plugins are rebuilt in the same change. Note the consequence: a stale `.wasm` fails to instantiate and is skipped at load with a generic probe error rather than the ABI-mismatch diagnostic, because the host must instantiate a component before it can call `sdk_abi()`. Once the app is public, every such change requires a real ABI bump and the support window above.

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

**There is no separate envelope entity.** Budget categories are `Expense`-type accounts in the account tree (see §4.3). A `Budget` is a permanent anchor on an account; everything else about it lives in time-ordered revisions, each governing from its `effective_from` date until the next one begins:

```
Budget {
    id:             BudgetId
    account_id:     AccountId       // required — always anchored to an account
    created_at:     Timestamp
    archived_at:    Option<Timestamp>
}

BudgetRevision {
    id:             BudgetRevisionId
    budget_id:      BudgetId
    effective_from: Date            // unique per budget
    name:           Option<String>  // e.g. "Weekly repayment", "Person: me"
    target:         Option<Amount>  // None = tracking-only, no allocation target
    target_expr:    Option<String>  // source expression, e.g. "(30 / 4)"; None = literal
    period:         BudgetPeriod    // see §7.2
    rollover:       RolloverPolicy  // carry forward / reset / cap at target
    intent:         BudgetIntent    // Limit / Goal / Estimate; see "Intent and verdict"
    tag_filter:     Option<TagId>   // postings matching this tag count against this budget;
                                    //   None = all postings to this account
    created_at:     Timestamp
}
```

When a revision has a `target_expr`, its `target` is the expression's value, re-evaluated on every write. A revision whose target sign differs from an adjacent revision's saves with a warning.

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

A budget counts every posting in its scope inclusively, the way Fava does: a parent budget's total already includes everything its sub-budgets claim. Display needs a second answer — which single row a posting belongs to — and that is a separate partition, not a second matching pass.

**Partition (ownership for display).** Each budget's scope is its account chain (type root down to its own account) plus its tag chain (`None` when unfiltered, else the filter tag's ancestors down to itself). Scope `A` contains scope `B` when `B`'s account chain starts with `A`'s and, whenever `A` filters on a tag, `B`'s tag chain starts with it too. A posting's owner is the most specific budget among every scope that matches it — the one no other matching scope strictly contains. When a posting's minimal matches are two scopes that neither contains, ownership is shared between them; this happens only when the scopes overlap without nesting (for example `Food #household` and `Groceries`, unfiltered, both containing `Groceries #household` but neither containing the other), and it always produces a double-counted row (see below). Where two minimal, incomparable scopes tie on containment, the row on the deeper account is preferred, then the one with the deeper tag chain — this tie-break only ever fires between overlapping scopes, so it never hides a double count.

**Rollup follows the account tree**, envelopes inclusive: a budget's own row sums every posting anywhere in its scope, matched or owned by a sub-budget or not, and an account row without its own budget sums its children. Each row also breaks its total into segments for the progress bar: `claimed` (spend a sub-budget or the budget's own scope has already accounted for), `unallocated` (spend an envelope owns outright; a budget without sub-budgets counts all its spend as claimed), and `unbudgeted` (spend beneath it that matches no budget at all).

**`↳ unallocated`.** Every budget that has sub-budgets (an *envelope*) gains a leftover row for the postings it owns itself — those that match no more specific budget. Its target, when the envelope has one, is the envelope's target minus its sub-budgets' targets in the same commodity (`None`, and the row shown mixed, when a sub-budget targets a different commodity). An envelope is **over-allocated** when that unallocated target is non-zero and its sign is opposite to the envelope's own target's sign — magnitude over-allocation read in the envelope's own orientation, since target sign is otherwise just orientation (a drawdown envelope with more than 100% of its budget promised out still reads over-allocated, not under). Once its window has started, an over-allocated envelope's `↳ unallocated` row is red whatever its spend, and counts red in the header summary: a ratio against a target of the wrong sign would read green. In the envelope's transaction list its own postings carry the bucket `↳ unallocated` and every other posting names the sub-budget that owns it; an account row above names the envelope instead, since several envelopes can share one account row. A sub-budget nests under its innermost envelope, so when a deeper unfiltered budget exists (`Groceries`), a filtered envelope over it (`Food #household`) nests its sub-budget (`Groceries #household`) under the deeper one and shows no `↳ unallocated` row for it.

**`↳ unbudgeted`.** Only account rows under the `Income` and `Expense` type roots gain a row for postings that match no budget at all — balance-sheet accounts (`Asset`, `Liability`, `Equity`) never do, since an unbudgeted balance there is just an untracked balance, not overspend. The row is omitted when its valued total is zero and every posting behind it was valuable; it still appears at zero when it holds postings with no known value (an unvaluable amount would otherwise vanish silently). An unbudgeted posting lands under the nearest account row at or above its account, not under a budget row: a budget row sums only its own postings, so a posting on a budgeted account dated before that budget's first revision goes to the account row above. Only when no account row survives above it (the type root itself merged with a budget) does it fall under the nearest budget row.

**Row labels** are relative to the parent row, not absolute: a budget row shows its revision name when it has one, else its account path with everything the parent row's account already names stripped off, followed by `#` and its tag path with everything the parent's tag filter already names stripped off, when both parts are present (e.g. `Groceries #household` under `Food #household` reads `Groceries`); either half is dropped when it adds nothing, and a tag-only label (the account part empty) is the bare tag path with no `#`, since the UI renders it as a tag chip instead (e.g. `#person:a` under an unfiltered `Haircuts` reads `person:a`). An account row without its own budget shows its account's leaf name. Leftover rows always show `↳ unallocated` or `↳ unbudgeted`, never a relative label.

**Double-counted, flagged not resolved.** A posting is double-counted when two of the budget rows it matches overlap without either nesting the other — the same condition that produces a shared owner above. This is evaluated from each posting's *full* match set, not just its owner: two matches can each have a more specific match elsewhere in the tree and still overlap with each other. The row flagged is the lowest common ancestor of the two overlapping rows — the row where their totals rejoin and a naive sum would double it. The tree does not resolve the overlap; it flags the row, and the user resolves it by splitting the transaction, removing a conflicting tag, or narrowing one of the budgets.

Budget anchoring is permanent: a `Budget` cannot be re-anchored to a different account. Account restructuring (e.g., splitting `Expenses:Food` into sub-accounts) requires archiving affected budgets and creating replacements on the new accounts.

**Intent and verdict.** Every budget revision carries an intent — `Limit` (stay within the target: spending, a drawdown), `Goal` (reach at least the target: savings, expected income) or `Estimate` (land near the target: interest accrual, a known bill) — defaulting to `Limit` on an `Expense`-type account and `Goal` everywhere else. A row's target is paced across its window: the reference an in-progress window judges against is `target × elapsed_days ÷ window_days` (elapsed counts today, so an open window's reference is never zero); a closed window's reference is the full target, and a future window has no verdict at all. The ratio `actual ÷ reference` is then classified by intent into three bands: `Limit` is red above 100%, warn from 85% to 100% inclusive, green below 85%; `Goal` is red below 85%, warn from 85% up to (not including) 100%, green at 100% or above; `Estimate` is green from 95% to 105% inclusive, warn from 85% up to 95% or above 105% up to 115%, red outside that. A row's own verdict never propagates past its own row; the tree separately tracks the worst verdict among every row beneath it, for the dot a collapsed row shows.

Conflict detection beyond double-counting is a UI concern. The event log records raw postings; resolution is not enforced at the storage layer.

### 7.2 Budget Periods

| Period | Notes |
| ------------------ | ------------------------------------------------ |
| Daily | Calendar day |
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
borrow-checker account [list|create|archive|close|reopen|set-opened-on|balance]
borrow-checker transaction [list|add|edit|reverse]
borrow-checker asset [record-valuation|depreciate|set-loan-terms|amortization|book-value]
borrow-checker profile [create|list|show|edit|remove]
borrow-checker import run --profile <name> [--dry-run]
borrow-checker import list
borrow-checker import discard <batch-id>
borrow-checker sync --profile <name> | --all [--dry-run]
borrow-checker export --format <ledger|beancount> --output <file>
borrow-checker report [net-worth|summary|categories]
borrow-checker budget [list|create|archive|status|update]
borrow-checker plugin [install|list|remove]
borrow-checker completions <bash|elvish|fish|powershell|zsh>
```

Importers source their own files from the profile config (see §5.2), so `import run` takes no file argument and no account argument: each `RawPosting` names its own account path, resolved to an id in `bc-core` at persistence time (see §5.2, §5.3). `import` is a subcommand group: `run` executes a profile, `list` shows every run newest first with its outcome, and `discard <batch-id>` undoes one (see §5.3) — reported the same way `run` is, with `--json` covering all three.

`run --dry-run` resolves the profile and reports what it would do without writing: the account paths that would not resolve, the commodity codes that are not registered, the rows that would be skipped and why, the tags and accounts that would be created, the declarations that conflict with a stored account or would be refused, and the per-account totals that would post. A `close` naming a missing account counts as an unresolved account, and an `open` naming an unregistered commodity as an unresolved commodity. It is the same run with its writes diverted, not a second implementation, so it cannot drift from what `run` does. The report leads with what is broken rather than what would succeed, because it exists for profile tuning; `--json` covers it as it does the other three, minus the `batch_id` key, since a dry run opens no batch and so leaves nothing to `list` or `discard`.

`sync` is the sweep over the same engine: `--profile <name>` runs one profile, `--all` runs every profile in name order, and `--dry-run` plans each without writing. A committing sweep takes one `pre-import` snapshot before its first write, not one per profile, and still opens one batch per profile so any one import stays independently discardable. A profile whose importer fails is recorded and the sweep continues; the command then exits non-zero after printing, naming the snapshot and every batch id. A run that stops after its batch opened — a write or the batch close failed — leaves that batch open with every row written before the stop, and the report names it so `import discard` can undo it; the rows are never rolled back. Under `--dry-run` it also exits non-zero when any profile has a *blocker* — an unresolved account or commodity, whether a posting or a declaration names it, or a posting skipped for any other cause — which is the gate a bootstrap script sequences before the real sweep. Warnings never block. `import run` and `sync --profile` do the same work and exit with the same error for the same failure; `import run` prints the detailed per-profile plan, `sync` one row per profile, and `--json` on `sync` embeds the `import run` payload per profile so the two read alike. A sweep's failures share one exit code, since several profiles have no single error to forward.

Import profiles are created and edited from the CLI. `profile create` takes the
importer's opaque config as a TOML or JSON file (`--config <FILE>`, or `-` for
stdin); TOML is converted to JSON inside `bc-cli`, so `bc-core` and the plugin
ABI continue to see a single JSON blob. Profile names are unique and are the
identifier every surface uses. An unrecognised `--importer` is a warning, not
an error: the plugin may not be installed yet, and `import` errors at the point
of use.

`csv` and `json` native export are post-v1 additions (see §5.1 format compatibility table) and will extend the `--format` option when implemented.

All commands support `--json` for structured output. Shell completions are generated on demand via `borrow-checker completions <bash|elvish|fish|powershell|zsh>`.

Backup (`backup`, `backup list`, `backup delete`, `backup rekey`) and restore (see §4.6) are exposed as CLI commands over the same `bc-core` service the GUI uses.

### 8.2 Tauri GUI (`bc-app`)

Layout: **icon rail + context-sensitive content**.

- Icon rail (left): Dashboard · Accounts · Budget · Reports · Plugins (plugin icons auto-append)
- **Dashboard** is the home screen: net worth, spend this month, budget remaining, recent transactions, budget health bars, quick-import button
- Accounts view: account tree (left panel) + transaction list + detail (right panel)
- Power users navigate directly via the account tree; new users land on the dashboard
- Settings → Backup panel: edit backup settings, trigger a manual backup, and restore from or delete an existing snapshot (see §4.6)

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
