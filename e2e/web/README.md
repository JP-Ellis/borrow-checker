# Web E2E Suite (Playwright)

Tests the Leptos frontend running under `trunk serve`. Does **not** require a
Tauri binary — Tauri IPC is stubbed.

## Prerequisites

- `mise install` from the repo root (installs Rust, trunk, stylance-cli, and all
  other project tools)

## Running

```sh
# Run all tests
mise run test:e2e:web

# Or run directly from this directory (same effect)
mise run test

# Open interactive UI mode
aubx playwright test --ui

# Update screenshot baselines (run on Linux to match CI baselines)
mise run test:e2e:web:update
```

## Screenshot baselines

Baselines are stored next to each spec file in auto-named `*-snapshots/`
directories. They are generated on Linux so that font rendering is consistent
across runs. If you update snapshots locally on macOS or Windows, minor
rendering differences may cause CI failures.

To regenerate all baselines, run `mise run test:e2e:web:update` on a Linux
machine and commit the updated snapshot files.

## IPC stub

All Tauri IPC calls (`window.__TAURI_INTERNALS__`) are intercepted by the
global fixture in `tests/fixtures/index.ts` and return `null` by default.
Tests that need specific IPC responses can call `page.addInitScript()` before
navigating to add per-test handlers.

## Stylance bundle

`trunk serve` does not run stylance. The `test` mise task depends on
`//crates/bc-ui:gen:styles`, so the bundle is always generated before Playwright
starts. On a fresh checkout, you can also run `stylance .` from `crates/bc-ui/`
manually.
