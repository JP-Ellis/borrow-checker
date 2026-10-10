# CLAUDE.md — e2e/

Guidance for AI agents working on end-to-end tests.

## Package manager

Use **aube** (not npm/pnpm/yarn). `aubx` replaces `npx`.
The lockfile is `aube-lock.yaml`; never commit `package-lock.json`.

```sh
aube install          # install dependencies
mise run //e2e:lint   # type-check the wdio and Playwright projects
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

## Browser suite

`web/` holds a second suite: Playwright against `borrow-checker-server`, the
web build of the UI.

```sh
mise run test:web     # build the web bundle and server, seed, run Playwright
```

It differs from the desktop suite:

- **One worker, one database.** `playwright.config.ts` starts one server on
  `fixtures/web.db`, which `test:web` reseeds before every run. Every spec
  shares that database, so a spec must not assert on a value another spec
  edits.
- **The client uses `fetch`,** so `page.route('**/rpc/<command>', …)` can hold
  a request open or fail it. Use it to reach save-race and error paths.
- **Two contexts stand in for two people.** `browser.newContext()` gives each
  page its own browser state against the same server.
- **Importers run for real.** `test:web` builds the plugins, and the server
  preopens `fixtures/web-docs` as `import.documents-root`. A spec writes its
  statements there and calls the CLI through `cli()` in `web/support/env.ts`,
  which shares the server's environment and database.

## Parallelism and the database

In the desktop suite, spec files are dealt into one group per worker, and
groups run concurrently (`maxInstances`). Each worker gets its own
`tauri-driver` (ports offset by worker slot), its own copy of the seeded
database and its own WebView data directory, because the app inherits
`BC_DB__PATH` and `XDG_DATA_HOME` from the driver that launches it — a single
shared driver would hand every session the same files.

A group runs in one app session. Between spec files, `resetOnNewSpec` restores
the seed into the live database, clears localStorage and reloads at `/`, ahead
of the next file's `before` hooks.

Consequences when writing specs:

- **Never rely on another spec file's writes.** Each file starts from the
  seed, at `/`, with empty localStorage.
- **Expect nothing else to be reset.** The user config directory and any
  backend state held outside the database carry into the next file.
- **Read the database via `DB_PATH` from `tests/support/db.ts`**, never a
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

## Selectors

Specs assert on the DOM — paths, text, ARIA labels — never on pixels. Prefer
semantic selectors (`$('main')`, `$('nav[aria-label="..."]')`) over Stylance
class names, which change on recompilation; use `data-testid` only when no
semantic alternative exists.

A scroll position is the exception: `scrollTop` and bounding-rect containment
are the only way to assert a scroll anchor held, so specs covering scroll
behavior assert on those directly.

## Seed contents

`crates/bc-seed/src/fixture.rs` builds the seed. Its inventory snapshot,
`crates/bc-seed/src/snapshots/bc_seed__fixture__tests__fixture_inventory.snap`,
records the transactions per account, the posting counts, the unbalanced
transactions and the payees. A spec that pins a count reads it from the
snapshot. A seed change that moves a pinned count shows up as a snapshot diff;
check every spec citing `fixture.rs` before accepting it.

## Dates and the clock

Specs run against the real system clock. `bc-seed` generates its data relative
to *now* (month offsets, not absolute dates), so assertions must be relative
too — derive expected months from the current date rather than hard-coding
them, or a run on the 1st of a month will fail.
