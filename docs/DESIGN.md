# BorrowChecker — Design Specification

**Date:** 2026-03-20
**Status:** Approved
**Pun:** Rust's borrow checker + personal finance (borrowing money)

______________________________________________________________________

## 1. Overview

BorrowChecker is a distributable, open-source personal finance application written in Rust. It targets users who want the transparency and auditability of plain-text accounting tools (ledger, beancount) without the UX tax of writing transactions by hand, wrestling with CSV imports, or manually composing reports.

**Core principles:**

- No lock-in: data is always exportable to open formats
- Plain-text compatibility: ledger and beancount files are first-class citizens
- Extensible: a WASM plugin system lets the community add importers, processors, and reports
- Multiple surfaces: CLI for scripting, TUI for terminal power users, Tauri GUI for everyone else

______________________________________________________________________

## 2. Goals

- Full read/write compatibility with ledger and beancount file formats
- SQLite as the internal storage engine (fast, reliable, portable)
- Append-only event log in the core (audit trail, undo/redo, future sync)
- Double-entry accounting enforced at the core level
- Import profiles: account-bound importer configurations that eliminate import ambiguity
- Envelope/zero-based budgeting as the default model, with category tracking as a fallback
- Fortnightly and financial-year periods as first-class budget intervals
- WASM plugin system with explicit ABI versioning and a graceful deprecation/grace-period policy
- Transaction processor pipeline (generalisation of categorisation)
- CLI, TUI, and Tauri GUI developed in parallel from the start
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
├── ROADMAP.md
├── crates/
│   ├── bc-models/              # shared domain types (accounts, transactions, envelopes, etc.)
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
│   ├── bc-tui/                 # ratatui TUI
│   └── bc-app/                 # Tauri GUI
└── plugins/                    # first-party example/bundled plugins
```

**Design philosophy — keep crates small and focused.** Each crate should have one clear purpose, a well-defined public API, and minimal dependencies. As the project grows it is expected and encouraged to introduce new crates or split existing ones. Utility crates will likely emerge as needed — e.g. `bc-config` (configuration management), `bc-otel` (OpenTelemetry tracing/metrics). Prefer creating a new crate over stuffing shared functionality into an existing one.

**Dependency relationships:**

- `bc-models` has no internal dependencies — it is the shared vocabulary for the whole workspace
- `bc-core` depends on `bc-models`; `bc-cli`, `bc-tui`, `bc-app` depend on `bc-core`
- `bc-format-*` crates depend on `bc-models` (domain types) and `bc-core` (import profiles, config)
- `bc-plugins` depends on `bc-core` (bridges WASM into the engine)
- `bc-sdk` is standalone — plugin authors only need it, not the full workspace

### 4.2 Core Engine (`bc-core`)

The core owns two layers:

**Storage layer (SQLite via `sqlx`):**

| Table | Purpose |
| ------------------ | ---------------------------------------------------- |
| `events` | Append-only event log — never updated, never deleted |
| `accounts` | Projected read model |
| `transactions` | Projected read model |
| `balances` | Projected read model (table exists in M1 schema as a planned cache; M1 queries the `postings` table live — this cache will be populated in a later milestone for performance) |
| `budget_envelopes` | Projected read model _(delivered in Milestone 5)_ |
| `asset_valuations` | Projected read model — latest market value per ManualAsset account _(delivered in Milestone 5A)_ |
| `asset_depreciations` | Projected read model — depreciation history per ManualAsset account _(delivered in Milestone 5A)_ |
| `loan_terms` | Projected read model — loan terms per Receivable account _(delivered in Milestone 5A)_ |
| `import_profiles` | Account-bound importer configurations _(delivered in Milestone 2)_ |
| `meta` | Schema version, user preferences, last-sync cursor |

**Event vocabulary:**

```
AccountCreated / AccountUpdated / AccountArchived
TransactionCreated / TransactionAmended / TransactionVoided
EnvelopeCreated / EnvelopeAllocated / EnvelopeMoved
ImportBatchStarted / ImportBatchCompleted
PluginRegistered / PluginUnregistered
AssetValuationRecorded
DepreciationCalculated
LoanTermsSet
```

**What the event log provides:**

- Undo/redo — walk the log backward/forward
- Full audit trail — every change is timestamped and sourced
- Time-travel queries — "what was my balance on 1 Jan?"
- Import idempotency — deduplication by content hash
- Future sync — replicate events to a server or mobile device

**Double-entry accounting** is enforced at the core level: every transaction must balance to zero across accounts, consistent with ledger/beancount semantics.

### 4.3 Account Model

Accounts are classified by `AccountType` (Asset, Liability, Equity, Income, Expense) — the canonical double-entry roots. The five variants are stable and unlikely to change; `#[non_exhaustive]` covers rare future additions.

**Hierarchy via `parent_id`:**

Accounts form an arbitrary-depth tree through an optional `parent_id: Option<AccountId>`. A root account (`parent_id = None`) is the authority for its `AccountType`; child accounts inherit their root's type (enforced in `bc-core` at creation time). The hierarchy supports:

- Institution grouping: `Assets > CommBank > Savings, Checking`
- Virtual sub-accounts: `Assets > CommBank > Offset > Mine, Partner, Shared`
- Rollups: summing a subtree gives the parent balance; virtual sub-accounts of a joint account should always sum to the real account's bank-statement balance
- Beancount/ledger export: the colon-separated path is derived by walking the ancestor chain

**Account kind** governs how a leaf account's balance is maintained:

| Kind | Description |
| ---------------- | ----------- |
| `DepositAccount` | Reconciles against a bank/card/brokerage statement. May have an import profile. Examples: checking, savings, credit card, investment portfolio. |
| `ManualAsset` | Manually-maintained real asset with no bank statement. Balance driven by valuation events. Examples: real property, vehicle, private equity stake. |
| `Receivable` | Money owed to you by a third party. Tracked via ordinary transactions (disbursement + repayments). May carry optional loan terms for amortization assistance. Examples: personal loan to a friend, loan to a trust. |
| `VirtualAllocation` | No independent existence. Subdivides a parent account's balance. Examples: earmarked sub-accounts within an offset account. |

Only `DepositAccount` accounts may have an import profile — enforced in `bc-core` at creation time.

**Cross-cutting labels via an entity-based tag model:**

The primary hierarchy can only express one grouping at a time. Cross-cutting concerns — ownership (mine / partner / shared), institution grouping across types, liquidity flags — are expressed as tag references on accounts.

Tags are first-class entities stored in a `tags` table with `id`, `name`, `parent_id` (self-referential for tag hierarchy), and `description`. `Account` holds `tag_ids: Vec<TagId>` — stable opaque references that survive renames. Human-readable paths are derived on demand via `TagForest::path_of(id) -> TagPath`: a `TagPath` is an ordered sequence of non-empty segments (`["institution", "commbank"]`) that serialises as a colon-joined string (`institution:commbank`).

The `account_tags` join table links accounts to their tags in the database.

Example tag paths: `institution:commbank`, `owner:mine`, `owner:shared`, `liquid`.

**Expense categorisation is separate from account hierarchy:**

Deep expense category trees (food > restaurant, food > groceries, holiday > food, etc.) belong to the **envelope / budget system** (Milestone 5), not to `Account`. Account hierarchy is for real and virtual financial accounts. Cross-cutting expense views are handled via tags on transactions and postings, enabling queries like "all food spend regardless of holiday context."

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
    fn name(&self) -> &'static str;
    fn detect(&self, bytes: &[u8]) -> bool;  // format / profile-aware sniffing
    fn import(&self, bytes: &[u8], config: &ImportConfig) -> Result<Vec<RawTransaction>, ImportError>;
}
```

The importer is a **pure parsing concern** — it converts bytes to `RawTransaction` values with no opinion about which account they belong to.

> **Option C factory pattern** — foundations implemented in Milestone 2, full plugin registry deferred to Milestone 6. `ImporterFactory` (in `bc-core`) holds two fn pointers: `fn(&[u8]) -> bool` for stateless format-level detection and `fn() -> Box<dyn Importer>` for instance creation. `ImporterRegistry` stores a list of factories and provides `detect_format`, `create_for_name`, and `create_for_bytes`. Each format crate exposes a free `importer_factory()` function. The `Importer::detect(&self, ...)` method is retained for profile-aware detection after an instance is configured with a specific account's import profile.

### 5.3 Import Profiles

Account binding lives in `bc-core` as an `ImportProfile`:

```rust
struct ImportProfile {
    id: ProfileId,                   // newtype wrapper around TypeId (see ID convention below)
    name: String,                    // e.g. "CommBank Savings"
    importer: String,                // e.g. "commbank-au"
    account_id: AccountId,           // where transactions land
    config: ImportConfig,            // column mappings, date formats, etc.
    created_at: Timestamp,
}
```

**ID convention:** All ID types (`ProfileId`, `AccountId`, `TransactionId`, etc.) are newtype wrappers around a typed prefixed ID from the [`mti`](https://crates.io/crates/mti) crate. This produces human-readable, type-safe IDs like `profile_01h455vb4pex5vsknk084sn02q` — the prefix makes the type visible in logs and debug output, and the Rust newtype ensures IDs are never confused with each other at compile time. All ID types are defined in `bc-models`.

Multiple profiles can reference the same importer with different account bindings. All CLI/TUI/GUI import operations work on profiles, not raw importers.

> **Deduplication deferred:** A `dedup_strategy` field (planned values: `None / ContentHash / FitId`) was initially scoped for v1 but has been deferred. Import idempotency via content-hash deduplication will be added in a later milestone once the full import pipeline matures. Until then, importing the same file twice will create duplicate transactions.

______________________________________________________________________

## 6. Plugin System

### 6.1 Runtime

- **`bc-plugins`**: WASM host runtime using [extism](https://extism.org/) (built on wasmtime)
- **`bc-sdk`**: Standalone crate published to crates.io. Plugin authors depend on this, compile to `wasm32-wasip1`, and distribute a single `.wasm` file.
- **Plugin discovery**: `~/.config/borrow-checker/plugins/` (configurable). A `plugins.toml` manifest lists enabled plugins and their configuration.

### 6.2 Plugin Manifest

Every plugin embeds or ships a sidecar manifest:

```toml
[plugin]
name        = "commbank-au"
version     = "1.2.0"
sdk_abi     = 2           # ABI version compiled against
min_host    = "0.5.0"     # minimum BorrowChecker version required
```

### 6.3 ABI Versioning

The SDK uses a **single integer ABI version**, separate from semver. Only breaking changes increment it; additive changes use capability negotiation (plugins query at runtime whether a host function exists).

**Support window policy:** A new ABI version is announced at release N. The previous version is deprecated and dropped no earlier than release N+2 — giving plugin authors at least one full release cycle to migrate.

| BorrowChecker | Supported ABIs | Notes |
| ------------- | -------------- | ------------------------- |
| 0.x | `[1]` | Initial release |
| 1.x | `[1, 2]` | v1 deprecated, v2 active |
| 2.x | `[2, 3]` | v1 dropped, v2 deprecated |

During the grace period the host loads deprecated-ABI plugins via a compatibility shim and warns the user at startup with a link to the migration guide.

### 6.4 Plugin Phases

**Phase 1 — Importers (Milestone 6, critical)**

Plugins implement the `Importer` trait. Registered by name; referenced in import profiles.

**Phase 2 — Transaction Processors (Milestone 8)**

A general-purpose pipeline that runs after import, before committing events. Each processor receives a `PendingTransaction` plus read-only context (account history, FX rates, user prefs) and returns a modified transaction or a review flag.

```rust
fn process(tx: PendingTransaction, ctx: &TransactionContext) -> ProcessorResult
```

Example processors: merchant normalisation, auto-categorisation, auto-split, FX enrichment, tax flagging, recurring detection, anomaly flagging, envelope auto-assignment. Processors declare a priority; pipeline order is deterministic and configurable.

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

Default methodology is **envelope/zero-based budgeting** (every dollar assigned to a purpose). Users who don't want zero-based budgeting use envelopes with no allocation target — they become plain category trackers. The data model is identical; it's a workflow preference.

**Envelope fields:**

- Name, parent envelope (nestable to arbitrary depth), icon/colour
- Allocation target (optional)
- Commodity (optional — required only when commodity-specific reporting is needed)
- Period (see §7.2)
- Rollover policy: carry forward / reset to zero / cap at target
- Tags (cross-cutting labels; see below)
- Linked account(s)

**Hierarchy:**

Envelopes form an arbitrary-depth tree via `parent_id: Option<EnvelopeId>`. There is no separate "group" entity — a parent envelope is simply an envelope whose children roll their actuals and allocations upward. This mirrors the `Expense:Health:Gym:Me` hierarchy from ledger/beancount. Postings are assigned to leaf envelopes only; parent envelopes aggregate their children's actuals and allocations upward automatically.

**Budget assignment vs. reporting dimensions:**

Two orthogonal mechanisms exist for categorising spending:

- `posting.envelope_id` → **where** the money was budgeted (zero-based assignment; one leaf envelope per posting; the primary budget signal)
- `posting.tag_ids` / `envelope.tag_ids` → **how** to slice for reporting (multi-dimensional; many tags per entity; enables cross-cutting views like "all spending tagged `person:me`" or "total budget for all envelopes tagged `context:holiday`")

Example: a gym posting assigned to `Health > Gym > Me` envelope and tagged `person:me`. This counts against the gym budget explicitly, and also appears in any "personal spending" report filtered by `person:me` tag — without double-counting in the budget.

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

**Mixed-period display:** all envelopes normalise to a user-chosen display period (monthly by default). An annual `Car Registration` envelope that accumulates monthly is displayed correctly alongside monthly grocery envelopes.

______________________________________________________________________

## 8. Frontends

### 8.1 CLI (`bc-cli`)

Thin binary over `bc-core`. Commands:

```
borrow-checker account [list|create|archive]
borrow-checker transaction [list|add|amend|void]
borrow-checker asset [record-valuation|depreciate|set-loan-terms|amortization|book-value]
borrow-checker import --profile <name> --counterpart <account-id> <file>
borrow-checker export --format <ledger|beancount> --output <file>
borrow-checker report [net-worth|summary|budget]
borrow-checker budget [status|allocate|envelopes]
borrow-checker plugin [install|list|remove]
borrow-checker completions <bash|elvish|fish|powershell|zsh>
```

`--counterpart` provides the offsetting account for the balancing posting on each imported line. CSV and OFX importers produce single-account `RawTransaction` values; the counterpart account is required to satisfy double-entry balance constraints.

`csv` and `json` native export are post-v1 additions (see §5.1 format compatibility table) and will extend the `--format` option when implemented.

All commands support `--json` for structured output. Shell completions are generated on demand via `borrow-checker completions <bash|elvish|fish|powershell|zsh>`.

### 8.2 TUI (`bc-tui`)

Built with [ratatui](https://ratatui.rs/).

Layout: **sidebar + main panel**. Account tree on the left; keyboard-first navigation. Views: Transactions, Budget, Reports. Vim-inspired key bindings with a discoverable `?` help overlay.

### 8.3 Tauri GUI (`bc-app`)

Layout: **icon rail + context-sensitive content**.

- Icon rail (left): Dashboard · Accounts · Budget · Reports · Plugins (plugin icons auto-append)
- **Dashboard** is the home screen: net worth, spend this month, budget remaining, recent transactions, envelope health bars, quick-import button
- Accounts view: account tree (left panel) + transaction list + detail (right panel)
- Power users navigate directly via the account tree; new users land on the dashboard

______________________________________________________________________

## 9. ROADMAP Summary

| Milestone | Description | Depends on |
| --------- | ------------------------------------------------ | ---------- |
| 0 | Project foundation (workspace, CI, docs) | — |
| 1 | Core engine (`bc-core`, SQLite, event log) | 0 |
| 2 | Format compatibility (`bc-format-*` crates) | 1 |
| 3 | CLI (`bc-cli`) | 1, 2 |
| 4 | TUI (`bc-tui`) | 1, 5\* |
| 5 | Budgeting (nested envelope hierarchy, tags, allocation, all periods) | 1 |
| 5A | Illiquid asset tracking (valuations, depreciation, loan terms) | 1, 5 |
| 6 | Plugin Phase 1: Importers | 2, 3 |
| 7 | Tauri GUI (`bc-app`) | 1, 2, 5 |
| 8 | Plugin Phase 2: Transaction Processors | 6 |
| 9 | Plugin Phase 3: Report Generators | 8 |
| 10 | Plugin Phase 4: UI Extensions | 7, 9 |
| 11 | Sync & multi-device (event replication, Android) | 10 |

_\* Milestone 4 (TUI) can start after Milestone 1 with basic transaction views. Budget and report views within the TUI are deferred until Milestone 5 is complete._

______________________________________________________________________

## 10. Key Technical Decisions

| Decision | Choice | Rationale |
| ---------------------- | -------------------------------------------- | ---------------------------------------------------------------- |
| Storage | SQLite via `sqlx` | Portable, zero-server, fast for single-user workloads |
| Event log | Append-only SQLite table | Audit trail, undo/redo, future sync without full CQRS overhead |
| WASM runtime | extism (wasmtime-backed) | Higher-level than raw wasmtime; handles ABI plumbing |
| TUI framework | ratatui | De-facto standard in the Rust ecosystem |
| GUI framework | Tauri | Rust-native, small binary, web frontend flexibility |
| Plugin ABI versioning | Integer ABI + N+2 grace period | Simple, explicit, protects the community ecosystem |
| Budget default | Envelope/zero-based | Most intentional model; degrades gracefully to category tracking |
| Importer/account split | Importer = parser, Profile = account binding | Clean separation; same parser serves multiple accounts |
| ID types | `mti` newtype wrappers (e.g. `profile_01h…`) | Type-safe, log-readable, no ID confusion across domain types |
