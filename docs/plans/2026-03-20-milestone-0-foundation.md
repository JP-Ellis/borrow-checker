# Milestone 0: Project Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the Cargo workspace skeleton, all placeholder crates, developer tooling, community files, CI pipeline, and release automation so every subsequent milestone has a clean, working foundation to build on.

**Architecture:** Single Cargo workspace (`resolver = "2"`, edition 2024) with all crates pre-declared as members. Each crate starts as a minimal stub that compiles clean. Strict lint configuration via `[workspace.lints]` and `.clippy.toml`. CI mirrors the pattern used across JP-Ellis Rust projects: `complete` sentinel job, `prek`, `committed`, `clippy` (via `cargo-hack`), `format` (nightly), `test` (via `cargo-nextest` + `cargo-hack`), `coverage` (via `cargo-llvm-cov`).

**Tech Stack:** Rust stable/nightly (fmt only), Cargo workspaces, `cargo-hack`, `cargo-nextest`, `cargo-llvm-cov`, `prek`, `tombi`, `biome`, `committed`, `release-plz`, GitHub Actions

---

## File Map

```
borrow-checker/
├── Cargo.toml                              # workspace root + lints
├── Cargo.lock                              # committed (workspace has binaries)
├── rust-toolchain.toml                     # pin Rust channel
├── .clippy.toml                            # clippy configuration
├── .rustfmt.toml                           # rustfmt (nightly features)
├── .editorconfig                           # editor settings
├── .tombi.toml                             # tombi TOML formatter config
├── .yamlfix.toml                           # YAML fixer config
├── biome.json                              # Biome JSON/JSONC formatter config
├── committed.toml                          # commit message linting
├── prek.toml                               # pre-commit hooks (prek format)
├── release-plz.toml                        # release automation config
├── .gitignore                              # comprehensive Rust + OS template
├── LICENSE                                 # MIT
├── CONTRIBUTING.md                         # fork/PR workflow
├── CODE_OF_CONDUCT.md                      # Contributor Covenant v2.1
├── CHANGELOG.md                            # bootstrapped; managed by release-plz
├── .github/
│   └── workflows/
│       ├── ci.yml                          # test + lint + coverage
│       └── release-plz.yml                 # automated releases
├── .config/
│   └── nextest.toml                        # cargo-nextest CI profile
├── docs/
│   ├── DESIGN.md                           # (already exists)
│   └── plans/
│       └── 2026-03-20-milestone-0-foundation.md  # this file
├── plugins/
│   └── .gitkeep                            # placeholder for first-party plugins
└── crates/
    ├── bc-models/
    │   ├── Cargo.toml
    │   └── src/lib.rs
    ├── bc-core/
    │   ├── Cargo.toml
    │   └── src/lib.rs
    ├── bc-format-csv/
    │   ├── Cargo.toml
    │   └── src/lib.rs
    ├── bc-format-ledger/
    │   ├── Cargo.toml
    │   └── src/lib.rs
    ├── bc-format-beancount/
    │   ├── Cargo.toml
    │   └── src/lib.rs
    ├── bc-format-ofx/
    │   ├── Cargo.toml
    │   └── src/lib.rs
    ├── bc-plugins/
    │   ├── Cargo.toml
    │   └── src/lib.rs
    ├── bc-sdk/
    │   ├── Cargo.toml
    │   └── src/lib.rs
    ├── bc-cli/
    │   ├── Cargo.toml
    │   └── src/main.rs
    ├── bc-tui/
    │   ├── Cargo.toml
    │   └── src/main.rs
    └── bc-app/
        ├── Cargo.toml
        └── src/main.rs
```

---

## Task 1: Workspace Root

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`

- [ ] **Step 1: Create the workspace `Cargo.toml`**

The lints section follows the opt-out approach: all clippy groups are enabled at `warn`, then individual lints that don't fit the project are explicitly allowed.

```toml
#:schema https://json.schemastore.org/cargo.json

[workspace]
resolver = "2"
members  = [
  "crates/bc-models",
  "crates/bc-core",
  "crates/bc-format-csv",
  "crates/bc-format-ledger",
  "crates/bc-format-beancount",
  "crates/bc-format-ofx",
  "crates/bc-plugins",
  "crates/bc-sdk",
  "crates/bc-cli",
  "crates/bc-tui",
  "crates/bc-app",
]

[workspace.package]
version      = "0.1.0"
edition      = "2024"
authors      = ["Joshua Ellis <josh@jpellis.me>"]
license      = "MIT"
repository   = "https://github.com/JP-Ellis/borrow-checker"
homepage     = "https://github.com/JP-Ellis/borrow-checker"
rust-version = "1.85"

[workspace.dependencies]
# Internal crates — reference as `bc-models.workspace = true` in member Cargo.tomls
bc-models           = { path = "crates/bc-models" }
bc-core             = { path = "crates/bc-core" }
bc-format-csv       = { path = "crates/bc-format-csv" }
bc-format-ledger    = { path = "crates/bc-format-ledger" }
bc-format-beancount = { path = "crates/bc-format-beancount" }
bc-format-ofx       = { path = "crates/bc-format-ofx" }
bc-plugins          = { path = "crates/bc-plugins" }
bc-sdk              = { path = "crates/bc-sdk" }

################################################################################
## Lints
################################################################################
# Opt-out approach: all groups are enabled at warn, then individual lints that
# don't fit the project are explicitly allowed. Add per-crate overrides inside
# the crate's own [lints] table using `lints.workspace = true`.
[workspace.lints.rust]
future-incompatible = "warn"
missing_docs        = "warn"
warnings            = "warn"

[workspace.lints.rustdoc]
missing-crate-level-docs = "warn"

[workspace.lints.clippy]
# Enable all groups at lower priority so individual lints can override
cargo       = { level = "warn", priority = -1 }
complexity  = { level = "warn", priority = -1 }
correctness = { level = "warn", priority = -1 }
pedantic    = { level = "warn", priority = -1 }
perf        = { level = "warn", priority = -1 }
restriction = { level = "warn", priority = -1 }
style       = { level = "warn", priority = -1 }
suspicious  = { level = "warn", priority = -1 }

# Restriction lints: opt-in group; silence the meta-lint since we're using it
blanket-clippy-restriction-lints = "allow"

# Personal style preferences
# Stubs: readme/keywords/categories added before publishing each crate
cargo_common_metadata          = "allow"
# String-handling crates (bc-format-*, bc-core, bc-models) need non-ASCII.
# Note: per-crate override is impossible when using `[lints] workspace = true`
# (Cargo prohibits mixing the two), so this must live at the workspace level.
non_ascii_literal              = "allow"
arbitrary_source_item_ordering = "allow"
cognitive_complexity           = "allow"
else_if_without_else           = "allow"
impl_trait_in_params           = "allow"
implicit_return                = "allow"
min_ident_chars                = "allow"
missing_trait_methods          = "allow"
pattern_type_mismatch          = "allow"
pub_with_shorthand             = "allow"
question_mark_used             = "allow"
ref_patterns                   = "allow"
self_named_module_files        = "allow"
separated_literal_suffix       = "allow"
single_call_fn                 = "allow"
single_char_lifetime_names     = "allow"
unreachable                    = "allow"

# Known upstream issues — revisit when fixed
# https://github.com/rust-lang/rust-clippy/issues/14056
panic_in_result_fn = "allow"
# https://github.com/rust-lang/rust-clippy/issues/15111
return_and_then = "allow"
```

- [ ] **Step 2: Create `rust-toolchain.toml`**

```toml
[toolchain]
channel = "stable"
```

- [ ] **Step 3: Verify TOML parses**

```bash
cargo metadata --no-deps 2>&1 | head -3 || true
```

Expected: error about missing crate directories (not a parse error) — exit code is non-zero here, which is fine. Move on.

---

## Task 2: Library Crate Stubs

Create `bc-models`, `bc-core`, `bc-plugins`, `bc-sdk`.

**Files:** `crates/{bc-models,bc-core,bc-plugins,bc-sdk}/Cargo.toml` + `src/lib.rs`

- [ ] **Step 1: Create `crates/bc-models/Cargo.toml`**

```toml
#:schema https://json.schemastore.org/cargo.json

[package]
name        = "bc-models"
description = "Shared domain types for BorrowChecker"

version.workspace      = true
edition.workspace      = true
authors.workspace      = true
license.workspace      = true
repository.workspace   = true
rust-version.workspace = true

[lints]
workspace = true
```

- [ ] **Step 2: Create `crates/bc-models/src/lib.rs`**

```rust
//! Shared domain types for BorrowChecker.
//!
//! This crate defines the vocabulary used across the entire workspace:
//! accounts, transactions, envelopes, and their typed ID wrappers.
//! It has no internal BorrowChecker dependencies.
```

- [ ] **Step 3: Create `crates/bc-core/Cargo.toml`**

```toml
#:schema https://json.schemastore.org/cargo.json

[package]
name        = "bc-core"
description = "BorrowChecker core engine: event log, SQLite projections, business logic"

version.workspace      = true
edition.workspace      = true
authors.workspace      = true
license.workspace      = true
repository.workspace   = true
rust-version.workspace = true

[dependencies]
bc-models = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 4: Create `crates/bc-core/src/lib.rs`**

```rust
//! BorrowChecker core engine.
//!
//! Owns the append-only event log, `SQLite` read projections, double-entry
//! accounting enforcement, import profiles, and the budgeting model.
```

- [ ] **Step 5: Create `crates/bc-plugins/Cargo.toml`**

```toml
#:schema https://json.schemastore.org/cargo.json

[package]
name        = "bc-plugins"
description = "WASM host runtime and plugin ABI bridge for BorrowChecker"

version.workspace      = true
edition.workspace      = true
authors.workspace      = true
license.workspace      = true
repository.workspace   = true
rust-version.workspace = true

[dependencies]
bc-core   = { workspace = true }
bc-models = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 6: Create `crates/bc-plugins/src/lib.rs`**

```rust
//! WASM host runtime and plugin ABI bridge.
//!
//! Loads `.wasm` plugin files via extism, wires up host functions,
//! and bridges plugin calls into `bc-core`.
```

- [ ] **Step 7: Create `crates/bc-sdk/Cargo.toml`**

```toml
#:schema https://json.schemastore.org/cargo.json

[package]
name        = "bc-sdk"
description = "Plugin author SDK for BorrowChecker — compile to wasm32-wasip1"

version.workspace      = true
edition.workspace      = true
authors.workspace      = true
license.workspace      = true
repository.workspace   = true
rust-version.workspace = true

# bc-sdk has NO internal workspace dependencies.
# Plugin authors depend on this crate alone, not the full workspace.

[lints]
workspace = true
```

- [ ] **Step 8: Create `crates/bc-sdk/src/lib.rs`**

```rust
//! BorrowChecker Plugin SDK.
//!
//! Plugin authors depend on this crate, compile to `wasm32-wasip1`,
//! and distribute a single `.wasm` file.
//!
//! This crate has no dependencies on the rest of the BorrowChecker workspace.
```

- [ ] **Step 9: Verify the four library crates build**

```bash
cargo build -p bc-models -p bc-core -p bc-plugins -p bc-sdk
```

Expected: compiles cleanly, no warnings

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml rust-toolchain.toml crates/bc-models crates/bc-core crates/bc-plugins crates/bc-sdk
git commit -m "feat: add workspace root and library crate stubs"
```

---

## Task 3: Format Crate Stubs

**Files:** `crates/bc-format-{csv,ledger,beancount,ofx}/Cargo.toml` + `src/lib.rs`

- [ ] **Step 1: Create `crates/bc-format-csv/Cargo.toml`**

```toml
#:schema https://json.schemastore.org/cargo.json

[package]
name        = "bc-format-csv"
description = "CSV import for BorrowChecker (configurable column mapping)"

version.workspace      = true
edition.workspace      = true
authors.workspace      = true
license.workspace      = true
repository.workspace   = true
rust-version.workspace = true

[dependencies]
bc-core   = { workspace = true }
bc-models = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Create `crates/bc-format-csv/src/lib.rs`**

```rust
//! CSV import for BorrowChecker.
//!
//! Implements the `Importer` trait for delimited text files with
//! configurable column mapping. Works with exports from most banks.
```

- [ ] **Step 3: Create `crates/bc-format-ledger/Cargo.toml`**

```toml
#:schema https://json.schemastore.org/cargo.json

[package]
name        = "bc-format-ledger"
description = "Ledger file read/write for BorrowChecker"

version.workspace      = true
edition.workspace      = true
authors.workspace      = true
license.workspace      = true
repository.workspace   = true
rust-version.workspace = true

[dependencies]
bc-core   = { workspace = true }
bc-models = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 4: Create `crates/bc-format-ledger/src/lib.rs`**

```rust
//! Ledger-format read and write for BorrowChecker.
//!
//! Full round-trip compatibility with `.ledger` files
//! as used by the ledger-cli tool.
```

- [ ] **Step 5: Create `crates/bc-format-beancount/Cargo.toml`**

```toml
#:schema https://json.schemastore.org/cargo.json

[package]
name        = "bc-format-beancount"
description = "Beancount file read/write for BorrowChecker"

version.workspace      = true
edition.workspace      = true
authors.workspace      = true
license.workspace      = true
repository.workspace   = true
rust-version.workspace = true

[dependencies]
bc-core   = { workspace = true }
bc-models = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 6: Create `crates/bc-format-beancount/src/lib.rs`**

```rust
//! Beancount-format read and write for BorrowChecker.
//!
//! Full round-trip compatibility with `.beancount` files.
```

- [ ] **Step 7: Create `crates/bc-format-ofx/Cargo.toml`**

```toml
#:schema https://json.schemastore.org/cargo.json

[package]
name        = "bc-format-ofx"
description = "OFX/QFX import for BorrowChecker"

version.workspace      = true
edition.workspace      = true
authors.workspace      = true
license.workspace      = true
repository.workspace   = true
rust-version.workspace = true

[dependencies]
bc-core   = { workspace = true }
bc-models = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 8: Create `crates/bc-format-ofx/src/lib.rs`**

```rust
//! OFX/QFX import for BorrowChecker.
//!
//! Parses Open Financial Exchange (OFX) and Quicken (QFX) files,
//! the standard export format for most US banks and brokerages.
```

- [ ] **Step 9: Build and commit**

```bash
cargo build -p bc-format-csv -p bc-format-ledger -p bc-format-beancount -p bc-format-ofx
git add crates/bc-format-csv crates/bc-format-ledger crates/bc-format-beancount crates/bc-format-ofx
git commit -m "feat: add format crate stubs (csv, ledger, beancount, ofx)"
```

---

## Task 4: Binary Crate Stubs

Full Tauri wiring (tauri-build, build.rs, tauri.conf.json, frontend) is deferred to Milestone 7. `bc-app` starts as a plain binary so the workspace compiles without Tauri system dependencies.

**Files:** `crates/{bc-cli,bc-tui,bc-app}/Cargo.toml` + `src/main.rs`

- [ ] **Step 1: Create `crates/bc-cli/Cargo.toml`**

```toml
#:schema https://json.schemastore.org/cargo.json

[package]
name        = "bc-cli"
description = "BorrowChecker command-line interface"

version.workspace      = true
edition.workspace      = true
authors.workspace      = true
license.workspace      = true
repository.workspace   = true
rust-version.workspace = true

publish = false   # distributed as binary, not via crates.io

[dependencies]
bc-core   = { workspace = true }
bc-models = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Create `crates/bc-cli/src/main.rs`**

```rust
//! BorrowChecker CLI — commands for scripting, automation, and power users.

fn main() {
    println!("BorrowChecker CLI - not yet implemented");
}
```

- [ ] **Step 3: Create `crates/bc-tui/Cargo.toml`**

```toml
#:schema https://json.schemastore.org/cargo.json

[package]
name        = "bc-tui"
description = "BorrowChecker terminal user interface"

version.workspace      = true
edition.workspace      = true
authors.workspace      = true
license.workspace      = true
repository.workspace   = true
rust-version.workspace = true

publish = false

[dependencies]
bc-core   = { workspace = true }
bc-models = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 4: Create `crates/bc-tui/src/main.rs`**

```rust
//! BorrowChecker TUI — keyboard-first terminal interface built with ratatui.

fn main() {
    println!("BorrowChecker TUI - not yet implemented");
}
```

- [ ] **Step 5: Create `crates/bc-app/Cargo.toml`**

```toml
#:schema https://json.schemastore.org/cargo.json

[package]
name        = "bc-app"
description = "BorrowChecker desktop GUI (Tauri)"

version.workspace      = true
edition.workspace      = true
authors.workspace      = true
license.workspace      = true
repository.workspace   = true
rust-version.workspace = true

publish = false

# Full Tauri wiring (tauri-build, build.rs, tauri.conf.json, frontend) added in Milestone 7.
# Plain binary here so the workspace compiles without Tauri system dependencies.

[dependencies]
bc-core   = { workspace = true }
bc-models = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 6: Create `crates/bc-app/src/main.rs`**

```rust
//! BorrowChecker desktop GUI — Tauri application.
//!
//! Full Tauri setup (tauri.conf.json, build.rs, frontend) added in Milestone 7.

fn main() {
    println!("BorrowChecker GUI - not yet implemented");
}
```

- [ ] **Step 7: Verify full workspace builds clean**

```bash
cargo build --workspace
```

Expected: all 11 crates compile, zero errors, zero warnings

- [ ] **Step 8: Verify clippy is clean**

```bash
cargo clippy --workspace -- -D warnings
```

Expected: zero warnings (the stub lib.rs files are just doc comments, so this will pass)

- [ ] **Step 9: Commit**

```bash
git add crates/bc-cli crates/bc-tui crates/bc-app
git commit -m "feat: add binary crate stubs (cli, tui, app)"
```

---

## Task 5: Developer Config Files

These files configure the tooling used across the workspace. Most live at the repo root; `nextest.toml` goes in `.config/` since nextest supports it.

**Files:** `.clippy.toml`, `.rustfmt.toml`, `.editorconfig`, `.tombi.toml`, `.yamlfix.toml`, `biome.json`, `committed.toml`, `.config/nextest.toml`

- [ ] **Step 1: Create `.clippy.toml`**

```toml
#:schema https://json.schemastore.org/any.json
#:tombi schema.strict = false

# ".." expands to Clippy's built-in defaults; BorrowChecker added on top
doc-valid-idents = ["..", "BorrowChecker"]

# Clippy's default only allows 2 segments; 3 is fine before requiring `use`
absolute-paths-max-segments = 3

# Relax in test code
allow-expect-in-tests = true
allow-panic-in-tests  = true
allow-print-in-tests  = true
allow-unwrap-in-tests = false

# Prefer macros from the `pretty_assertions` crate
disallowed-macros = [
  "std::assert_eq",
  "std::assert_matches::assert_matches",
  "std::assert_matches::debug_assert_matches",
  "std::assert_ne",
]

# Prefer semi-colons inside blocks _except_ for single-line blocks
semicolon-inside-block-ignore-singleline  = true
semicolon-outside-block-ignore-multiline  = true

# We split `impl` across modules; one file with multiple impls is fine
inherent-impl-lint-scope = "module"
```

- [ ] **Step 2: Create `.rustfmt.toml`**

> **Note:** The `unstable_features` options only apply when using `cargo +nightly fmt`. The CI format job uses the nightly toolchain specifically for this. On stable, `cargo fmt` ignores unknown options.

```toml
#:schema https://www.schemastore.org/rustfmt.json
# Used by `cargo +nightly fmt`; unknown options are ignored on stable.

unstable_features      = true
group_imports          = "StdExternalCrate"
imports_granularity    = "Crate"
reorder_imports        = true
```

- [ ] **Step 3: Create `.editorconfig`**

```ini
# EditorConfig: https://EditorConfig.org
root = true

[*]
charset                  = utf-8
end_of_line              = lf
indent_size              = 2
indent_style             = space
insert_final_newline     = true
trim_trailing_whitespace = true

[*.md]
indent_size = 4

[*.rs]
indent_size = 4
```

- [ ] **Step 4: Create `.tombi.toml`**

```toml
toml-version = "v1.1.0"

[format]
  [format.rules]
  indent-sub-tables = true
```

- [ ] **Step 5: Create `.yamlfix.toml`**

```toml
#:schema https://www.schemastore.org/any.json
line_length        = 100
section_whitelines = 1
sequence_style     = "block_style"
whitelines         = 1
```

- [ ] **Step 6: Create `biome.json`**

Biome handles JSON and JSONC formatting. This project is Rust-first, so JS/TS linting is disabled — only JSON formatting is enabled.

```json
{
  "$schema": "https://biomejs.dev/schemas/1.9.4/schema.json",
  "files": {
    "include": ["**/*.json", "**/*.jsonc"]
  },
  "formatter": {
    "enabled": true,
    "indentStyle": "space",
    "indentWidth": 2,
    "lineWidth": 100
  },
  "json": {
    "formatter": {
      "enabled": true,
      "trailingCommas": "none"
    }
  },
  "linter": {
    "enabled": false
  },
  "javascript": {
    "formatter": {
      "enabled": false
    }
  }
}
```

- [ ] **Step 7: Create `committed.toml`**

```toml
#:schema https://raw.githubusercontent.com/crate-ci/committed/refs/heads/master/config.schema.json

style               = "conventional"
line_length         = 80
merge_commit        = false
no_fixup            = false
subject_capitalized = false
allowed_types = [
  "chore",
  "docs",
  "feat",
  "fix",
  "perf",
  "refactor",
  "revert",
  "style",
  "test",
]
# Ignore bot authors (e.g. GitHub Actions, Renovate)
ignore_author_re = "(?i)^.*\\[bot\\] <.*>$"
```

- [ ] **Step 8: Create `.config/nextest.toml`**

```toml
[profile.ci]
# Run all tests instead of failing fast on the first failure
fail-fast = false

# Terminate slow tests after 5 × 60 s
slow-timeout = { period = "60s", terminate-after = 5 }

# Emit JUnit XML for Codecov test results upload
junit = { path = "junit.xml" }
```

- [ ] **Step 9: Verify tombi formats the TOML config files**

```bash
# Install tombi if not already installed: cargo install tombi-cli
tombi format .tombi.toml committed.toml .yamlfix.toml
```

Expected: files formatted (no output, or "formatted N files")

- [ ] **Step 10: Commit**

```bash
git add .clippy.toml .rustfmt.toml .editorconfig .tombi.toml .yamlfix.toml biome.json committed.toml .config/nextest.toml
git commit -m "chore: add clippy, rustfmt, editorconfig, tombi, biome, nextest configs"
```

---

## Task 6: prek.toml

`prek` is the TOML-based pre-commit tool (modern replacement for `.pre-commit-config.yaml`). Run on every commit and in CI.

Uses:
- `pre-commit-hooks` for generic file checks
- `yamlfix` for YAML formatting
- `biome` for JSON
- `tombi` for TOML (replacing taplo)
- `mdformat` for Markdown (replacing markdownlint)
- `committed` for commit message style
- `typos` for spell checking
- `shellcheck` + `shfmt` for shell scripts
- Local hooks for Rust (cargo check, clippy, fmt)

**Files:**
- Create: `prek.toml`

- [ ] **Step 1: Create `prek.toml`**

```toml
#:schema https://raw.githubusercontent.com/j178/prek/refs/heads/master/prek.schema.json
#:tombi toml-version = "v1.1.0"

default_install_hook_types = ["commit-msg", "pre-commit", "pre-push"]
default_stages             = ["pre-commit"]

exclude = { glob = ["**/snapshots/**/*.snap"] }

# Generic file hygiene
[[repos]]
repo = "https://github.com/pre-commit/pre-commit-hooks"
rev  = "v5.0.0"
hooks = [
  { id = "check-added-large-files" },
  { id = "check-case-conflict" },
  { id = "check-executables-have-shebangs" },
  { id = "check-merge-conflict" },
  { id = "check-shebang-scripts-are-executable", exclude = "\\.rs$" },
  { id = "check-symlinks" },
  { id = "destroyed-symlinks" },
  { id = "detect-private-key" },
  { id = "end-of-file-fixer" },
  { id = "fix-byte-order-marker" },
  { id = "mixed-line-ending" },
  { id = "trailing-whitespace" },
  # Syntax checks (check only, do not modify)
  { id = "check-json" },
  { id = "check-toml" },
  { id = "check-xml" },
  { id = "check-yaml" },
]

# YAML formatting
[[repos]]
repo  = "https://github.com/lyz-code/yamlfix/"
rev   = "1.19.1"
hooks = [{ id = "yamlfix", args = ["--config-file", ".yamlfix.toml"] }]

# JSON formatting (via Biome)
[[repos]]
repo  = "https://github.com/biomejs/pre-commit"
rev   = "v2.4.6"
hooks = [{ id = "biome-check" }]

# TOML formatting (tombi replaces taplo)
[[repos]]
repo  = "https://github.com/tombi-toml/tombi-pre-commit"
rev   = "v0.9.3"
hooks = [{ id = "tombi-format" }, { id = "tombi-lint" }]

# Commit message style
[[repos]]
repo  = "https://github.com/crate-ci/committed"
rev   = "v1.1.11"
hooks = [{ id = "committed" }]

# Markdown formatting (mdformat replaces markdownlint)
# CHANGELOG.md is excluded because it contains HTML comments managed by release-plz
# that mdformat would strip.
[[repos]]
repo  = "https://github.com/hukkin/mdformat"
rev   = "0.7.22"
hooks = [{ id = "mdformat", exclude = "^CHANGELOG\\.md$" }]

# Spell checking
[[repos]]
repo  = "https://github.com/crate-ci/typos"
rev   = "v1.29.10"
hooks = [{ id = "typos" }]

# Shell script linting
[[repos]]
repo  = "https://github.com/shellcheck-py/shellcheck-py"
rev   = "v0.10.0.1"
hooks = [{ id = "shellcheck", args = ["--external-sources"] }]

# Shell script formatting
[[repos]]
repo  = "https://github.com/scop/pre-commit-shfmt"
rev   = "v3.12.0-2"
hooks = [{ id = "shfmt" }]

# Rust local hooks
[[repos]]
repo = "local"
hooks = [
  {
    id             = "cargo-check",
    name           = "cargo check",
    entry          = "cargo check --",
    language       = "system",
    types          = ["rust"],
    pass_filenames = false,
  },
  {
    id             = "cargo-clippy",
    name           = "cargo clippy",
    entry          = "cargo clippy --",
    language       = "system",
    types          = ["rust"],
    pass_filenames = false,
  },
  {
    id             = "cargo-fmt",
    name           = "cargo fmt",
    entry          = "cargo fmt --",
    language       = "system",
    types          = ["rust"],
    pass_filenames = false,
  },
]
```

> **Rev pinning note:** The `rev` values above are correct as of early 2026. Before committing, run `prek autoupdate` to pull the latest revs.

- [ ] **Step 2: Install prek and update hook revs**

```bash
# Install prek if not present
cargo install prek   # or: pipx install prek

# Update all hook revs to latest
prek autoupdate
```

Expected: `prek.toml` updated with latest revs (or "Already up to date")

- [ ] **Step 3: Install the hooks**

```bash
prek install
```

Expected: hooks installed into `.git/hooks/`

- [ ] **Step 4: Run prek on all files to verify it passes**

```bash
prek run --all-files
```

Expected: all hooks pass. If any fail, fix the issues before committing.

- [ ] **Step 5: Commit**

```bash
git add prek.toml
git commit -m "chore: add prek.toml pre-commit hooks"
```

---

## Task 7: .gitignore + plugins placeholder

Uses the comprehensive template from github/gitignore, following the same pattern as other JP-Ellis Rust projects.

**Files:**
- Create: `.gitignore`
- Create: `plugins/.gitkeep`

- [ ] **Step 1: Create `.gitignore`**

```gitignore
################################################################################
## Project specific
################################################################################

/.env

################################################################################
## Standard Templates
## Sources: Rust, Archives, Backup, Emacs, JetBrains, Linux,
##          VisualStudioCode, Windows, macOS (from github/gitignore)
################################################################################

########################################
## Rust
########################################
# Build output
debug/
target/

# Cargo.lock is committed for workspaces with binaries
# (See: https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html)

# rustfmt backup files
**/*.rs.bk

# MSVC debug info
*.pdb

########################################
## Archives
########################################
*.7z *.jar *.rar *.zip *.gz *.gzip *.tgz *.bzip *.bzip2 *.bz2
*.xz *.lzma *.cab *.xar *.zst *.tzst *.iso *.tar
*.dmg *.xpi *.gem *.egg *.deb *.rpm *.msi *.msm *.msp *.txz

########################################
## Backup
########################################
*.bak *.gho *.ori *.orig *.tmp

########################################
## Emacs
########################################
*~
\#*\#
/.emacs.desktop
/.emacs.desktop.lock
*.elc
auto-save-list
tramp
.\#*
.org-id-locations
*_archive
*_flymake.*
/eshell/history
/eshell/lastdir
/elpa/
*.rel
/auto/
.cask/
dist/
flycheck_*.el
/server/
.projectile
.dir-locals.el
/network-security.data

########################################
## JetBrains
########################################
.idea/**/workspace.xml
.idea/**/tasks.xml
.idea/**/usage.statistics.xml
.idea/**/dictionaries
.idea/**/shelf
.idea/**/aws.xml
.idea/**/contentModel.xml
.idea/**/dataSources/
.idea/**/dataSources.ids
.idea/**/dataSources.local.xml
.idea/**/sqlDataSources.xml
.idea/**/dynamic.xml
.idea/**/uiDesigner.xml
.idea/**/dbnavigator.xml
.idea/**/gradle.xml
.idea/**/libraries
cmake-build-*/
.idea/**/mongoSettings.xml
*.iws
out/
.idea_modules/
atlassian-ide-plugin.xml
.idea/replstate.xml
.idea/sonarlint/
com_crashlytics_export_strings.xml
crashlytics.properties
crashlytics-build.properties
fabric.properties
.idea/httpRequests
.idea/caches/build_file_checksums.ser

########################################
## Linux
########################################
.fuse_hidden*
.directory
.Trash-*
.nfs*

########################################
## Patch
########################################
*.orig
*.rej

########################################
## Visual Studio Code
########################################
.vscode/*
!.vscode/settings.json
!.vscode/tasks.json
!.vscode/launch.json
!.vscode/extensions.json
!.vscode/*.code-snippets
.history/
*.vsix

########################################
## Windows
########################################
Thumbs.db
Thumbs.db:encryptable
ehthumbs.db
ehthumbs_vista.db
*.stackdump
[Dd]esktop.ini
$RECYCLE.BIN/
*.cab *.msi *.msix *.msm *.msp *.lnk

########################################
## macOS
########################################
.DS_Store
.AppleDouble
.LSOverride
._*
.DocumentRevisions-V100
.fseventsd
.Spotlight-V100
.TemporaryItems
.Trashes
.VolumeIcon.icns
.com.apple.timemachine.donotpresent
.AppleDB
.AppleDesktop
Network Trash Folder
Temporary Items
.apdisk

################################################################################
## Project-specific overrides
################################################################################
```

- [ ] **Step 2: Create `plugins/.gitkeep`**

The `plugins/` directory holds first-party bundled plugins (added in Milestone 6). The placeholder ensures the directory is tracked now.

```bash
mkdir -p plugins && touch plugins/.gitkeep
```

- [ ] **Step 3: Generate and commit `Cargo.lock`**

`Cargo.lock` is intentionally **not** gitignored — BorrowChecker contains binaries, so committing it ensures reproducible builds.

```bash
cargo generate-lockfile
git add .gitignore plugins/.gitkeep Cargo.lock
git commit -m "chore: add .gitignore, plugins/ placeholder, and Cargo.lock"
```

---

## Task 8: Community Files

**Files:** `LICENSE`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`

- [ ] **Step 1: Create `LICENSE`**

```
MIT License

Copyright (c) 2026 Joshua Ellis

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 2: Create `CONTRIBUTING.md`**

```markdown
# Contributing to BorrowChecker

Thank you for your interest in contributing!

## How to Contribute

1. Fork the repository on GitHub
2. Create a feature branch: `git checkout -b feat/my-feature`
3. Make your changes — see [Development Setup](#development-setup) below
4. Ensure all checks pass (see [Checks](#checks))
5. Commit using [Conventional Commits](https://www.conventionalcommits.org/) format
6. Open a pull request against `main`

## Development Setup

**Prerequisites:** Rust stable + nightly (for `cargo fmt`), `prek`, `cargo-nextest`, `cargo-hack`

```bash
git clone https://github.com/JP-Ellis/borrow-checker
cd borrow-checker
prek install           # install git hooks
cargo build --workspace
cargo nextest run --workspace
```

## Checks

```bash
cargo build --workspace                          # compile
cargo hack --feature-powerset clippy -- -D warnings  # lint
cargo +nightly fmt --check                       # format
cargo nextest run --workspace                    # test
```

## Commit Format

We use [Conventional Commits](https://www.conventionalcommits.org/):

| Type        | Purpose                         |
| ----------- | ------------------------------- |
| `feat:`     | new feature                     |
| `fix:`      | bug fix                         |
| `docs:`     | documentation only              |
| `chore:`    | maintenance (CI, deps, tooling) |
| `refactor:` | code change with no feature/fix |
| `perf:`     | performance improvement         |
| `test:`     | adding or updating tests        |
| `revert:`   | reverting a commit              |

## Plugin Development

To write a BorrowChecker plugin, see the `bc-sdk` crate documentation.
Plugins are compiled to `wasm32-wasip1` and distributed as `.wasm` files.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
```

- [ ] **Step 3: Create `CODE_OF_CONDUCT.md`**

Copy the full [Contributor Covenant v2.1](https://www.contributor-covenant.org/version/2/1/code_of_conduct/) text and save it as `CODE_OF_CONDUCT.md`. Replace `[INSERT CONTACT METHOD]` with a contact email.

- [ ] **Step 4: Commit**

```bash
git add LICENSE CONTRIBUTING.md CODE_OF_CONDUCT.md
git commit -m "docs: add license and community files"
```

---

## Task 9: CHANGELOG.md + release-plz.toml

`release-plz` manages the changelog on every release. Bootstrap the file with just the header; release-plz populates it. The `release-plz.toml` mirrors the pattern from other JP-Ellis projects.

**Files:** `CHANGELOG.md`, `release-plz.toml`

- [ ] **Step 1: Create `CHANGELOG.md`**

The header here matches the `header` field in `release-plz.toml` (Step 2). They must stay in sync.

```markdown
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- markdownlint-disable -->
```

- [ ] **Step 2: Create `release-plz.toml`**

```toml
# release-plz configuration
# See: https://release-plz.dev/docs/config

[workspace]
# Changelog
changelog_update = true

# Release/publish
git_release_enable  = true
git_tag_name        = "{{ package }}/v{{ version }}"
publish             = true
publish_allow_dirty = false

# Open release PRs as drafts so they can be reviewed before merging
pr_draft = true

# Binary crates: not published to crates.io, but DO get GitHub releases
# so that compiled binaries can be attached as release artifacts later.
[[package]]
name    = "bc-cli"
publish = false

[[package]]
name    = "bc-tui"
publish = false

[[package]]
name    = "bc-app"
publish = false

[changelog]
protect_breaking_commits = true
tag_pattern              = '^.*/v[0-9]+\.[0-9]+\.[0-9]+$'
commit_preprocessors     = [
  # Strip PR number appended by GitHub's merge button
  { pattern = '\s*\(#([0-9]+)\)$', replace = "" },
]
commit_parsers = [
  # Skip dependency-only chore/fix commits (e.g. from Renovate)
  { message = '^(chore|fix)\(deps.*\)', skip = true },
  # Group by type
  { group = "<!-- 00 -->🚀 Features",              message = "^feat" },
  { group = "<!-- 01 -->🐛 Bug Fixes",             message = "^fix" },
  { group = "<!-- 50 -->🚜 Refactor",              message = "^refactor" },
  { group = "<!-- 51 -->⚡ Performance",            message = "^perf" },
  { group = "<!-- 52 -->🎨 Styling",               message = "^style" },
  { group = "<!-- 60 -->📚 Documentation",          message = "^docs?" },
  { group = "<!-- 80 -->🧪 Testing",               message = "^test" },
  { group = "<!-- 98 -->◀️ Revert",                message = "^revert" },
  { group = "<!-- 99 -->⚙️ Miscellaneous Tasks",   message = "(^chore|.*)" },
]

header = """# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!-- markdownlint-disable -->
"""

body = """
## [{{ version | trim_start_matches(pat="v") }}]{% if release_link %}({{ release_link }}){% endif %} - _{{ timestamp | date(format="%Y-%m-%d") }}_
{% for group, commits in commits | group_by(attribute="group") %}
### {{ group | striptags | trim | upper_first }}
{% for commit in commits -%}
-   {% if commit.scope -%}_({{ commit.scope }})_ {% endif -%}
    {% if commit.breaking %}[**breaking**] {% endif -%}
    {{ commit.message | upper_first }}
{% endfor -%}
{% endfor %}
{% if github.contributors %}
### Contributors
{% for contributor in github.contributors %}
{% if not contributor.username or contributor.username is ending_with("[bot]") %}{% continue %}{% endif %}
-   @{{ contributor.username }}
{% endfor -%}
{% endif -%}
"""
```

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md release-plz.toml
git commit -m "chore: bootstrap CHANGELOG and add release-plz config"
```

---

## Task 10: GitHub Actions CI

Follows the pattern from `rust-skiplist` and `amber-api`:
- `complete` sentinel job (required checks gate)
- `committed` job (PR only)
- `prek` job (runs all pre-commit hooks, skipping local Rust hooks)
- `clippy` job (`cargo-hack --feature-powerset`)
- `format` job (nightly rustfmt)
- `test` job (`cargo-hack` + `cargo-nextest`, matrix: stable on 3 OS + beta on Linux)
- `coverage` job (`cargo-llvm-cov`)
- All action SHAs are pinned; update periodically with Dependabot/Renovate

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Create `.github/workflows/ci.yml`**

> **Before pinning:** The SHAs in the template below are placeholders matching known versions as of early 2026. After creating the file, run `prek run --all-files` — the `check-yaml` hook will verify syntax. To get up-to-date pinned SHAs, use `gh api /repos/{owner}/{repo}/commits/{tag}` or a tool like `pinact`.

```yaml
---
name: CI

permissions:
  contents: read

on:
  pull_request:
    branches:
      - main
    types:
      - opened
      - synchronize
      - reopened
      - ready_for_review
  push:
    branches:
      - main

env:
  RUST_BACKTRACE: '1'
  FORCE_COLOR: '1'
  NEXTEST_PROFILE: ci

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}

jobs:
  ##############################################################################
  ## Sentinel — required checks gate
  ##############################################################################
  complete:
    name: CI complete
    if: always()
    needs:
      - committed
      - prek
      - clippy
      - format
      - test
      - coverage
    runs-on: ubuntu-latest
    steps:
      - name: Failed
        run: exit 1
        if: >
          contains(needs.*.result, 'failure')
          || contains(needs.*.result, 'cancelled')

  ##############################################################################
  ## Commit message style (PRs only)
  ##############################################################################
  committed:
    name: Committed
    if: github.event_name == 'pull_request'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2
        with:
          fetch-depth: 0
      - uses: crate-ci/committed@faeed42f2e10c244533a01525f13c4d8b6ce383f  # v1.1.11
        with:
          args: --no-merge-commit

  ##############################################################################
  ## Pre-commit hooks (prek)
  ##############################################################################
  prek:
    name: Prek
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2

      - name: Install Rust (stable)
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt,clippy

      - name: Install prek
        uses: taiki-e/install-action@7a1c695e89f3bef9c0f0f2b88f9f6f8dd51da7c7  # v2.49.0
        with:
          tool: prek

      - name: Restore prek cache
        uses: actions/cache@5a3ec84eff668545956fd18022155c47e93e2684  # v4.2.3
        with:
          path: ~/.cache/prek
          key: prek-${{ runner.os }}-${{ hashFiles('prek.toml') }}

      - name: Run prek
        env:
          # Rust hooks run in the dedicated clippy/format jobs below
          SKIP: cargo-check,cargo-clippy,cargo-fmt
        run: prek run --all-files

  ##############################################################################
  ## Clippy
  ##############################################################################
  clippy:
    name: Clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2

      - name: Install Rust (stable)
        uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy

      - name: Restore Rust cache
        uses: Swatinem/rust-cache@9bdad043e88c75890e36ad3bbc8d27f0090dd609  # v2.7.8

      - name: Install cargo-hack
        uses: taiki-e/install-action@7a1c695e89f3bef9c0f0f2b88f9f6f8dd51da7c7  # v2.49.0
        with:
          tool: cargo-hack

      - name: Clippy (no dev deps)
        run: cargo hack --feature-powerset --no-dev-deps clippy --workspace -- -D warnings

      - name: Clippy (with dev deps)
        run: cargo hack --feature-powerset clippy --workspace --all-targets -- -D warnings

  ##############################################################################
  ## Format (nightly for unstable rustfmt options)
  ##############################################################################
  format:
    name: Format
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2

      - name: Install Rust (nightly)
        uses: dtolnay/rust-toolchain@nightly
        with:
          components: rustfmt

      - name: Check formatting
        run: cargo fmt --check

  ##############################################################################
  ## Test matrix
  ##############################################################################
  test:
    name: Test (${{ matrix.os }}, ${{ matrix.toolchain }})
    if: github.event.pull_request.draft == false || github.event_name != 'pull_request'
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            toolchain: stable
          - os: windows-latest
            toolchain: stable
          - os: macos-latest
            toolchain: stable
          - os: ubuntu-latest
            toolchain: beta
    runs-on: ${{ matrix.os }}
    continue-on-error: ${{ matrix.toolchain != 'stable' }}

    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2

      - name: Install Rust (${{ matrix.toolchain }})
        uses: dtolnay/rust-toolchain@master
        with:
          toolchain: ${{ matrix.toolchain }}

      - name: Restore Rust cache
        uses: Swatinem/rust-cache@9bdad043e88c75890e36ad3bbc8d27f0090dd609  # v2.7.8

      - name: Install cargo-hack and cargo-nextest
        uses: taiki-e/install-action@7a1c695e89f3bef9c0f0f2b88f9f6f8dd51da7c7  # v2.49.0
        with:
          tool: cargo-hack,cargo-nextest

      - name: Run tests
        run: cargo hack --feature-powerset nextest run --workspace

  ##############################################################################
  ## Coverage
  ##############################################################################
  coverage:
    name: Coverage
    if: github.event.pull_request.draft == false || github.event_name != 'pull_request'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2

      - name: Install Rust (stable)
        uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools-preview

      - name: Restore Rust cache
        uses: Swatinem/rust-cache@9bdad043e88c75890e36ad3bbc8d27f0090dd609  # v2.7.8

      - name: Install cargo-llvm-cov and cargo-nextest
        uses: taiki-e/install-action@7a1c695e89f3bef9c0f0f2b88f9f6f8dd51da7c7  # v2.49.0
        with:
          tool: cargo-llvm-cov,cargo-nextest

      - name: Generate coverage
        run: cargo llvm-cov nextest --all-features --lcov --output-path coverage.lcov

      - name: Upload coverage to Codecov
        uses: codecov/codecov-action@18283e04ce6e62d8b62fef9c5ad0abb3f4e2e1c0  # v5.4.3
        with:
          files: coverage.lcov
          token: ${{ secrets.CODECOV_TOKEN || '' }}

      - name: Upload test results to Codecov
        if: ${{ !cancelled() }}
        uses: codecov/test-results-action@47f0f3f4f4b70dc8e0d5a4a4af00dfe8e9f3c9d4  # v1.1.1
        with:
          token: ${{ secrets.CODECOV_TOKEN || '' }}
```

- [ ] **Step 2: Verify CI YAML parses**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" && echo "YAML valid"
```

Expected: `YAML valid`

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add GitHub Actions CI workflow"
```

---

## Task 11: release-plz GitHub Actions Workflow

The release-plz workflow uses a GitHub App token (not `GITHUB_TOKEN`) so that the release PR triggers CI. See [release-plz docs on GitHub App tokens](https://release-plz.dev/docs/github/token).

> **Pre-requisite:** Create a GitHub App for releases (or reuse an existing one) and add `RELEASE_APP_ID` (variable) and `RELEASE_APP_PRIVATE_KEY` (secret) to the repository. `CARGO_REGISTRY_TOKEN` is needed only when actually publishing to crates.io (can be added later).

**Files:**
- Create: `.github/workflows/release-plz.yml`

- [ ] **Step 1: Create `.github/workflows/release-plz.yml`**

```yaml
---
name: Release PLZ

permissions:
  contents: write
  id-token: write
  pull-requests: write

on:
  push:
    branches:
      - main

env:
  RUST_BACKTRACE: '1'
  FORCE_COLOR: '1'

jobs:
  release-pr:
    name: Release PR
    runs-on: ubuntu-latest
    # Only run on the upstream repo, not forks
    if: github.repository == 'JP-Ellis/borrow-checker'
    concurrency:
      group: release-plz-${{ github.ref }}
      cancel-in-progress: false

    steps:
      - name: Generate GitHub App token
        id: app-token
        uses: actions/create-github-app-token@df432ceedc7162793a195dd1713ff69aedc7558a  # v2.0.6
        with:
          app-id: ${{ vars.RELEASE_APP_ID }}
          private-key: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}

      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2
        with:
          fetch-depth: 0
          token: ${{ steps.app-token.outputs.token }}

      - name: Configure git identity
        env:
          GH_TOKEN: ${{ steps.app-token.outputs.token }}
        run: |
          git config user.name "${{ steps.app-token.outputs.app-slug }}[bot]"
          git config user.email "$(gh api "/users/${{ steps.app-token.outputs.app-slug }}[bot]" --jq '.id')+${{ steps.app-token.outputs.app-slug }}[bot]@users.noreply.github.com"

      - name: Install Rust (stable)
        uses: dtolnay/rust-toolchain@stable

      - name: Restore Rust cache
        uses: Swatinem/rust-cache@9bdad043e88c75890e36ad3bbc8d27f0090dd609  # v2.7.8

      - name: Run release-plz release-pr
        uses: release-plz/action@1528104d2ca23787631a1c1f022abb64b34c1e11  # v0.5.128
        with:
          command: release-pr
        env:
          GITHUB_TOKEN: ${{ steps.app-token.outputs.token }}

  release:
    name: Release
    runs-on: ubuntu-latest
    if: github.repository == 'JP-Ellis/borrow-checker'
    environment: crates-io
    concurrency:
      group: release-plz-${{ github.ref }}
      cancel-in-progress: false

    steps:
      - name: Authenticate with crates.io
        uses: rust-lang/crates-io-auth-action@b7e9a28eded4986ec6b1fa40eeee8f8f165559ec  # v1.0.3
        id: auth

      - name: Generate GitHub App token
        id: app-token
        uses: actions/create-github-app-token@df432ceedc7162793a195dd1713ff69aedc7558a  # v2.0.6
        with:
          app-id: ${{ vars.RELEASE_APP_ID }}
          private-key: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}

      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683  # v4.2.2
        with:
          fetch-depth: 0
          token: ${{ steps.app-token.outputs.token }}

      - name: Configure git identity
        env:
          GH_TOKEN: ${{ steps.app-token.outputs.token }}
        run: |
          git config user.name "${{ steps.app-token.outputs.app-slug }}[bot]"
          git config user.email "$(gh api "/users/${{ steps.app-token.outputs.app-slug }}[bot]" --jq '.id')+${{ steps.app-token.outputs.app-slug }}[bot]@users.noreply.github.com"

      - name: Install Rust (stable)
        uses: dtolnay/rust-toolchain@stable

      - name: Restore Rust cache
        uses: Swatinem/rust-cache@9bdad043e88c75890e36ad3bbc8d27f0090dd609  # v2.7.8

      - name: Run release-plz release
        uses: release-plz/action@1528104d2ca23787631a1c1f022abb64b34c1e11  # v0.5.128
        with:
          command: release
        env:
          GITHUB_TOKEN: ${{ steps.app-token.outputs.token }}
          CARGO_REGISTRY_TOKEN: ${{ steps.auth.outputs.token }}
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/release-plz.yml
git commit -m "ci: add release-plz workflow"
```

---

## Task 12: Final Verification

- [ ] **Step 1: Run the full local suite**

```bash
cargo build --workspace
```
Expected: all 11 crates compile, zero errors, zero warnings

```bash
cargo +nightly fmt --check
```
Expected: no output (already formatted)

```bash
cargo clippy --workspace -- -D warnings
```
Expected: zero warnings

```bash
# Install cargo-nextest if not present: cargo install cargo-nextest
cargo nextest run --workspace
```
Expected: `running 0 tests` per crate, all pass

- [ ] **Step 2: Run prek on all files**

```bash
prek run --all-files
```
Expected: all hooks pass

- [ ] **Step 3: Check git log**

```bash
git log --oneline
```

Expected (newest first) — the last commit (`chore: add spec and roadmap`) is a **pre-existing commit** from the brainstorming phase, not created by any task in this plan:

```
ci: add release-plz workflow
ci: add GitHub Actions CI workflow
chore: bootstrap CHANGELOG and add release-plz config
docs: add license and community files
chore: add .gitignore, plugins/ placeholder, and Cargo.lock
chore: add prek.toml pre-commit hooks
chore: add clippy, rustfmt, editorconfig, tombi, biome, nextest configs
feat: add binary crate stubs (cli, tui, app)
feat: add format crate stubs (csv, ledger, beancount, ofx)
feat: add workspace root and library crate stubs
chore: add spec and roadmap  ← pre-existing
```

- [ ] **Step 4: Push to GitHub and verify CI**

```bash
git push origin main
```

Open the GitHub Actions tab and verify all CI jobs pass: `complete`, `committed` (PR only), `prek`, `clippy`, `format`, `test` (4 matrix entries), `coverage`.

- [ ] **Step 5: Update ROADMAP.md**

Change `**Status:** ◻️` to `**Status:** ✅` for Milestone 0.

```bash
git add ROADMAP.md
git commit -m "chore: mark Milestone 0 complete"
git push origin main
```

---

## Checklist Summary

| Task | Description                                                                                                                                       | Status |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 1    | Workspace root (`Cargo.toml`, `rust-toolchain.toml`, comprehensive lints)                                                                         | ◻️      |
| 2    | Library crate stubs (bc-models, bc-core, bc-plugins, bc-sdk)                                                                                      | ◻️      |
| 3    | Format crate stubs (csv, ledger, beancount, ofx)                                                                                                  | ◻️      |
| 4    | Binary crate stubs (cli, tui, app)                                                                                                                | ◻️      |
| 5    | Developer config files (.clippy.toml, .rustfmt.toml, .editorconfig, .tombi.toml, .yamlfix.toml, biome.json, committed.toml, .config/nextest.toml) | ◻️      |
| 6    | prek.toml (pre-commit hooks with mdformat)                                                                                                        | ◻️      |
| 7    | .gitignore + plugins/.gitkeep + Cargo.lock                                                                                                        | ◻️      |
| 8    | Community files (LICENSE, CONTRIBUTING.md, CODE_OF_CONDUCT.md)                                                                                    | ◻️      |
| 9    | CHANGELOG.md + release-plz.toml                                                                                                                   | ◻️      |
| 10   | GitHub Actions CI workflow                                                                                                                        | ◻️      |
| 11   | release-plz GitHub Actions workflow                                                                                                               | ◻️      |
| 12   | Final verification + push + ROADMAP update                                                                                                        | ◻️      |
