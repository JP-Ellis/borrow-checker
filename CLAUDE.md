# CLAUDE.md

This file provides guidance to AI agents when working with code in this
repository.

## Commands

Tasks run through `mise` (`mise tasks` lists them): `test`, `lint`, `format`,
`coverage`, `dev:app`, `test:e2e`, `dev:web`, `test:web`, `test:qa`, `build:server`.

**Check `bc-ui` on `--target wasm32-unknown-unknown`.** Many `web-sys` and
`js-sys` APIs are absent on native, so a native pass proves nothing.

**Build `--release` before a bulk CLI run.** Every `borrow-checker` invocation
opens the database, applies the backup policy and checks migrations; under the
debug profile that cost dominates a loop over a hundred accounts.

## `bc-ipc` and the `models` feature

`bc-ipc` is the serde contract between the hosts (`bc-app`, `bc-server`) and
`bc-ui` (WASM). Command bodies live in `bc-service`; each host forwards to
`bc_service::dispatch`. DTO↔domain conversions live in the crate that owns the
non-IPC side, as
`From`/`TryFrom` (the orphan rule forbids hosting them in `bc-app`).
`bc-models`-facing impls sit behind the optional `bc-ipc/models` feature, and
`bc-core`, `bc-config`, `bc-plugins` and `bc-query` each gain an `ipc` feature
for theirs,
so the WASM bundle never pulls in `bc-models`.

Only scalar, enum and `Commodity` conversions belong in `bc-ipc`. Presentation
logic that walks the domain (account paths, tag resolution, `Transaction` and
`AccountNode` assembly) is a `bc-core` extension trait (`AccountNodeExt`,
`TransactionExt`, `AuditEntryExt`), which keeps every dependency arrow pointing
at `bc-ipc`.

## Code conventions

Clippy runs every group at `warn` and says what it wants. Two rules it does not
enforce:

- Hoist `use` to the top of the enclosing module, `mod tests` included, never
  inside a function body unless a name collision forces it.

- Keep test code out of coverage. Mark every `#[cfg(test)]` module with

  ```rust
  #[cfg_attr(coverage_nightly, coverage(off))]
  ```

  and every crate root with

  ```rust
  #![cfg_attr(coverage_nightly, feature(coverage_attribute))]
  ```

Tests use `rstest` for parameterised cases and `insta` for snapshots.

## Test data

**Never use real personal or financial data** in tests, fixtures or doc
examples: account numbers, real amounts, people's names. Invent obviously
fake values (account `123456789`). Real data has leaked into this public repo
before and needed a history rewrite. Real chains, banks and places (a
supermarket, a city) are fine as payees and descriptions.

**Round any statistic derived from real data** before it enters a spec,
fixture default, test or commit message — `--skew 0.30`, not `0.32`. Posting
counts, transaction rates and leg ratios reconstruct a profile of real
financial behaviour.

## Gotchas

**Debug `bc-ui` builds erase view types.** `mise.toml` sets
`CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="--cfg erase_components"`, which
takes a debug build from about 20 minutes to under a minute. `build:app` and
`build:server` clear it for release, and `lint` checks both builds. In an
interactive shell the `cargo` shim re-applies it, so
`CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS= cargo …` still builds erased;
clear it inside `mise exec` instead.

**`bc-plugins` integration tests need built plugins.** They load
`wasm32-wasip2` artifacts from `plugins/`, and fail in a checkout that has not
built them. To verify unrelated work, exclude them:

```sh
cargo nextest run --workspace -E 'not package(bc-plugins)'
```

Plugin *unit* tests run natively, where `usize` is
64-bit; only the integration tests exercise the real 32-bit target.

**The pre-commit hook runs workspace-wide lint** on any staged `.rs` file,
including a cold `wasm32-wasip2` build of the plugins. A deliberately
non-compiling intermediate commit in a multi-crate migration takes
`--no-verify`; never stub a downstream crate to satisfy the hook.

**Amounts are TEXT decimal strings, so SQLite cannot sum them.** `SUM` returns
a `real`. Every balance aggregation stays in Rust `rust_decimal`.

**`bc-ui` has two transports.** Without features it talks to Tauri;
`--features http` talks to `bc-server`. Lint both on
`wasm32-unknown-unknown`.

## Design principles

**Warn, don't block.** An unbalanced transaction saves with a warning; editing
a reconciled one is allowed with a warning. Hard errors are for
unrepresentable states: no postings, two or more elided postings, a lone
elided posting. One blocking check sits outside them: a whole-transaction edit
whose base is stale fails, because neither overwrite direction is recoverable
without a merge view.

**Schema changes may break.** The app has never been deployed. Fold changes
into the existing migrations; write no compatibility shims or data
migrations.

## Workflow

- **Copilot auto-reviews every PR.** Do not add it as a reviewer.
- **Descoped work gets an issue only when it is worth fixing.** Before calling
  a design or implementation done, name each out-of-scope item's trigger and
  outcome for one user with one ledger. File it, linked into its parent epic's
  checklist, when the outcome is a wrong money figure, lost data or a silent
  change to stored data, or when ordinary use triggers it, or when it is a
  feature worth scheduling on its own. List the rest in the PR body as known
  limitations: a missing test with no user-visible failure, a race needing two
  concurrent writers, a parser edge that fails loudly, a cosmetic glitch that
  clears on the next action.
- **A merge's `Closes #N` can be wrong.** A PR that merely references an issue
  closes it too. For each issue a merge closed, confirm the symbol or line it
  names actually changed. Squash merges make `git rev-list main..branch`
  useless for this.
