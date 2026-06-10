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

Baselines are generated inside the pinned Linux container (same WebKitGTK build
as CI) with a **frozen clock** (`FAKETIME="2025-01-15 12:00:00"`) so they are
stable across dates and monthly image rebuilds. Do not commit snapshots generated
outside this container — rendering differences will cause CI failures.

The `update-e2e-snapshots` workflow runs on the 1st of each month to rebuild the
container image, push it to GHCR, and regenerate baselines if anything changed.
Trigger it manually via `workflow_dispatch` to bootstrap after the Containerfile
changes, or to force a one-off baseline refresh.

## Container image

`e2e/.container-image` holds the GHCR digest reference used by CI and the
`container` mise task. CI pulls this exact image; the monthly workflow rebuilds
and re-pins it. If the file is empty, `mise run //e2e:container:build` is used
as a fallback (builds locally from the Containerfile).

To pull the pinned image locally:

```sh
mise run //e2e:container:pull   # docker pull using e2e/.container-image
```

To push a new image manually (e.g. to bootstrap before the monthly workflow has
run), build with the mise task (which targets `linux/amd64` correctly on all
hosts) then tag, push, and record the digest:

```sh
mise run //e2e:container:build
docker tag bc-e2e-native ghcr.io/jp-ellis/borrow-checker-e2e
DIGEST=$(docker push ghcr.io/jp-ellis/borrow-checker-e2e | awk '/digest:/{print $3}')
printf 'ghcr.io/jp-ellis/borrow-checker-e2e@%s\n' "$DIGEST" > e2e/.container-image
```

## Linux cache

`.linux-cache/` (gitignored) is mounted into the container as:

- `/repo/target` — Cargo build output
- `/repo/e2e/node_modules` — npm packages compiled for Linux

This keeps warm caches on the host across container runs. Created automatically
by the `container` mise task.
