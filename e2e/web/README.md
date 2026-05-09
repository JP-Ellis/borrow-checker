# Web E2E Suite (Playwright)

Tests the Leptos frontend running under `trunk serve`. Does **not** require a
Tauri binary — Tauri IPC is stubbed.

## Prerequisites

- `mise install` (installs `trunk`, `stylance-cli`, Rust)
- `npm install` (in this directory)
- `npx playwright install --with-deps` (downloads Chromium, Firefox, WebKit)

## Running

```sh
# Run all tests
npx playwright test

# Run a specific file
npx playwright test tests/visual/root.spec.ts

# Open interactive UI mode
npx playwright test --ui

# Update screenshot baselines (run on Linux to match CI baseline)
npx playwright test --update-snapshots
```

## Screenshot baselines

Baselines are stored next to each spec file in auto-named `*-snapshots/`
directories. They are generated on Linux (the CI runner) so that font
rendering is consistent across runs. If you update snapshots locally on macOS
or Windows, minor rendering differences may cause CI failures.

To regenerate all baselines from CI, push a branch and trigger the
`e2e-web` job manually with `--update-snapshots`.

## IPC stub

All Tauri IPC calls (`window.__TAURI_INTERNALS__`) are intercepted by the
global fixture in `tests/fixtures/index.ts` and return `null` by default.
Tests that need specific IPC responses can call `page.addInitScript()` before
navigating to add per-test handlers.

## Stylance bundle

`trunk serve` does not run stylance. If `crates/bc-ui/style/bundle.css` does
not exist (e.g. fresh checkout), run `stylance .` from `crates/bc-ui/` first.
In CI this is handled by the `e2e-web` job before starting Playwright.
