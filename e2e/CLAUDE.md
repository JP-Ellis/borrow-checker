# CLAUDE.md — e2e/

Guidance for AI agents working on end-to-end tests.

## Package manager

Use **aube** (not npm/pnpm/yarn). `aubx` replaces `npx`.
The lockfile is `aube-lock.yaml`; never commit `package-lock.json`.

```sh
aube install          # install dependencies
aubx tsc --noEmit     # TypeScript type check (no emit)
```

## Running tests

Prefer `mise` tasks from the repo root — they handle dependency ordering:

```sh
mise run test:e2e               # build app + run tests locally
mise run test:e2e --container   # run in Linux container (required on macOS)
```

Or run directly from this directory (requires the app to already be built):

```sh
SKIP_BUILD=1 aubx wdio run wdio.conf.ts
```

## Test organisation

| Directory | Purpose |
|-----------|---------|
| `tests/flows/` | Functional flows (navigation, CRUD) |
| `tests/visual/` | Visual regression + design-token / APCA contrast |

Visual specs run before flow tests (see `specs` ordering in `wdio.conf.ts`)
because flow tests mutate the database.

## Adding tests

**Flow tests** — each test should:

1. Navigate to the relevant page via the top-bar nav.
1. Drive the UI with WebdriverIO selectors.
1. Assert the expected result in the DOM or SQLite.

Prefer semantic HTML selectors (`$('main')`, `$('nav[aria-label="..."]')`) over
scoped CSS class names, which may change with Stylance recompilation.

Use `data-testid` attributes only when there is no semantic alternative.

**Visual tests** — call `browser.checkScreen(tag)` and assert the result equals
`0` (pixel-perfect). Force colour scheme with
`document.documentElement.setAttribute('data-theme', 'light'|'dark')` rather
than relying on the host GTK theme.

## Screenshot baselines

Baselines are generated on Linux (WebKitGTK). Do not commit snapshots generated
on macOS or Windows — rendering differences will cause CI failures.

## Linux cache

`.linux-cache/` (gitignored) is mounted into the container as:

- `/repo/target` — Cargo build output
- `/repo/e2e/node_modules` — npm packages compiled for Linux

This keeps warm caches on the host across container runs. Created automatically
by the `container` mise task.
