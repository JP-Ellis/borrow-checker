# E2E Tests

WebdriverIO + tauri-driver end-to-end tests for the compiled Tauri desktop app.

Tests run against the real `bc-app` binary on WebKitGTK — the same engine that
ships to users. IPC, SQLite persistence, and real rendering are all exercised.

## Prerequisites

Linux only — `tauri-driver` drives `WebKitWebDriver`, which has no equivalent on
macOS or Windows. On other platforms the suite exits gracefully instead of
failing.

- `cargo install tauri-cli tauri-driver` (one-time)
- Ubuntu: the `webkit2gtk-driver` package (`webkitgtk-webdriver` from 25.10 on)
- A display — headless runs need `xvfb-run`

## Quick start

```sh
# Install Node dependencies
aube install

# Run tests (builds the app automatically)
aubx wdio run wdio.conf.ts
```

Prefer the `mise` task from the repo root — it handles dependency ordering:

```sh
mise run test:e2e
```

## Test structure

All specs live in `tests/flows/` and cover full app flows: shell navigation,
transaction CRUD, budgets, and the global filter. Assertions target the DOM
(text, ARIA labels, `data-testid`) rather than rendered pixels.

## Database seeding

Each test run seeds `fixtures/test.db` via the `bc-seed` binary (invoked in
the `onPrepare` hook in `wdio.conf.ts`). The `fixtures/` directory is
gitignored — it is created at runtime.

`bc-seed` generates data relative to the current date, so specs must derive
expected dates from the clock rather than hard-coding them.
