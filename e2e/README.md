# E2E Tests

WebdriverIO + tauri-driver end-to-end tests for the compiled Tauri desktop app.

Tests run against the real `bc-app` binary on WebKitGTK — the same engine that
ships to users. IPC, SQLite persistence, and real rendering are all exercised.

## Prerequisites

`tauri-driver` covers Linux and Windows; macOS ships no desktop WebDriver
client, so the suite exits gracefully there instead of failing.

- `cargo install tauri-cli tauri-driver` (one-time)
- Ubuntu: the `webkit2gtk-driver` package (`webkitgtk-webdriver` from 25.10 on)
- A display — headless runs need `xvfb-run`

### macOS

Running the suite on macOS needs `@wdio/tauri-service` with its `embedded`
driver provider, which relies on two crates (`tauri-plugin-wdio` and
`tauri-plugin-wdio-webdriver`) plus the `@wdio/tauri-plugin` **frontend** JS
module imported into the page. That last part is the blocker: `bc-ui` is
Leptos/Trunk with no npm frontend dependencies, and without `window.wdioTauri`
the service burns a 5s timeout per probe — a trial run took 21m34s and failed
14 of 15 specs, against 4m06s and 15/15 here.

CrabNebula's driver also supports macOS but requires a paid subscription
(`CN_API_KEY`); it offers nothing over `tauri-driver` on Linux/Windows.

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
