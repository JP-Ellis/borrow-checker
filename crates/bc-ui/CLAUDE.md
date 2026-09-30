# CLAUDE.md

This file provides guidance to AI agents working in `bc-ui`.

## Target

`bc-ui` compiles **only** to `wasm32-unknown-unknown`, and `mod components`
and everything under it is `#[cfg(target_arch = "wasm32")]`-gated, so each
target sees a different module graph. Both must pass:

```sh
cargo clippy -p bc-ui --target wasm32-unknown-unknown -- -D warnings
cargo clippy -p bc-ui --target wasm32-unknown-unknown --features http -- -D warnings
cargo clippy -p bc-ui --all-targets -- -D warnings
```

**The cross-target `#[expect]` trap.** A lint that fires on one target only
makes a plain `#[expect]` *unfulfilled* on the other, which breaks
`-D warnings` too. Prefer renaming to suppressing; if a suppression must be
per-target, use `#[cfg_attr(not(target_arch = "wasm32"), expect(...))]`. An
unused `pub` item still trips `dead_code` in this binary crate, and the change
that adds its first wasm consumer must *remove* the now-unfulfilled expect.

**Unit-testing wasm-gated logic.** Tests under the gated tree do not run under
a native `cargo nextest run`. Put pure logic in a Leptos-free file and
`include!` it from the `#[cfg(test)] mod components_tests` shim in `main.rs`.
A file mixing Leptos and pure logic cannot be included; split the helper out
first.

## Where things go

- **Pages own their sub-components.** A page keeps its own `components/`
  folder; promote a component to the top-level `components/` only when a
  second page uses it.
- **Every component is a directory:** `mod.rs`, its `*.module.scss`, and a
  `qa.rs` showcase registered under `/__test/*` (debug builds only) covering
  its states and edge cases. The QA routes are the way to check a component
  visually without the full app.
- **Class names come from the module** (`import_style!`, then
  `style::class_name`), never from string literals.
- **Visual values come from the tokens** in `style/tokens/`: never a hardcoded
  colour, spacing, radius or font size. `docs/gui/design-system.md` lists them.

`docs/gui/` holds the architecture, component standards, design system and
development workflow in depth.

## SCSS

- **Never add `@use` to a `.module.scss` file.** The shared imports (`bp`,
  `focus`, `interactive`) come from `scss_prelude` in `Cargo.toml`, and an
  `@use` in a module file breaks the compilation of `style/bundle.scss`.
- **Interpolate variables in `@container` conditions,** as in
  `#{bp.$bp-md}`. A bare variable resolves in `@media` but not in
  `@container`.
- **Block comments only** (`/* */`); the toolchain rejects `//`.
- Nest `@media` inside the rule it affects.
- **Responsive grids** change `grid-template-columns` at nested breakpoints,
  hiding cells with `display: none` at the same breakpoints so headers and rows
  stay in sync. Size amount columns with `auto`, so a value never overflows
  into its neighbour, and proportional columns with `fr`.

## Leptos idioms

- Pass `ReadSignal` or `Signal` to children, not `RwSignal`, so each piece of
  state has one writer.
- `#[prop(into)]` on `String` props; `#[prop(optional)]` or
  `#[prop(default)]` over `Option<T>` where a default makes sense.
- Event handlers passed down are `Callback<T>`, invoked with `.run(value)`.
