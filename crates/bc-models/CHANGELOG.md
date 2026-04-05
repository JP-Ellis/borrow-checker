# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- markdownlint-disable -->
## [0.1.0](https://github.com/JP-Ellis/borrow-checker/releases/tag/bc-models/v0.1.0) - _2026-04-05_

### 🚀 Features
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
-   Add missing coverage for events, settings, models
-   _(bc-models)_ Cover pre-FY date in FinancialQuarter
-   _(bc-models)_ Add more id tests
-   Add trivial tests to all crates

### ⚙️ Miscellaneous Tasks
-   Improve typed id tests
-   Add linting and formatting tools

