# Desktop GUI — Development Guide

## Prerequisites

Install all tooling via [mise](https://mise.jdx.dev/):

```sh
mise install
```

This installs the Rust toolchain (stable + nightly fmt), `trunk`, `tauri-cli`, and
all other required tools declared in `mise.toml`.

## Running in development mode

```sh
mise run dev:app
```

This starts two processes inside `crates/bc-ui/`:

- **`stylance --watch`** (background) — watches `*.module.scss` files and regenerates
  `style/bundle.css` on each change, enabling CSS-only hot-reload without a full WASM
  rebuild.
- **`trunk serve`** (foreground) — compiles the Leptos WASM bundle, watches for Rust
  source changes, and hot-reloads the Tauri webview on `http://localhost:1420`.

Tauri launches the application window once the dev server is ready.

## Building a release bundle

```sh
mise run build:app
```

Compiles the Leptos frontend for release (`--no-default-features --features csr`)
then bundles the Tauri native application. Output lands in
`crates/bc-app/target/release/bundle/`.

Bundling is disabled by default (`bundle.active = false` in `tauri.conf.toml`)
for development convenience; set it to `true` before a release build.

## Regenerating Tauri schemas

The `crates/bc-app/gen/schemas/` directory contains ACL manifests and JSON
Schemas used for IDE completion. They are regenerated automatically by
`cargo build -p bc-app` whenever `build.rs` runs. See
[`crates/bc-app/gen/README.md`](../../crates/bc-app/gen/README.md) for details.

## Regenerating icons

`crates/bc-app/icons/icon.svg` is the source of truth for the app icon. All
derived formats (PNG sizes, ICO, ICNS) are generated artefacts and gitignored.
They are regenerated automatically as a dependency of `dev:app` and
`build:app`, or can be run explicitly:

```sh
mise run gen:icons
```

## Notes

- The `crates/bc-ui/Trunk.toml` fixes the dev server port at **1420** and
  tells Trunk to ignore changes under `crates/bc-app/` to avoid recompile
  loops.
