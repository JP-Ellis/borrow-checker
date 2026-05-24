# E2E Tests

Two separate test suites covering different layers of the application. Run them
independently — they have different prerequisites, tools, and CI jobs.

## Suites

| Suite | Directory | Tool | Server | What it tests |
|---|---|---|---|---|
| Web | `e2e/web/` | Playwright | `trunk serve` (port 1420) | Visual snapshots of `/__test/*` pages, shell routing |
| Desktop | `e2e/desktop/` | WebdriverIO + tauri-driver | Tauri binary | Full app flows with IPC |

## Quick start

Both suites use **aube** as their package manager (`aubx` in place of `npx`).

**Web suite:**

```sh
cd e2e/web
aube install
aubx playwright install --with-deps
aubx playwright test
```

**Desktop suite** (requires `cargo install tauri-driver`):

```sh
cd e2e/desktop
aube install
aubx wdio run wdio.conf.ts
```

Prefer the `mise run test:e2e:*` tasks from the repo root — they handle
dependency ordering (stylance bundle generation, Tauri binary build) automatically.

See each suite's `README.md` for full setup, including platform-specific notes.
