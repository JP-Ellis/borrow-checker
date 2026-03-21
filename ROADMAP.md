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

**Status:** ◻️
**Crate:** `bc-core`

- [ ] SQLite storage layer via `sqlx` with migrations
- [ ] Append-only event log table + projection engine
- [ ] Event vocabulary: Account, Transaction, Envelope, Import, Plugin events
- [ ] Account model (create, archive, multi-currency support)
- [ ] Double-entry transaction model (enforced balance-to-zero)
- [ ] Balance calculation engine
- [ ] Period model (weekly, fortnightly, monthly, quarterly, financial year, calendar year, custom)
- [ ] Global settings (financial year start, fortnightly anchor date, display currency)

______________________________________________________________________

## Milestone 2 — Format Compatibility

**Status:** ◻️
**Crates:** `bc-format-csv`, `bc-format-ledger`, `bc-format-beancount`, `bc-format-ofx`

- [ ] `Importer` + `Exporter` traits
- [ ] CSV import (configurable column mapping)
- [ ] Import profiles (account-bound importer configs, dedup strategy)
- [ ] Ledger read + write (round-trip)
- [ ] Beancount read + write (round-trip)
- [ ] OFX/QFX import
- [ ] Format auto-detection (`detect()`)

______________________________________________________________________

## Milestone 3 — CLI

**Status:** ◻️
**Crate:** `bc-cli`

- [ ] `account` commands (list, create, archive)
- [ ] `transaction` commands (list, add, amend, void)
- [ ] `import` command (`--profile <name> <file>`)
- [ ] `export` command (`--format`, `--output`)
- [ ] `report` commands (monthly, annual, net-worth, budget)
- [ ] `budget` commands (status, allocate, envelopes)
- [ ] `plugin` commands (install, list, remove)
- [ ] `--json` flag on all commands for scripting
- [ ] Shell completions (bash, zsh, fish)

______________________________________________________________________

## Milestone 4 — TUI

**Status:** ◻️
**Crate:** `bc-tui`
**Depends on:** Milestone 1 (core views); budget/report views deferred until Milestone 5

- [ ] Sidebar + main panel layout (ratatui)
- [ ] Account tree navigation (keyboard-first)
- [ ] Transaction list + detail view
- [ ] Vim-inspired key bindings
- [ ] `?` help overlay
- [ ] Budget envelope view _(requires Milestone 5)_
- [ ] Reports view _(requires Milestone 5)_

______________________________________________________________________

## Milestone 5 — Budgeting

**Status:** ◻️
**Crate:** `bc-core` extension

- [ ] Envelope model (name, group, icon, allocation target, rollover policy)
- [ ] Envelope groups (nestable categories)
- [ ] All period types (incl. fortnightly + financial year)
- [ ] Allocation workflow (zero-based: assign every dollar)
- [ ] Category tracking mode (envelopes without allocation target)
- [ ] Rollover policies (carry forward / reset / cap)
- [ ] Budget vs. actuals tracking
- [ ] Mixed-period display normalisation
- [ ] Budget views in CLI + TUI

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
