# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- markdownlint-disable -->
## [0.1.0](https://github.com/JP-Ellis/borrow-checker/releases/tag/bc-models/v0.1.0) - _2026-08-02_

### 🚀 Features
-   _(bc-core)_ Record whether an import made a posting
-   _(bc-core)_ Tombstone source refs of deleted legs
-   _(bc-core)_ Record import batch provenance
-   _(bc-models)_ Source refs point at postings
-   _(bc-models)_ Add SourceRef provenance type
-   _(bc-models)_ Add commodity display metadata
-   _(bc-models)_ Add commodity aliases
-   _(bc-models)_ Effective tag union helper
-   Balance resolution and reconcile gate
-   _(bc-models)_ Allow elided posting amount
-   _(bc-core)_ Add transaction extra_dates
-   Add transaction note field
-   _(bc-models)_ Add Balances accumulator
-   _(bc-models)_ Add Amount arithmetic + AmountError
-   _(bc-models)_ Snap effective date to grid boundary
-   _(bc-core)_ Reject duplicate revision effective dates
-   _(bc-models)_ Add revision timeline resolution
-   _(bc-models)_ Split budget into anchor + revision
-   _(bc-models)_ Add spread_from/spread_until to Posting
-   _(bc-models)_ Derive PartialEq for Budget types
-   _(bc-models)_ Add Budget domain types and RolloverPolicy
-   _(bc-models)_ Add BudgetWindow calendar-range type
-   _(models,core,cli)_ Milestone 5 — budgeting system
-   _(models)_ Add envelope_id to Posting
-   _(models)_ Add envelope budgeting types
-   _(models,core,cli)_ AU mortgage loan model
-   _(core)_ Add LoanService with amortization schedule
-   _(models)_ Add acquisition fields to Account
-   _(models)_ Add loan domain types
-   _(models)_ Add valuation domain types
-   _(bc-models)_ Add kind field to Account
-   _(bc-models)_ Add AccountKind enum
-   _(bc-models)_ Add FinancialQuarter period variant
-   _(bc-models)_ Re-export bon builder types
-   _(bc-models)_ Add Tag, TagId, and TagForest
-   _(bc-models)_ Add Commodity entity
-   _(bc-models)_ Add account hierarchy and TagPath
-   _(bc-core)_ Implement core engine
-   _(bc-logging)_ Add logging crate with OTel
-   _(bc-models)_ Add GlobalSettings
-   _(bc-models)_ Add Period enum
-   _(bc-models)_ Add Transaction and Posting types
-   _(bc-models)_ Add Account and AccountType
-   _(bc-models)_ Add CommodityCode and Amount
-   _(bc-models)_ Add typed ID newtypes using mti
-   Add workspace root and library crate stubs

### 🐛 Bug Fixes
-   _(bc-core)_ Make the discard report tell the truth
-   _(bc-core)_ Preserve provenance across edits
-   _(bc-app,bc-core,bc-ipc)_ Budget fixes
-   _(bc-core,bc-app)_ Budget drill-down and spread
-   _(bc-models)_ Drop unresolved intra-doc link
-   _(plugins)_ Complete plugin system implementation
-   _(models)_ Add archived_at() to Group and Envelope
-   Cleanup from PR#24 review
-   _(models)_ Lint — suffix sep + doc unwrap
-   _(models,core)_ Introduce DepreciationId newtype
-   Milestone 5A code review fixes
-   Address Copilot review findings
-   _(bc-models)_ Use drop() for destructor value
-   _(bc-models)_ Re-export PeriodBuildError
-   _(bc-models)_ Add AccountKind re-export early
-   _(bc-models)_ Fix Kind enum visibility
-   _(bc-models)_ Rename account type field back to account_type
-   _(bc-models)_ Resolve pre-existing clippy warnings
-   _(bc-models)_ Address quality review blockers
-   _(bc-models)_ Address code review findings
-   _(bc-models)_ Re-export CommodityId
-   _(bc-models)_ Wire thin ID re-exports
-   Resolve clippy warnings in test code
-   _(bc-core)_ Add DB transactions

### 🚜 Refactor
-   _(bc-models)_ Remove Link stub
-   _(bc-models)_ Require explicit decimals
-   Replace status with reconciliation
-   _(bc-models)_ Rename posting memo to note
-   Remove Envelope, use account-anchored Budget
-   _(plugins)_ Address PR review feedback
-   Address milestone-5 PR review feedback
-   _(milestone/1)_ Address final code review findings
-   _(bc-models)_ Privatise Amount fields
-   _(bc-models)_ Ergonomic builder defaults
-   _(bc-models)_ Finalise private modules
-   _(bc-models)_ Rewrite transaction.rs
-   _(bc-models)_ Rewrite Account with bon
-   _(bc-models)_ Rewrite period.rs
-   _(bc-models)_ Move define_id! to lib.rs

### 🎨 Styling
-   Move trait bounds into where clauses
-   _(budget)_ Apply rustfmt across migrated crates
-   Use item import granularity

### 📚 Documentation
-   Add rationale comments and clarify doc edge cases
-   _(bc-models)_ Add example to AccountKind doc comment
-   _(bc-models)_ Clarify start_day cap at 28
-   _(bc-models)_ Explain CommodityCode vs Commodity
-   _(bc-models)_ Expand Tag field docs + example
-   _(bc-models)_ Expand Commodity field docs
-   _(bc-models)_ Fix Posting and Link docs
-   _(bc-models)_ Expand transaction field docs
-   _(bc-models)_ Fix Account field docs
-   _(bc-models)_ Account field docs + example
-   _(bc-models)_ Add TagBuilder to tag module inventory

### 🧪 Testing
-   _(bc-models)_ Cover sub_unchecked
-   _(bc-models)_ Hoist imports, add archive test
-   Add missing coverage for events, settings, models
-   _(bc-models)_ Cover pre-FY date in FinancialQuarter
-   _(bc-models)_ Add more id tests
-   Add trivial tests to all crates

### ⚙️ Miscellaneous Tasks
-   Anonymise sample bank/employer names
-   Improve typed id tests
-   Add linting and formatting tools

