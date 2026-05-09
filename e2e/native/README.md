# Native E2E Suite (WebdriverIO + tauri-driver)

Tests the full Tauri desktop application. Requires a compiled debug binary and
the platform's WebDriver server.

## Prerequisites

- `mise install` from the repo root (installs Rust, tauri-cli, tauri-driver, and
  all other project tools)

### Linux

```sh
sudo apt-get install webkit2gtk-driver
```

### Windows

Install [Microsoft Edge WebDriver](https://developer.microsoft.com/en-us/microsoft-edge/tools/webdriver/) and ensure it is on `PATH`.

### macOS ⚠️ Experimental

macOS WKWebView has no native WebDriver support. Testing on macOS requires [tauri-webdriver](https://github.com/danielraffel/tauri-webdriver), which is **experimental** (released February 2026) and may be unstable.

Follow the install instructions in the tauri-webdriver repository. This is **not** required for CI (Linux handles the stable baseline); macOS support is best-effort and the CI job runs with `continue-on-error: true`.

## Running

```sh
# Run all tests (builds the debug binary automatically via mise deps)
mise run test:e2e:native

# Or run directly from this directory (same effect)
mise run test
```

To skip the Tauri build when the binary is already up-to-date:

```sh
SKIP_BUILD=1 aubx wdio run wdio.conf.ts
```

## Adding flow tests

Each flow test in `tests/flows/` should:

1. Seed state via a `#[cfg(debug_assertions)]`-gated Tauri command (e.g. `__seed_accounts`) registered in `crates/bc-app/src/commands.rs`.
1. Drive the UI with WebdriverIO selectors.
1. Assert the expected result in the rendered DOM.

Prefer semantic HTML selectors (`$('main')`, `$('nav[aria-label="..."]')`) over CSS class selectors, which may be scoped by stylance.
