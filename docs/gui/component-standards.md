# Component Standards

> Leptos 0.8 conventions for `bc-ui`. Read after `architecture.md`.

## File Layout

```
crates/bc-ui/src/
  main.rs           — CSR entry: mount_to_body(App)
  app.rs            — Router + ConsoleShell wrapper
  shell.rs          — ConsoleShell (owns palette_open signal, ⌘K listener)
  shell/            — TopBar, CommandPalette
  components/       — Primitive design-system atoms (Num, TagToken, StatusPill)
  pages/            — One directory per route
  styles/           — tokens.css only; no other stylesheets at crate root
```

Each module has one clear responsibility. Domain logic belongs in `pages/` or
`components/`, not in `shell/`.

## Resource Pattern

Every component that fetches remote data uses `LocalResource` (required because
`bc_ipc::client` futures are not `Send`). Pass the IPC client function directly
when no reactive signals are captured; use a closure when the fetch depends on
signals:

```rust
// Simple case — no reactive dependencies
let plugins = LocalResource::new(bc_ipc::client::list_plugins);

// Reactive case — re-fetches when a signal changes
let transactions = LocalResource::new(move || async move {
    data_version.get(); // subscribe
    bc_ipc::client::list_transactions(&account_id).await
});

// Match on the resource in the view
{move || match plugins.get() {
    None          => view! { <PluginsSkeleton /> }.into_any(),
    Some(Err(e))  => view! { <ErrorBanner message=format!("…: {e}") /> }.into_any(),
    Some(Ok(data)) => view! { <PluginsTable plugins=data /> }.into_any(),
}}
```

No `<Suspense>` wrapper is needed — the `None` arm renders the skeleton directly.

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
| Fetch function | IPC client fn passed directly (no wrapper needed) |
| Resource signal | `<resource>` (snake_case) |
| CSS classes | `kebab-case`, BEM preferred |
| CSS custom properties | `--bc-<token>` |

## CSS Conventions

Component styles live in a `.module.scss` file co-located with the Rust source. Stylance compiles these to hash-scoped classes, so class names can be short and descriptive without BEM nesting.

```scss
// components/my_widget/my_widget.module.scss
.container { … }
.label     { … }
```

### Using @use

Import shared SCSS when the component needs breakpoints or mixins. Never hard-code pixel values for breakpoints or copy-paste the focus-ring style.

```scss
@use '../../styles/tokens/breakpoints' as bp;
@use '../../styles/mixins/focus';
@use '../../styles/mixins/responsive';

.container {
  @include responsive.respond-above(bp.$bp-lg) {
    padding: var(--bc-space-6);
  }
}

.action:focus-visible {
  @include focus.focus-ring;
}
```

### Design token reference

Always reference `var(--bc-*)` custom properties for colours, spacing, radii, and typography. Never hard-code colour values or pixel sizes that exist in the token system. Token definitions live in `crates/bc-ui/src/styles/tokens/`.

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
