# Contributing to BorrowChecker

Thank you for your interest in contributing!

## How to Contribute

1. Fork the repository on GitHub
1. Create a feature branch: `git checkout -b feat/my-feature`
1. Make your changes — see [Development Setup](#development-setup) below
1. Ensure all checks pass (see [Checks](#checks))
1. Commit using [Conventional Commits](https://www.conventionalcommits.org/) format
1. Open a pull request against `main`

## Development Setup

**Prerequisites:** Rust stable + nightly (for `cargo fmt`), [`prek`](https://github.com/j178/prek) (`cargo install prek`), `cargo-nextest`, `cargo-hack`

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

| Type | Purpose |
|---|---|
| `feat:` | new feature |
| `fix:` | bug fix |
| `docs:` | documentation only |
| `chore:` | maintenance (CI, deps, tooling) |
| `refactor:` | code change with no feature/fix |
| `perf:` | performance improvement |
| `test:` | adding or updating tests |
| `revert:` | reverting a commit |

## Plugin Development

To write a BorrowChecker plugin, see the `bc-sdk` crate documentation (coming in Milestone 6).
Plugins are compiled to `wasm32-wasip1` and distributed as `.wasm` files.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
