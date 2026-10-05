# GUI Architecture

> Written during M7 Phase 1. Update when architectural decisions change, not
> when the code diverges from it.

## Crate Roles

| Crate | Compile target | Role |
| ----------- | --------------- | -------------------------------------------------------------------- |
| `bc-ipc` | native + WASM | Shared serde types. Zero native-only deps. Defines `BcError`. |
| `bc-ui` | `wasm32-*` only | Leptos 0.8 CSR frontend. Depends only on `bc-ipc`, `bc-expr` and `bc-query`. |
| `bc-query` | native + WASM | Transaction query language: parser, printer, resolver, completion context. Never depends on `bc-models`. |
| `bc-service` | native | Command bodies and `dispatch`, shared by every host. |
| `bc-app` | native | Tauri host; one `rpc` command over `bc_service::dispatch`. |
| `bc-server` | native | axum host; serves `POST /rpc/{cmd}` over `bc_service::dispatch` and the `bc-ui` HTTP bundle. |
| `bc-models` | native | Shared data models (SQLite rows, account types). Used by `bc-service`; not available to `bc-ui`. |

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
1. `bc-service` is the only host crate that imports both `bc-core` and
   `bc-ipc`; `bc-core`, `bc-config` and `bc-plugins` each import `bc-ipc`
   under an optional `ipc` feature for their own DTO conversion, never for
   command logic.
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

Tauri runs `stylance` and then `trunk build` in `crates/bc-ui` (see
`crates/bc-app/Tauri.toml`).
Output goes to `crates/bc-ui/dist/`.

**WASM-only check** (no Tauri system libraries required), for each transport:

```sh
cargo clippy -p bc-ui --target wasm32-unknown-unknown
cargo clippy -p bc-ui --target wasm32-unknown-unknown --features http
```

**Web server build** — `mise run build:server` builds the `http`-featured
`bc-ui` bundle into `crates/bc-ui/dist-web/`, then builds
`borrow-checker-server`, which embeds that bundle at compile time via
`rust-embed`. The web bundle must exist before the server binary compiles.

**Web dev loop** — `mise run dev:web` runs the server against the dev
database with Trunk hot-reloading the bundle in front of it. Trunk listens on
`127.0.0.1:1421` by default; `--address` and `--port` change that, and
`--address 0.0.0.0` serves the LAN. The server stays on loopback behind
Trunk's proxy.

### CSS Build Pipeline

Global styles compile from `crates/bc-ui/style/main.scss` via Trunk's built-in SCSS support (requires `sass` on PATH, provided by `npm:sass` in `mise.toml`). Trunk emits a single compiled CSS file linked from `index.html`.

Component `.module.scss` files are processed separately by `stylance-cli`, which generates hash-scoped class names and bundles output into `style/bundle.css`. Both outputs are linked from `index.html`; they are independent pipelines.

In dev mode, `stylance --watch` and `trunk serve` run concurrently (see `Tauri.toml`). Trunk's file watcher handles hot-reload for `main.scss` changes; Stylance's watcher handles module changes.

## Command Conventions

- Names: `snake_case` verb-noun — `list_accounts`, `get_dashboard_summary`
- Handlers live in `crates/bc-service/src/commands/<domain>.rs`
- A new command needs a name in `bc_ipc::commands` (and in `ALL`), an `Args`
  struct if it takes arguments, a dispatch arm in `bc_service::dispatch`, and
  a client wrapper
- `bc-app` registers one Tauri command, `rpc`, and `bc-server` routes
  `POST /rpc/{cmd}`; both forward to `bc_service::dispatch`

## Settings → Backup Panel

An editable settings surface (backup directory, retain-count, retain-days, auto-pre-migration
toggle) with a dirty-gated save/discard bar shown only while the draft differs from the saved
settings. A "Create backup now" action triggers a manual snapshot, and a list of the open ledger's
backups each expose confirm-gated Restore and Delete actions. Each needs a second click; Restore's
label warns that BorrowChecker will restart to apply the swap. The retain-count field notes that
retention applies per automatic kind and never removes a manual backup.

## Feature Flag Matrix

| Feature | Crate | Activates |
| ------- | -------- | ----------------------------------------------- |
| `http` | `bc-ui` → `bc-ipc` | HTTP transport to `bc-server` |

## Error Propagation

```
bc-core error  →  bc-service handler  maps to  BcError  →  bc-ui ErrorBanner
```

`bc-service` maps every `bc-core` error to a `BcError` variant before returning.
`bc-ui` displays the `BcError::to_string()` in a dismissible inline error
banner — never panics.
