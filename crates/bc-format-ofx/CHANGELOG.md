# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- markdownlint-disable -->
## [0.1.0](https://github.com/JP-Ellis/borrow-checker/releases/tag/bc-format-ofx/v0.1.0) - _2026-04-05_

### 🚀 Features
-   _(bc-format-ofx)_ Add importer_factory
-   _(ofx)_ Add importer for OFX/QFX files
-   Add format crate stubs
-   Add workspace root and library crate stubs

### 🐛 Bug Fixes
-   Address in-file CLAUDE: review comments
-   _(bc-format-ofx)_ Validate required fields in parser
-   _(ofx)_ Validate currency/account; tighten detect
-   _(ofx)_ Re-export OfxImporter from crate root

### 🚜 Refactor
-   Use module-local names, re-export with prefix

### 🧪 Testing
-   Add trivial tests to all crates

### ⚙️ Miscellaneous Tasks
-   Add cargo-machete to CI and pre-commit hook
-   Add linting and formatting tools

