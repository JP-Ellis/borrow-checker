# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- markdownlint-disable -->
## [0.1.0](https://github.com/JP-Ellis/borrow-checker/releases/tag/bc-config/v0.1.0) - _2026-05-04_

### 🚀 Features
-   _(cli)_ URL install, move plugin dir to config
-   _(config)_ Add debug tracing to config loading
-   _(cli)_ Integrate plugin registry
-   _(bc-config)_ Use XDG + native config dir hierarchy
-   _(bc-config)_ Add Settings from config

### 🐛 Bug Fixes
-   _(plugins)_ Complete plugin system implementation
-   _(ci)_ Resolve clippy and Windows test failures
-   _(bc-models)_ Re-export PeriodBuildError

### 🚜 Refactor
-   Address second-round review comments
-   _(cli)_ Consolidate db_path into Settings
-   _(plugins)_ Address PR review feedback
-   _(config,cli)_ Move db-path to bc-config
-   _(milestone/1)_ Address final code review findings

### ⚙️ Miscellaneous Tasks
-   Resolve new lints

