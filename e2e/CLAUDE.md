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
mise run test:e2e               # build app + run tests (Linux)
mise run test:e2e --container   # run in a Linux container (macOS/Windows)
```

On Linux this needs `WebKitWebDriver` (the `webkit2gtk-driver` package on
Ubuntu) for `tauri-driver` to talk to. macOS has no desktop WebDriver client,
so hosts other than Linux go through the container, whose image is built
locally from `Containerfile` and never pinned to a registry digest — CI runs
the suite natively and does not touch it. See the README for why the
macOS-capable `embedded` provider is not adopted.

Or run directly from this directory (requires the app to already be built):

```sh
SKIP_BUILD=1 aubx wdio run wdio.conf.ts
```

## Test organisation

All specs live in `tests/flows/` and cover functional flows (navigation, CRUD).
They assert on the DOM — paths, text, ARIA labels — never on pixels. Shared
helpers live in `tests/support/`.

## Parallelism and the database

Spec files run concurrently (`maxInstances`). Each worker gets its own
`tauri-driver` (ports offset by worker slot) and its own copy of the seeded
database, because the app inherits `BC_DB_PATH` from the driver that launches
it — a single shared driver would hand every session the same file.

Consequences when writing specs:

- **Never rely on another spec file's writes.** Each file starts from the same
  freshly-copied seed and is otherwise isolated.
- **Read the database via `DB_PATH` from `tests/support/db.js`**, never a
  hardcoded `fixtures/test.db` — that path no longer exists.
- **Wait before chaining off a lookup** (`await el.waitForDisplayed()`).
  Start-up competes for CPU across workers, so an element that was reliably
  present when tests ran serially may not be yet.

`onPrepare` seeds `fixtures/template.db` and checkpoints its WAL before workers
copy it; without that checkpoint the copies would be missing most of the seed.

## Avoiding stale-element warnings

Leptos replaces DOM nodes when a signal changes, so an element handle captured
before a re-render is stale afterwards. WebdriverIO recovers by re-finding it
from the original selector, but logs `Request encountered a stale element` each
time. Re-query at the point of use rather than holding a handle across an
interaction that re-renders — see `tests/support/palette.ts`.

## Adding tests

Each test should:

1. Navigate to the relevant page via the top-bar nav.
1. Drive the UI with WebdriverIO selectors.
1. Assert the expected result in the DOM or SQLite.

Prefer semantic HTML selectors (`$('main')`, `$('nav[aria-label="..."]')`) over
scoped CSS class names, which may change with Stylance recompilation.

Use `data-testid` attributes only when there is no semantic alternative.

## Dates and the clock

Specs run against the real system clock. `bc-seed` generates its data relative
to *now* (month offsets, not absolute dates), so assertions must be relative
too — derive expected months from the current date rather than hard-coding
them, or a run on the 1st of a month will fail.
