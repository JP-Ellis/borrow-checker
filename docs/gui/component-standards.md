# Component Standards

> Leptos 0.8 conventions for `bc-ui`. Read after `architecture.md`.

## File Layout

```
crates/bc-ui/src/
  main.rs           — CSR entry: mount_to_body(App)
  app.rs            — Router + ConsoleShell wrapper
  shell/            — ConsoleShell, TopBar, CommandPalette
  components/       — Primitive design-system atoms (Num, TagToken, StatusPill)
  pages/            — One file per route
  styles/           — tokens.css only; no other stylesheets at crate root
```

Each module has one clear responsibility. Domain logic belongs in `pages/` or
`components/`, not in `shell/`.

## Resource + Suspense Pattern

Every component that fetches remote data:

```rust
// 1. Async fetch — calls invoke()
async fn fetch_accounts() -> Result<Vec<bc_ipc::AccountRow>, String> {
    tauri_sys::core::invoke::<Vec<bc_ipc::AccountRow>>("list_accounts", &())
        .await
        .map_err(|e| e.to_string())
}

// 2. Resource in the component body
let accounts = Resource::new(|| (), |_| fetch_accounts());

// 3. Suspense wraps the data-dependent subtree
view! {
    <Suspense fallback=|| view! { <AccountListSkeleton /> }>
        {move || accounts.get().map(|result| match result {
            Ok(rows) => view! { <AccountList rows /> }.into_any(),
            Err(e)   => view! { <ErrorBanner message=e /> }.into_any(),
        })}
    </Suspense>
}
```

## Skeleton Components

- Match the loaded content's exact dimensions to prevent layout shift
- Background: animated gradient over `--bc-surface-alt`
- Naming: `<ComponentNameSkeleton />` — one per data-fetching component

## Mutation Pattern

After any write command (create, amend, void, allocate):

1. `tauri_sys::core::invoke("command_name", &args).await`
1. `Ok`: call `.refetch()` on every affected `Resource`
1. `Err`: surface the `BcError` display string in a dismissible inline banner

No optimistic updates in M7.

## No unwrap()

- Never `.unwrap()` in component or command code
- Map `tauri_sys` errors: `.map_err(|e| e.to_string())`
- Surface errors via inline `ErrorBanner`, never panic

## Naming Conventions

| Item | Convention |
| --------------------- | --------------------------------- |
| Component function | `PascalCase` |
| Skeleton component | `<NameSkeleton />` |
| Fetch function | `fetch_<resource>` |
| Resource signal | `<resource>` (snake_case) |
| CSS classes | `kebab-case`, BEM preferred |
| CSS custom properties | `--bc-<token>` |

## CSS Conventions

`crates/bc-ui/src/styles/tokens.css` defines all `--bc-*` custom properties. Higher-level
styles consume these tokens and make only the minimum changes needed — never hard-code raw
values that already exist as a token.

Each component that needs scoped styles owns a `<name>.module.scss` file imported via
[`stylance`](https://github.com/basro/stylance-rs). Stylance generates unique class hashes, so
SCSS classes inside a module should use short, expressive names — no need for deep BEM
hierarchies to prevent collisions:

```scss
/* accounts_list.module.scss */
.row          { ... }  /* scoped — no clash risk with another component's .row */
.row-selected { ... }
```

## Clippy in WASM Context

All workspace lints apply. Key implications:

- `#[expect(reason = "...")]` — never `#[allow]`
- `missing_docs`: all public items need rustdoc
- `module_name_repetitions`: name types without their module prefix

Check WASM-specifically:

```sh
cargo clippy -p bc-ui --target wasm32-unknown-unknown --features csr -- -D warnings
```

## Accessibility Baseline

- Semantic HTML (`<header>`, `<nav>`, `<main>`, `<button>`)
- `aria-label` on icon-only buttons and the logo mark
- `aria-live="polite"` on dynamic status elements (sync pill)
- Keyboard navigation for the command palette: ↑↓ Enter Esc
