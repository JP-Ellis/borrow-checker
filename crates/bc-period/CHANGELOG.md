# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- markdownlint-disable -->
## [0.1.0](https://github.com/JP-Ellis/borrow-checker/releases/tag/bc-period/v0.1.0) - _2026-11-15_

### 🐛 Bug Fixes
-   _(ipc)_ Carry the domain Period across IPC unchanged
-   _(period)_ Reject invalid periods at deserialisation

### 🚜 Refactor
-   _(period)_ Extract Period into the bc-period crate

### 🧪 Testing
-   _(period)_ Snapshot the serde wire shape of every Period variant

