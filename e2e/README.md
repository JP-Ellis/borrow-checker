# E2E Tests

Two separate test suites covering different layers of the application. Run them
independently — they have different prerequisites, tools, and CI jobs.

## Suites

| Suite | Directory | Tool | Server | What it tests |
|---|---|---|---|---|
| Web | `e2e/web/` | Playwright | `trunk serve` (port 1420) | Visual snapshots of `/__test/*` pages, shell routing |
| Native | `e2e/native/` | WebdriverIO + tauri-driver | Tauri binary | Full app flows with IPC |

## Quick start

**Web suite:**

```sh
cd e2e/web
npm install
npx playwright install --with-deps
npx playwright test
```

**Native suite** (requires `cargo install tauri-driver`):

```sh
cd e2e/native
npm install
npx wdio run wdio.conf.ts
```

See each suite's `README.md` for full setup, including platform-specific notes.
