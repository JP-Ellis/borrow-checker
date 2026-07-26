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
mise run test:e2e     # build app + run tests
```

The suite needs `tauri-driver` talking to `WebKitWebDriver`, so it runs on Linux
only (on Ubuntu, the `webkit2gtk-driver` package). On other platforms
`wdio.conf.ts` exits gracefully when the driver never comes up.

Or run directly from this directory (requires the app to already be built):

```sh
SKIP_BUILD=1 aubx wdio run wdio.conf.ts
```

## Test organisation

All specs live in `tests/flows/` and cover functional flows (navigation, CRUD).
They assert on the DOM — paths, text, ARIA labels — never on pixels.

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
