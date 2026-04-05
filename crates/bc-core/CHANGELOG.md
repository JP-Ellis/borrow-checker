# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- markdownlint-disable -->
## [0.1.0](https://github.com/JP-Ellis/borrow-checker/releases/tag/bc-core/v0.1.0) - _2026-04-05_

### 🚀 Features
-   _(core)_ Validate set_parent inputs
-   _(core)_ Expand EnvelopeCreated event fields
-   _(models,core,cli)_ Milestone 5 — budgeting system
-   _(cli)_ Budget groups/envelopes/allocate/status
-   _(core)_ Persist and load envelope_id on postings
-   _(core)_ Add BudgetEngine
-   _(core)_ Add allocation CRUD to EnvelopeService
-   _(core)_ Add EnvelopeService CRUD
-   _(core)_ Add envelope event variants to Event enum
-   _(core)_ Add envelope schema migrations (0009 + 0010)
-   _(core)_ Guard acquisition fields to ManualAsset
-   _(models,core,cli)_ AU mortgage loan model
-   _(core)_ Add BalanceEngine::net_worth()
-   _(core)_ Add LoanService with amortization schedule
-   _(core)_ Add AssetService
-   _(core)_ Persist account acquisition fields
-   _(core)_ InvalidAccountKind + profile guard
-   _(core)_ Add illiquid asset event variants
-   _(core)_ Add migrations for illiquid asset tables
-   _(core)_ Add ImportProfileService::list_all()
-   _(core)_ Implement TransactionService::amend()
-   _(bc-core)_ Add DedupStrategy and update ImportProfile
-   _(bc-core)_ Add static name(), from_value, as_typed
-   _(bc-core)_ Add ImporterRegistry for format auto-detection
-   _(bc-core)_ Add ImporterFactory
-   _(ledger)_ Add importer and exporter
-   _(core)_ Add ImportProfile storage with CRUD service
-   _(core)_ Add ExportData and Exporter trait
-   _(core)_ Add import contract types
-   _(bc-core)_ Persist commodity/tag links on create
-   _(bc-core)_ Full state in AccountCreated event
-   _(bc-core)_ Add parent_id to accounts schema
-   _(bc-core)_ Persist and query AccountKind
-   _(bc-core)_ Add kind column to accounts table
-   _(bc-core)_ Replace GlobalSettings stub
-   _(bc-core)_ Update transaction.rs for new schema
-   _(bc-core)_ Update account.rs for new schema
-   _(bc-models)_ Add account hierarchy and TagPath
-   _(bc-core)_ Implement core engine
-   Add workspace root and library crate stubs

### 🐛 Bug Fixes
-   _(core)_ Add backticks to identifiers in doc comment
-   _(core)_ Resolve post-rebase conflicts from M5A merge
-   _(core)_ Box recursive rollover_for future
-   _(core)_ Multi-period carry-forward rollover
-   _(core)_ Propagate non-NotFound errors in set_parent
-   _(core)_ Require commodity with allocation_target
-   _(core)_ Address post-merge CI and review feedback
-   _(core)_ Clean up rollover_for match arm
-   _(core)_ Offset must not reduce annuity payment
-   _(core)_ Warn and test depreciation clamp edge cases
-   _(core)_ Log clamped depreciation amount in record_depreciation
-   _(core)_ Clamp depreciation amount to [0, book_val]
-   _(core)_ Validate loan terms inputs in set_loan_terms
-   _(core)_ Improve AccountKind guard docs and test coverage
-   _(core)_ Fix repayment_frequency column comment
-   Clippy warnings and migration README
-   Cleanup from PR#24 review
-   _(core)_ Inclusive day count in depreciation
-   _(core)_ Clarify DepreciationPolicy::None error
-   _(core,cli)_ Reject period_days == 0
-   _(core)_ Guards, errors, ordering, overflow
-   _(core)_ Lint, delegation, and error handling fixes
-   _(models,core)_ Introduce DepreciationId newtype
-   Milestone 5A code review fixes
-   _(core)_ Delta in valuation tx; reads in tx
-   _(test,core)_ Fix Windows CI + Copilot review
-   _(core)_ Prevent FK violation in transaction amend
-   _(core,cli)_ Path-safe SQLite open + skip empty parent
-   _(cli,core)_ Doc clarity and machete cleanup
-   _(review)_ Address Milestone 3 code review findings
-   _(bc-core)_ Correct param in registry doc test
-   _(bc-core)_ Remove Copy; add detect/create tests
-   Address Copilot review findings
-   _(ci)_ Resolve clippy and Windows test failures
-   _(bc-core)_ Drop UNIQUE on commodities.code
-   _(bc-core)_ Correct settings doc links
-   _(bc-core)_ Use checked_add for financial arithmetic
-   _(bc-core)_ Fix TOCTOU, voided string, position overflow
-   _(bc-core)_ Remove explicit deref-reborrow
-   _(bc-models)_ Address quality review blockers
-   _(bc-models)_ Address code review findings
-   _(bc-models)_ Wire thin ID re-exports
-   Resolve clippy warnings in test code
-   _(bc-core)_ Add DB transactions

### 🚜 Refactor
-   Address milestone-5 PR review feedback
-   _(core,cli)_ Bon builder for AccountService::create
-   _(core)_ FromRow struct + explicit match
-   _(bc-core)_ Address review comments
-   _(milestone/1)_ Address final code review findings
-   _(bc-models)_ Privatise Amount fields
-   _(bc-core)_ Extract insert_event helper
-   _(bc-core)_ Address reviewer feedback
-   _(bc-core)_ Tidy AccountKind tests
-   _(bc-models)_ Finalise private modules
-   _(bc-models)_ Rewrite transaction.rs
-   _(bc-models)_ Rewrite Account with bon

### ⚡ Performance
-   _(core)_ Enable WAL mode and NORMAL sync for SQLite

### 🎨 Styling
-   Use item import granularity

### 📚 Documentation
-   _(core)_ Fix stale rollover_for doc comment
-   _(core)_ Clarify allocate() upsert/event-log semantics
-   Events.rs aggregate comment; ROADMAP TUI
-   Book-value command + migration numbering

### 🧪 Testing
-   _(core)_ Complete envelope event round-trip tests
-   _(core)_ Verify ManualAsset fields persist
-   Add missing coverage for events, settings, models
-   _(bc-core)_ Event log clean after failed ops
-   Add trivial tests to all crates

### ⚙️ Miscellaneous Tasks
-   Add cargo-machete to CI and pre-commit hook
-   _(bc-core)_ Add SQL performance indexes
-   _(bc-core)_ Rewrite migrations for new schema
-   Add linting and formatting tools

