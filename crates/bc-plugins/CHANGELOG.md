# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- markdownlint-disable -->
## [0.1.0](https://github.com/JP-Ellis/borrow-checker/releases/tag/bc-plugins/v0.1.0) - _2026-08-02_

### 🚀 Features
-   _(bc-core)_ Add Importer config validation
-   _(bc-sdk)_ Carry source location for diagnostics
-   _(bc-plugins)_ Add ipc-gated PluginInfo conversion
-   _(plugins)_ Drop TOML sidecar, probe WASM
-   _(plugins)_ Logger WIT import and host impl
-   _(plugins)_ Implement bc-plugins wasmtime host
-   Add workspace root and library crate stubs

### 🐛 Bug Fixes
-   _(bc-core)_ Require Importer::validate explicitly
-   _(bc-plugins)_ Guard unset documents_root
-   Address review findings on PR #131
-   _(beancount)_ Restore multi-commodity warning
-   _(plugins)_ TryFrom in translate.rs
-   _(plugins)_ Narrow clippy expect on bindgen module
-   _(sdk)_ Doc examples and #[inline] on HostCtx
-   _(plugins)_ Harden plugin host
-   _(plugins)_ Complete plugin system implementation
-   _(plugins)_ Preserve BadValue variant and add inline

### 🚜 Refactor
-   Parse(config) sourcing ABI, drop detect
-   Require at least one posting per raw tx
-   Multi-posting raw transaction model
-   Address second-round review comments
-   _(wit)_ Rename world to 'borrow-checker'
-   _(plugins)_ Address PR review feedback

### 📚 Documentation
-   Describe validate as it actually behaves

### 🧪 Testing
-   _(bc-plugins)_ Pin validate at the host boundary
-   _(bc-sdk)_ Cover source-location line/row math
-   _(bc-plugins)_ Source files via preopen
-   _(bc-plugins)_ Supply account in csv fixture
-   Add trivial tests to all crates

### ⚙️ Miscellaneous Tasks
-   Add cargo-machete to CI and pre-commit hook
-   Add linting and formatting tools

