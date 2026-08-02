# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- markdownlint-disable -->
## [0.1.0](https://github.com/JP-Ellis/borrow-checker/releases/tag/bc-config/v0.1.0) - _2026-08-02_

### 🚀 Features
-   _(bc-config)_ Add the pre-discard snapshot setting
-   _(bc-config)_ Add pre-import backup setting
-   _(bc-config)_ Add documents_root setting
-   _(bc-config)_ Persist [backup] section to config
-   _(bc-config)_ Add [backup] settings section
-   _(bc-config)_ Add ipc-gated SettingsInfo conversion
-   _(cli)_ URL install, move plugin dir to config
-   _(config)_ Add debug tracing to config loading
-   _(cli)_ Integrate plugin registry
-   _(bc-config)_ Use XDG + native config dir hierarchy
-   _(bc-config)_ Add Settings from config

### 🐛 Bug Fixes
-   _(bc-config)_ Persist cleared retain_count
-   Address PR review comments on settings page
-   _(bc-ui)_ Address review findings on PR #133
-   _(plugins)_ Complete plugin system implementation
-   _(ci)_ Resolve clippy and Windows test failures
-   _(bc-models)_ Re-export PeriodBuildError

### 🚜 Refactor
-   Address second-round review comments
-   _(cli)_ Consolidate db_path into Settings
-   _(plugins)_ Address PR review feedback
-   _(config,cli)_ Move db-path to bc-config
-   _(milestone/1)_ Address final code review findings

### 🧪 Testing
-   _(bc-config)_ Cover backup key removal

### ⚙️ Miscellaneous Tasks
-   _(ci)_ Switch to setup-rust-toolchain
-   Remove TOML sidecar generation
-   Resolve new lints

