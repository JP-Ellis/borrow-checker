# BorrowChecker Roadmap

> A living document. Status: ◻️ planned · 🔄 in progress · ✅ complete

BorrowChecker is a Rust-based personal finance application with ledger/beancount compatibility, a WASM plugin system, and three interfaces: CLI, TUI, and Tauri GUI.

______________________________________________________________________

## Milestone 0 — Project Foundation

**Status:** ✅

- [x] Cargo workspace skeleton (`crates/` layout)
- [x] GitHub Actions CI (test, clippy, fmt)
- [x] License (MIT), CONTRIBUTING.md, CODE_OF_CONDUCT.md
- [x] `.gitignore`, `rust-toolchain.toml`
- [x] Basic `docs/` structure
- [x] `CHANGELOG.md` bootstrapped
- [x] Release-plz config

______________________________________________________________________

## Milestone 1 — Core Engine

**Status:** ✅
**Crates:** `bc-core`, `bc-config`, `bc-otel`

- [x] SQLite storage layer via `sqlx` with migrations
- [x] Append-only event log table + projection engine
- [x] Event vocabulary: Account events (`AccountCreated`, `AccountUpdated`, `AccountArchived`) and Transaction events (`TransactionCreated`, `TransactionAmended`, `TransactionVoided`)
- [x] Account model (create, archive, multi-currency support; parent hierarchy; cross-cutting `TagPath` labels; `AccountKind` discriminator)
- [x] Double-entry transaction model (enforced balance-to-zero)
- [x] Balance calculation engine
- [x] Period model (weekly, fortnightly, monthly, quarterly, **financial quarter**, financial year, calendar year, custom)
- [x] Global settings (financial year start, fortnightly anchor date, display currency)

______________________________________________________________________

## Milestone 2 — Format Compatibility

**Status:** ✅
**Crates:** `bc-format-csv`, `bc-format-ledger`, `bc-format-beancount`, `bc-format-ofx`

- [x] `Importer` + `Exporter` traits
- [x] CSV import (configurable column mapping)
- [x] Import profiles (account-bound importer configs, dedup strategy)
- [x] Ledger read + write (round-trip)
- [x] Beancount read + write (round-trip)
- [x] OFX/QFX import
- [x] Format auto-detection (`detect()`)

______________________________________________________________________

## Milestone 3 — CLI

**Status:** ✅ (budget/plugin commands stubbed; see Milestone 5/6)
**Crate:** `bc-cli`

- [x] `account` commands (list, create, archive)
- [x] `transaction` commands (list, add, amend, void)
- [x] `import` command (`--profile <name> <file>`)
- [x] `export` command (`--format`, `--output`)
- [x] `report` commands (monthly, annual, net-worth)
- [ ] `report budget` _(requires Milestone 5)_
- [x] `--json` flag on all commands for scripting
- [x] Shell completions (bash, zsh, fish)

______________________________________________________________________

## Milestone 4 — TUI

**Status:** ✅
**Crate:** `bc-tui`
**Depends on:** Milestone 1 (core views); budget/report views depend on Milestone 5

- [x] Sidebar + main panel layout (ratatui)
- [x] Account tree navigation (keyboard-first)
- [x] Transaction list + detail view
- [x] Vim-inspired key bindings
- [x] `?` help overlay
- [x] Budget envelope view _(requires Milestone 5)_
- [x] Reports view _(requires Milestone 5)_

______________________________________________________________________

## Milestone 5 — Budgeting

**Status:** ✅
**Crate:** `bc-core` extension

- [x] Envelope model (name, parent hierarchy, icon, allocation target, rollover policy)
- [x] Arbitrary-depth envelope hierarchy via `parent_id: Option<EnvelopeId>` (no separate group entity)
- [x] Envelope tags for cross-cutting budget views
- [x] All period types (incl. fortnightly + financial year)
- [x] Allocation workflow (zero-based: assign every dollar)
- [x] Category tracking mode (envelopes without allocation target)
- [x] Rollover policies (carry forward / reset / cap)
- [x] Budget vs. actuals tracking
- [x] Mixed-period display normalisation
- [x] Budget views in CLI
- [ ] Budget views in TUI _(out of scope for Milestone 5; planned for Milestone 4 (TUI))_

______________________________________________________________________

## Milestone 5A — Illiquid Asset Tracking

**Status:** ✅
**Crates:** `bc-models`, `bc-core`
**Depends on:** Milestone 1, Milestone 5

- [x] `AccountKind` enum on `Account` model (`DepositAccount`, `ManualAsset`, `Receivable`, `VirtualAllocation`) — _implemented in Milestone 1_
- [x] `bc-core` enforces: only `DepositAccount` may have an import profile
- [x] `AssetValuationRecorded` event: point-in-time market value with `ValuationSource`
- [x] `ValuationSource` enum: `ManualEstimate`, `ProfessionalAppraisal`, `TaxAssessment`, `MarketData`, `AgreedValue`
- [x] Auto-generated balancing transaction on valuation (optional, via `counterpart_id`)
- [x] `DepreciationPolicy` on `ManualAsset` accounts: `None`, `StraightLine`, `DecliningBalance`
- [x] On-demand depreciation calculation (`DepreciationCalculated` event)
- [x] Depreciation entries: `Expenses:Depreciation:<name>` / credit asset account
- [x] Book value (depreciation basis) and market value (appraisal) tracked separately in reports
- [x] `LoanTermsSet` event: principal, interest rate, start date, term, repayment frequency
- [x] Amortization schedule calculation for display and repayment-split assistance
- [x] `acquisition_date` + `acquisition_cost` optional fields on `ManualAsset` accounts (cost basis, depreciation baseline)
- [x] `RepaymentFrequency` enum: `Weekly`, `Fortnightly`, `Monthly`, `Quarterly`, `Custom`
- [x] Net worth calculation includes all `AccountKind` variants (zero balance if no value recorded)
- [x] CLI support: record valuation, trigger depreciation, set loan terms
- [ ] TUI support: record valuation, trigger depreciation, set loan terms (blocked on Milestone 4)
- [x] CLI `asset book-value` command

______________________________________________________________________

## Milestone 6 — Plugin System Phase 1: Importers

**Status:** ◻️
**Crates:** `bc-plugins`, `bc-sdk`

- [ ] WASM host runtime (extism / wasmtime)
- [ ] Plugin manifest schema (name, version, sdk_abi, min_host)
- [ ] Plugin discovery (`~/.config/borrow-checker/plugins/`, `plugins.toml`)
- [ ] ABI versioning scheme (integer ABI v1, capability negotiation)
- [ ] Grace-period deprecation policy (N+2 rule, startup warnings)
- [ ] `Importer` plugin interface + host bridge
- [ ] `bc-sdk` v1 published to crates.io
- [ ] Example importer plugin (generic CSV)

______________________________________________________________________

## Milestone 7 — Tauri GUI

**Status:** ◻️
**Crate:** `bc-app`

- [ ] Icon rail navigation (Dashboard, Accounts, Budget, Reports, Plugins)
- [ ] Dashboard home (net worth, spend, budget remaining, recent transactions, envelope bars, quick-import)
- [ ] Accounts view (account tree + transaction list + detail panel)
- [ ] Budget envelope view + progress bars
- [ ] Basic charts (net worth over time, spend by category)
- [ ] Onboarding flow (financial year start, fortnightly anchor, first account)

______________________________________________________________________

## Milestone 8 — Plugin System Phase 2: Transaction Processors

**Status:** ◻️
**Crates:** `bc-core`, `bc-plugins`, `bc-sdk`

- [ ] Transaction processor pipeline in `bc-core`
- [ ] `ProcessorResult` (modify / flag for review / split / pass-through)
- [ ] Read-only `TransactionContext` (account history, FX rates, user prefs)
- [ ] Pipeline ordering + priority configuration
- [ ] Processor plugin interface + `bc-sdk` ABI v2
- [ ] Built-in processors: merchant normalisation, auto-categorisation
- [ ] Review queue for flagged transactions (CLI + TUI + GUI)

______________________________________________________________________

## Milestone 9 — Plugin System Phase 3: Report Generators

**Status:** ◻️
**Crates:** `bc-plugins`, `bc-sdk`

- [ ] `ReportQuery` + `ReportData` + `ReportOutput` types
- [ ] `ReportGenerator` plugin interface + `bc-sdk` update
- [ ] Chart data format (series + chart type hint; frontend renders)
- [ ] Plugin-provided reports surfaced in Tauri (Reports section)
- [ ] Plugin-provided reports accessible via CLI (`--json`)

______________________________________________________________________

## Milestone 10 — Plugin System Phase 4: UI Extensions

**Status:** ◻️
**Crates:** `bc-plugins`, `bc-app`

- [ ] Plugin-declared Tauri panel pages
- [ ] Plugin icons auto-registered in navigation rail
- [ ] Plugin sandbox / permission model for UI extensions
- [ ] Community plugin registry / discovery (design + prototype)

______________________________________________________________________

## Milestone 11 — Sync & Multi-Device

**Status:** ◻️

- [ ] Event log replication protocol (design doc)
- [ ] Optional self-hosted sync server
- [ ] Conflict resolution strategy
- [ ] Multi-device event merge
- [ ] Android companion app _(stretch goal)_

______________________________________________________________________

## Ideas Backlog

> Features that don't fit current milestones but shouldn't be forgotten.

- Investment / portfolio tracking
- Open Banking / bank API integrations
- Multi-user / shared accounts
- Recurring transaction detection (built-in, beyond plugin)
- Tax report generation (ATO, HMRC, IRS formats)
- Receipt attachment (image → transaction)
- Natural language transaction entry
- Browser extension for one-click import
