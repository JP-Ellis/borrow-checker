# CLAUDE.md — e2e/

This file provides guidance to AI agents working on end-to-end tests.

## Package Manager

Both suites use **aube** (not npm/pnpm/yarn). Use `aubx` in place of `npx`.
The lockfile is `aube-lock.yaml`; do not commit `package-lock.json` or `pnpm-lock.yaml`.

```sh
aube install                                   # install dependencies
aubx playwright install --with-deps chromium   # install browser binaries (web suite)
```

Prefer running tests via `mise` tasks from the repo root — they handle
dependency ordering (stylance bundle, app binary build) automatically:

```sh
mise run test:e2e          # both suites
mise run test:e2e:web      # Playwright suite only
mise run test:e2e:desktop  # WebdriverIO suite only
```

## Suites

| Suite | Directory | Tool | What it tests |
|-------|-----------|------|---------------|
| Web | `web/` | Playwright | Visual snapshots of `/__test/*` pages, shell routing; Tauri IPC is stubbed |
| Desktop | `desktop/` | WebdriverIO + tauri-driver | Full app flows against a compiled Tauri binary |

See each suite's own `README.md` for prerequisites and platform-specific notes.

## Screenshot Baselines (web suite)

Baselines are generated on **Linux** so font rendering is consistent with CI.
Do not commit snapshots generated on macOS or Windows — minor rendering
differences will cause CI failures.

To regenerate baselines, run `mise run test:e2e:web --update-snapshots` inside
a Linux environment and commit the updated `*-snapshots/` files.

## Adding Desktop Flow Tests

Each flow test in `desktop/tests/flows/` should:

1. Seed state via a `#[cfg(debug_assertions)]`-gated Tauri command (e.g.
   `__seed_accounts`) registered in `crates/bc-app/src/commands.rs`.
1. Drive the UI with WebdriverIO selectors.
1. Assert the expected result in the rendered DOM.

Prefer semantic HTML selectors (`$('main')`, `$('nav[aria-label="..."]')`) over
scoped CSS class names, which may change with Stylance recompilation.
