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

This starts Trunk in watch mode (serving the Leptos frontend at `http://localhost:1420`)
and launches the Tauri application window against that dev server. Changes to
`crates/bc-ui/` are recompiled automatically by Trunk.

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

## Notes

- `crates/bc-app/icons/icon.png` is required at compile time by
  `tauri::generate_context!()` even when the bundle icon list is empty. The
  placeholder ships with the repo so the crate builds without a full asset
  pipeline.
- The `crates/bc-ui/Trunk.toml` fixes the dev server port at **1420** and
  tells Trunk to ignore changes under `crates/bc-app/` to avoid recompile
  loops.
