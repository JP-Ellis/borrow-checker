# E2E Tests

WebdriverIO + tauri-driver end-to-end tests for the compiled Tauri desktop app.

Tests run against the real `bc-app` binary on WebKitGTK — the same engine that
ships to users. IPC, SQLite persistence, and real rendering are all exercised.

## Prerequisites

- `cargo install tauri-cli tauri-driver` (one-time)
- Linux: `webkitgtk-webdriver` package (Ubuntu 26.04+: included in `Containerfile`)
- macOS/Windows: run via the container task (see below)

## Quick start

```sh
# Install Node dependencies
aube install

# Run tests (builds the app automatically)
aubx wdio run wdio.conf.ts
```

Prefer the `mise` tasks from the repo root — they handle dependency ordering:

```sh
mise run test:e2e            # run locally (no container)
mise run test:e2e --container  # run in Linux container (recommended on macOS)
```

## Visual regression

Baselines live in `tests/visual/__snapshots__/desktop_wry/` and are generated
on Linux (WebKitGTK) for consistency. To regenerate them:

```sh
mise run test:e2e --container
```

Delete the relevant `*-wry.png` files before running to force a fresh capture.

## Test structure

| Directory | What it covers |
|-----------|---------------|
| `tests/flows/` | Full app flows: shell navigation, transaction creation |
| `tests/visual/` | Visual regression and design-token / APCA contrast checks |

## Database seeding

Each test run seeds `fixtures/test.db` via the `bc-seed` binary (invoked in
the `onPrepare` hook in `wdio.conf.ts`). The `fixtures/` directory is
gitignored — it is created at runtime.
