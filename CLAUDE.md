# CLAUDE.md

This file provides guidance to AI agents when working with code in this repository.

## Commands

Tasks are run via `mise`. Key tasks:

```sh
mise run dev:app          # Hot-reload desktop app (Tauri + Trunk)
mise run test             # Unit tests + doc tests
mise run test:unit        # cargo hack feature-powerset nextest (all feature combos)
mise run test:docs        # cargo test --doc --workspace
mise run test:e2e         # WebdriverIO desktop E2E tests
mise run lint [--fix]     # Clippy on native + wasm32-unknown-unknown
mise run format [--fix]   # Check formatting (nightly rustfmt + leptosfmt)
mise run check [--fix]    # format + lint
mise run coverage         # LCOV report via cargo llvm-cov
```

Direct cargo equivalents:

```sh
cargo nextest run -p <crate>                                # single crate tests
cargo test --doc -p <crate>                                 # doc tests for one crate
cargo clippy --workspace --all-targets -- -D warnings       # native lint
cargo clippy -p bc-ipc -p bc-ui --target wasm32-unknown-unknown -- -D warnings  # WASM lint
cargo +nightly fmt                                          # format (nightly required)
leptosfmt crates/                                           # format Leptos view! macros
```

**bc-ui must be checked on `--target wasm32-unknown-unknown`**, not native. Many `web-sys`/`js-sys` APIs are absent on native, so a passing native check does not mean the UI crate compiles correctly.

## Workspace Layout

```text
crates/
  bc-models/       # Shared domain types — no I/O, no internal deps
  bc-config/       # Config file loading and directory resolution
  bc-core/         # Business logic: event log, SQLite projections, services
  bc-ipc/          # IPC message types shared between bc-app (native) and bc-ui (WASM)
  bc-app/          # Tauri 2 desktop wrapper (native binary)
  bc-cli/          # CLI binary (borrow-checker)
  bc-ui/           # Leptos WASM frontend (served in Tauri WebView)
  bc-plugins/      # Wasmtime host runtime for importer plugins
  bc-sdk/          # Plugin author SDK (compiles to wasm32-wasip2)
  bc-sdk-macros/   # Proc-macros for bc-sdk
  bc-otel/         # OpenTelemetry initialisation
plugins/           # First-party importer plugins (CSV, OFX, Ledger, Beancount)
e2e/               # WebdriverIO + tauri-driver desktop app tests
```

## Data Flow

```text
bc-models  ←─ referenced by everything
bc-config  ←─ bc-core, bc-app
bc-core    ←─ bc-app, bc-cli           (SQLite + event log)
bc-ipc     ←─ bc-app (native side) + bc-ui (WASM side)
bc-ipc     ─→ bc-models                (optional, `models` feature — native only)
bc-plugins ←─ bc-app                   (Wasmtime host)
bc-sdk     ←─ plugins/*                (compiled to wasm32-wasip2)
bc-ui      ←─ bc-app via Tauri WebView (compiled to wasm32-unknown-unknown)
```

`bc-models` defines all domain types (Account, Transaction, etc.) using a `define_id!` macro for typed ID newtypes. `bc-core` services own all business logic and talk to SQLite via `sqlx`. `bc-ipc` is the contract between Tauri commands (native) and Leptos (WASM) — keep it minimal and `serde`-serialisable.

### `bc-ipc` conversions and the `models` feature

DTO↔domain conversions live in the crate owning the non-IPC side, so they can be idiomatic `From`/`TryFrom` (the orphan rule forbids hosting them in `bc-app`). To keep the default (WASM) build of `bc-ipc` free of `bc-models`, the `bc-models`-facing impls are gated behind an optional `bc-ipc/models` feature; `bc-core`/`bc-config`/`bc-plugins` each gain an opt-in `ipc` feature for their own `From` impls into `bc-ipc`. `bc-ui` depends on `bc-ipc` with default features only, so the WASM bundle never pulls in `bc-models`.

Keep only **basic** conversions (scalar/enum/`Commodity`↔DTO) inside `bc-ipc` behind `models`. Presentation logic that walks the domain (account-path building, tag resolution, `Transaction`/`AccountNode` assembly) belongs in `bc-core` as extension traits (e.g. `AccountNodeExt`, `TransactionExt`, `AuditEntryExt`) — `bc-ipc` stays a thin contract, and the dependency graph stays acyclic (all arrows point toward `bc-ipc`).

## Lints

The workspace enables all Clippy groups at `warn` (priority = -1) and selectively allows exceptions. This means every public item needs a doc comment, `#[allow]` is banned in favour of `#[expect(lint, reason = "...")]`, `unwrap()` is disallowed in library code, and `clippy::module_name_repetitions` fires if you name a type after its module. Prefer naming types without the module prefix and re-exporting with an alias at the crate boundary.

## Testing Conventions

- Unit tests live in `#[cfg(test)] mod tests` alongside source.
- Integration tests are in `crates/$crate/tests/`.
- Use `pretty_assertions::assert_eq!` (not `std::assert_eq!`).
- Use `rstest` for parameterised tests and `insta` for snapshot assertions.
- Run a single test: `cargo nextest run -p <crate> <test-name>`.

## Commit Style

Conventional commits are enforced (`committed.toml`). Subject line ≤ 50 characters. Format: `type(scope): message` where scope is usually the crate short name (e.g. `bc-ui`, `bc-core`).

## Workflow

- **Copilot auto-reviews every PR** in this repo. Do not call `gh pr edit --add-reviewer copilot-pull-request-reviewer` — it will trigger automatically.
- **`docs/superpowers/`** is gitignored. Never `git add` or commit files under that path (specs and plans written there are ephemeral).
