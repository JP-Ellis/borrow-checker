# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- markdownlint-disable -->
## [0.1.0](https://github.com/JP-Ellis/borrow-checker/releases/tag/bc-format-beancount/v0.1.0) - _2026-04-05_

### 🚀 Features
-   _(bc-format-beancount)_ Add importer_factory
-   _(beancount)_ Add importer and exporter
-   Add format crate stubs
-   Add workspace root and library crate stubs

### 🐛 Bug Fixes
-   Address in-file CLAUDE: review comments
-   _(bc-format-beancount)_ Warn on multi-currency tx
-   _(format-crates)_ Swap/remove winnow dep
-   _(beancount)_ Flag parse; zero-posting test
-   _(beancount)_ Sort transactions by date in exporter

### 🚜 Refactor
-   Use module-local names, re-export with prefix

### 🧪 Testing
-   Add trivial tests to all crates

### ⚙️ Miscellaneous Tasks
-   Add cargo-machete to CI and pre-commit hook
-   Add linting and formatting tools

