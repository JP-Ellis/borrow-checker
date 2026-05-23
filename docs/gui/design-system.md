# Design System

## Principles

**Calm shell, loud numbers.** The app chrome (surfaces, borders) is deliberately muted — off-white backgrounds, near-black ink, 1px hairline borders. Monetary values break the calm: monospace, tabular, with negative tracking and semantic colour.

**Syntax for context.** Colour carries meaning, not decoration. Good/warn/bad semantic roles map directly to financial outcomes. Syntax colour roles (keyword, string, number, type, fn, comment) apply to code-like output (log lines, tag paths, CLI output).

**Mono for chrome, sans for sentences.** Fira Code renders all data, labels, and navigation chrome. Inter Tight appears only for narrative text (page titles, descriptions, help copy).

______________________________________________________________________

## Where to Find Things

| Concern | File |
| ---------------------------------------------------- | -------------------------------------------------- |
| Colour tokens (light + dark + syntax + semantic) | `crates/bc-ui/src/styles/tokens/_colors.scss` |
| Spacing scale | `crates/bc-ui/src/styles/tokens/_spacing.scss` |
| Type ramp + radius | `crates/bc-ui/src/styles/tokens/_typography.scss` |
| Breakpoint variables | `crates/bc-ui/src/styles/tokens/_breakpoints.scss` |
| Z-index scale | `crates/bc-ui/src/styles/tokens/_layout.scss` |
| Motion tokens | `crates/bc-ui/src/styles/tokens/_motion.scss` |
| Shell layout (`.console-shell`, `.top-bar`, `.page`) | `crates/bc-ui/src/styles/shell/` |
| Shared mixins (focus-ring, truncate, respond-above) | `crates/bc-ui/src/styles/mixins/` |

______________________________________________________________________

## Pattern Catalogue

These patterns are the design target for components not yet built. Implement them as Leptos components with co-located `.module.scss` files following the conventions in `component-standards.md`.

### Primitive Components

**Num** — monetary value display. Always monospace, tabular-nums, negative tracking. Colour: `--bc-good` (positive), `--bc-bad` (negative), `--bc-ink` (neutral). Implemented: `components/num/`.

**TagToken** — inline tag badge. Fira Code, `--bc-radius-tag`, no background by default (caller provides context colour). Implemented: `components/tag_token/`.

**StatusPill** — coloured pill with dot indicator. Three variants: good/warn/bad. Implemented: `components/status_pill/`.

**KbdHint** — keyboard shortcut display. Use `--bc-radius-kbd`, `--bc-surface`, `--bc-border`, `--bc-font-mono`, `var(--bc-text-eyebrow)`.

### Data Rows

**Account row** — institution logo (24px circle), account name (sans, label weight), masked number (mono, mute), balance (Num component, right-aligned). Separator: 1px `--bc-border`.

**Transaction row** — date (mono, mute, ISO format), description (sans), category tag (TagToken), amount (Num). Expandable inline for split transactions.

**Envelope row** — envelope name (sans), allocated/spent bar (full-width, `--bc-accent` fill, `--bc-surface-accent` track, 3px height), remaining amount (Num, right-aligned). Tree indent via `--bc-space-4` per level.

### Cards and KPI Tiles

**Card shell** — `--bc-radius-card` border-radius, `--bc-surface` background, `1px solid --bc-border` border, `--bc-space-6` padding.

**KPI tile** — card shell containing: eyebrow label (Fira Code, `--bc-text-eyebrow`, `--bc-ink-mute`), large value (Num, `--bc-text-page-title` or `--bc-text-section`), optional sparkline (48px tall, `--bc-accent` stroke).

### Navigation

**Top bar** — 52px fixed height. Logo mark (24px square, `--bc-accent` fill), wordmark (mono, label weight), tab strip (`.top-bar__tab` / `.top-bar__tab--active`), search trigger (280px, ⌘K hint), avatar (28px circle). All implemented in `src/styles/shell/_top-bar.scss`.

**Command palette** — full-screen overlay, `--bc-z-modal`, `--bc-surface` panel centred at 40% from top, 600px wide, fuzzy-search input, result list with keyboard navigation. Not yet implemented.

### Log and Output

**Log line** — timestamp (mono, mute, ISO), level badge (StatusPill variant), message (mono, ink). Monospace grid alignment: timestamp fixed-width, message wraps.

______________________________________________________________________

## Best Practices

1. Numbers are the loudest element on any surface — never reduce their contrast.
1. One border weight everywhere: `1px solid var(--bc-border)`. Use `--bc-border-strong` only for active/focus states.
1. Colour signals meaning. Don't use `--bc-good` for decorative green.
1. Mono for data and chrome; sans for sentences and headings.
1. Calm shell, dense data — keep surfaces muted so numbers pop.
1. Expand inline, not in modals, for transaction detail and split views.
1. CLI tone in empty states and help copy — direct, imperative, no marketing language.
1. ⌘K is universal — every action reachable from the command palette.
1. Use real mock data in stubs: AUD amounts, ISO-8601 dates, realistic institution names.
1. Both light and dark modes ship together — test derived tokens with `[data-theme="dark"]` on `<html>`.
