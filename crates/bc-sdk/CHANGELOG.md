# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- markdownlint-disable -->
## [0.1.0](https://github.com/JP-Ellis/borrow-checker/releases/tag/bc-sdk/v0.1.0) - _2026-08-02_

### 🚀 Features
-   _(bc-sdk)_ Add importer validate hook
-   _(bc-sdk)_ Carry source location for diagnostics
-   _(plugins)_ Logger WIT import and host impl
-   _(sdk)_ Add bc-sdk-macros #[importer] macro
-   _(sdk)_ Add bc-sdk with WIT interface and guest bindings
-   Add workspace root and library crate stubs

### 🐛 Bug Fixes
-   _(csv)_ Polish review nits from headerless CSV
-   _(sdk)_ No-op plugin logs off-wasm
-   _(bc-sdk)_ Export RawPosting from crate root
-   _(sdk)_ Doc examples and #[inline] on HostCtx
-   _(plugins)_ Harden plugin host
-   _(plugins)_ Complete plugin system implementation
-   _(sdk)_ Pub_export_macro and RawTransaction test

### 🚜 Refactor
-   _(bc-sdk)_ Require Importer::validate explicitly
-   Parse(config) sourcing ABI, drop detect
-   Require at least one posting per raw tx
-   Multi-posting raw transaction model
-   _(wit)_ Rename world to 'borrow-checker'
-   _(plugins)_ Address PR review feedback

### 🎨 Styling
-   Move trait bounds into where clauses

### 📚 Documentation
-   Describe validate as it actually behaves
-   Parallel-trait invariants, fix doctest lambdas

### 🧪 Testing
-   Add trivial tests to all crates

### ⚙️ Miscellaneous Tasks
-   Housekeeping for bc-sdk and bc-sdk-macros
-   Add linting and formatting tools

