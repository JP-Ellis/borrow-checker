# GUI Architecture

> Written during M7 Phase 1. Update when architectural decisions change, not
> when the code diverges from it.

## Crate Roles

| Crate | Compile target | Role |
| ----------- | --------------- | -------------------------------------------------------------------- |
| `bc-ipc` | native + WASM | Shared serde types. Zero native-only deps. Defines `BcError`. |
| `bc-ui` | `wasm32-*` only | Leptos 0.8 CSR frontend. Depends only on `bc-ipc`. |
| `bc-app` | native | Tauri v2 host. Wraps `bc-core`; maps results to `bc-ipc` types. |
| `bc-models` | native | Shared data models (SQLite rows, account types). Used by `bc-app`; not available to `bc-ui`. |

`bc-ui` targets `wasm32-unknown-unknown` (Tauri's browser webview), so any crate with
native-only dependencies — including `bc-core` and `bc-models` — will fail to compile into it.
`bc-ipc` is the WASM-safe data layer that carries types across the IPC boundary.

Note: `wasm32-unknown-unknown` is the correct target for Leptos running in Tauri's embedded
webview (a browser context). The `wasm32-wasip2` target used elsewhere in the workspace is for
the plugin system (Wasmtime runtime) — a different deployment environment.

## IPC Boundary Rules

1. `bc-ipc` compiles to `wasm32-unknown-unknown`. The `wasm-purity` CI job (to be added)
   will enforce this.
1. `bc-ui` never imports `bc-core`, `bc-models`, or any native-only crate. Enforced by the
   WASM compilation target; these crates must not appear in `bc-ui`'s `Cargo.toml`.
1. `bc-app` is the only crate allowed to import both `bc-core` and `bc-ipc`.
1. All commands return `Result<T, BcError>` where `T` is a `bc-ipc` type.
1. Monetary amounts: `i64` cents, never `f64`.
1. IDs: `String` (mti newtype IDs serialise to their string form).
1. Enum variants carry `#[non_exhaustive]` for forward compatibility.
1. All `bc-ipc` types implement `Send + Sync`, `Serialize`, `Deserialize`,
   `Clone`, `Debug`.

## Build Pipeline

**Development** — run from `crates/bc-app/`:

```sh
cargo tauri dev
```

Tauri executes `trunk serve --config ../bc-ui/Trunk.toml` as `beforeDevCommand`.
Trunk rebuilds the WASM on file change and serves at `http://localhost:1420`.

**Production** — run from `crates/bc-app/`:

```sh
cargo tauri build
```

Tauri executes `trunk build --config ../bc-ui/Trunk.toml --no-default-features --features csr`.
Output goes to `crates/bc-ui/dist/`.

**WASM-only check** (no Tauri system libraries required):

```sh
mise run check:wasm
```

## Command Conventions

- Names: `snake_case` verb-noun — `list_accounts`, `get_dashboard_summary`
- Each command must have a matching entry in `crates/bc-app/capabilities/`
  before it can be called from the frontend
- Handlers live in `crates/bc-app/src/commands/<domain>.rs`
- Commands are registered in `crates/bc-app/src/lib.rs` via
  `tauri::generate_handler![]`

## Feature Flag Matrix

| Feature | Crate | Activates |
| ------- | -------- | ----------------------------------------------- |
| `csr` | `bc-ui` | `leptos/csr` and `leptos_router/csr` |

SSR and hydrate features are reserved for a hypothetical future web deployment.

## Error Propagation

```
bc-core error  →  bc-app handler  maps to  BcError  →  bc-ui ErrorBanner
```

`bc-app` maps every `bc-core` error to a `BcError` variant before returning.
`bc-ui` displays the `BcError::to_string()` in a dismissible inline error
banner — never panics.
