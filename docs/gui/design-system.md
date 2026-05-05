# BorrowChecker GUI — Design System

> Extracted from the prototype handoff (`Borrow Checker-handoff.zip`, `page-components-ref.jsx` and `console-shell.jsx`).
> This is the visual contract. Use token names in code — never raw hex values.
> Update this document when the design changes, not when the code diverges from it.

______________________________________________________________________

## 1. Foundations

Three principles govern every screen:

| Principle | Rule |
|---|---|
| **Calm shell** | Surfaces are off-white (`bg`) or near-black. Borders are always 1px hairlines, never heavier. |
| **Loud numbers** | Monospace, tabular figures, slight negative letter-spacing. Numbers are the loudest element on screen. |
| **Syntax for context** | Metadata uses the same colour roles as a code editor: `keyword`, `string`, `number`, `type`, `fn`, `comment`. |

______________________________________________________________________

## 2. Typography

Two families only. Never introduce a third.

| Family | Role | Usage |
|---|---|---|
| **Fira Code** 400 / 500 / 600 | Identity | All numerals, labels, kickers, log lines, command bars, keyboard shortcuts, code-like metadata |
| **Inter Tight** 400 / 500 / 600 / 700 | Narrative | Payee names, descriptions, headings, body copy, any sentence a human reads |

### Type scale

| Token | Family | Size | Weight | Letter-spacing | Example |
|---|---|---|---|---|---|
| `page-title` | Inter Tight | 32px | 600 | −0.7 | Components reference |
| `section-h2` | Inter Tight | 22px | 600 | −0.4 | Foundations |
| `card-title` | Inter Tight | 16px | 600 | −0.2 | Smart Access |
| `body` | Inter Tight | 13.5px | 400 | 0 | A tight, low-contrast sans serif… |
| `mono-row` | Fira Code | 13px | 500 | −0.2 | +4,280.55 |
| `mono-meta` | Fira Code | 11.5px | 500 | 0 | 2026-04-30 · cleared |
| `kicker` | Fira Code | 10.5px | 600 | 1.2 | NET WORTH (uppercase) |
| `caption` | Fira Code | 9.5px | 500 | 0.5 | tx · acct · env |

Numeric rendering: always `font-variant-numeric: tabular-nums` on monetary values.

______________________________________________________________________

## 3. Colour Tokens

Tokens are functional, not decorative. Always reference by token name in code.

All CSS custom property definitions use the **oklch** colour space — never hex or `rgb()`.
Calculations (alpha mixing, tinting, lightness stepping) are done in oklch, which produces
perceptually uniform results that sRGB arithmetic cannot. Hex values in the tables below are
visual reference only; convert to `oklch(...)` before writing CSS.

### 3.1 Light mode

| Token | Hex | Role |
|---|---|---|
| `bg` | `#F6F4EF` | Page background — warm off-white |
| `surface` | `#FFFEFB` | Card / panel surface |
| `surfaceAlt` | `#F1EEE7` | Secondary surface (table headers, sidebar, code blocks) |
| `surfaceHi` | `#E8E4DA` | Highlighted surface (hover state, active chip) |
| `border` | `#E2DED3` | Default hairline border |
| `borderStrong` | `#CFC9BC` | Emphasis border (selected card, dialog) |
| `ink` | `#1B1B1A` | Primary text |
| `inkSoft` | `#4D4A43` | Secondary text |
| `inkMute` | `#857F71` | Tertiary / metadata |
| `inkDim` | `#B6B0A0` | Placeholder / disabled |
| `accent` | `#C2410C` | Primary accent (configurable; burnt orange default) |
| `accentSoft` | `accent + 15% alpha` | Accent tint (selected item backgrounds) |

### 3.2 Dark mode

| Token | Hex | Role |
|---|---|---|
| `bg` | `#0E0F14` | Page background — cool slate |
| `surface` | `#15171F` | Card / panel surface |
| `surfaceAlt` | `#1B1E28` | Secondary surface |
| `surfaceHi` | `#252834` | Highlighted surface |
| `border` | `#272A36` | Default hairline border |
| `borderStrong` | `#383C4A` | Emphasis border |
| `ink` | `#E8EAEE` | Primary text |
| `inkSoft` | `#A6ADBC` | Secondary text |
| `inkMute` | `#6B7180` | Tertiary / metadata |
| `inkDim` | `#4A4F5C` | Placeholder / disabled |

### 3.3 Syntax colours

Used for code-like metadata, kickers, and console chrome — not for status states.

| Token | Light | Dark | Usage |
|---|---|---|---|
| `keyword` | `#C2410C` | `#C2410C` | Reserved words, primary actions, accent (same as accent) |
| `string` | `#3F7A2C` | `#9DD17A` | Strings, income values, positive indicators |
| `number` | `#9A6B00` | `#E0C674` | Numeric literals, amounts in code context |
| `type` | `#1F6FB0` | `#7BB7E0` | Type names, account kinds, institution labels |
| `fn` | `#7E3FA8` | `#C792EA` | Function names, plugin names, recurring indicators |
| `comment` | `#9C9384` | `#6B7180` | Inline comments, secondary annotations |

### 3.4 Semantic / status colours

Reserve these for actual state — never for decoration.

| Token | Light | Dark | Usage |
|---|---|---|---|
| `good` | `#3F7A2C` | `#52C58A` | Reconciled, on track, cleared, synced |
| `goodSoft` | `#E7EFDE` | `#1E2C24` | good background tint |
| `bad` | `#B53049` | `#E5536F` | Overspent, failed, negative delta, error |
| `badSoft` | `#F6E1E5` | `#2C1A20` | bad background tint |
| `warn` | `#9A6B00` | `#E0B05A` | Pending, unallocated, needs attention |
| `warnSoft` | `#F4ECD8` | `#2C2517` | warn background tint |

______________________________________________________________________

## 4. Spacing & Radius

### Spacing scale (4px base)

`4 · 6 · 8 · 10 · 12 · 14 · 18 · 22 · 28`

- Card inner padding: 18px (large), 14px (compact)
- Grid gaps: 12px standard, 16px between major sections
- Row padding: 9–10px normal density, 5–6px compact density

### Radius table

| Value | Used on |
|---|---|
| `3px` | Tag tokens, kbd glyphs |
| `4px` | Keyboard chips, small controls |
| `5px` | Tabs, ghost buttons |
| `6px` | Input controls, dropdowns |
| `8px` | Cards, panels, modals |
| `99px` | Status pills (fully rounded) |

Borders are always `1px`. Never combine two border colours on the same edge.

______________________________________________________________________

## 5. Primitive Components

The smallest reusable atoms. Build every screen from these — resist inventing alternatives.

### `Num`

Renders any monetary value.

- Family: Fira Code, `tabular-nums`, `letter-spacing: -0.2`
- Sign character: `+` or `−` (proper Unicode minus U+2212, not hyphen)
- Colour: `good` for positive deltas, `bad` for negative, `ink` for neutral balances
- Never render a dollar amount with `f64` formatting — source from i64 cents

### `TagToken`

Inline tag badge.

- 10.5px Fira Code · padding `1px 4px` · radius 3
- Background: `color @ 12% alpha`
- Click handler optional (adds `cursor: pointer`)
- Default colour: `string` tone

### `StatusPill`

One-word state indicator.

- Radius 99 · padding `4px 8px`
- Leading 6px circle dot in the tone colour
- Background: the soft variant of the tone
- Always one word (`synced` · `pending` · `error`)

### `kbd` / ghost button / chip-link

- `kbd`: 10.5px Fira Code, `2px 6px` padding, radius 3, `surface` background, `border` border
- Ghost button: 11px Fira Code, `4px 10px`, radius 5, `surface` background, `border` border
- Accent ghost button: same but `accentSoft` background and `accent` border + text
- Never use solid filled colour buttons outside the single primary action per region

### Terminal prompt input

- Leading `›` glyph in `accent` colour
- Fira Code 11.5px on `surfaceAlt` background
- Placeholder shows a pre-coloured example command using syntax tokens
- `⌘K` shortcut badge right-aligned

______________________________________________________________________

## 6. Pattern Catalogue

Recurring building blocks. Match these proportions precisely when adding new screens.

### Card shell

```
border-radius: 8px
border: 1px solid border
overflow: hidden
header: mono · 10–12px · inkSoft
header rhs: ghost button with "→" suffix
```

### KPI tile

```
kicker: mono · 10px · uppercase · letter-spacing 1 · inkMute
value:  Num · 22–32px · weight 700
delta:  mono · 10.5px · good or bad · sign char included
```

### Account row (sidebar / dashboard)

```
grid: [22px institution mark] [1fr name + meta] [90px delta] [130px balance]
institution mark: 22px square, radius 4, surfaceAlt, lowercase first letter, mono 10px
name: sans 13px weight 500 · ink
meta: mono 10.5px · type tone for institution · inkMute for account number
balance: Num right-aligned · bad if negative
```

### Transaction row (register)

```
grid: [70px date] [1fr payee + tags] [150px account] [180px envelope] [130px amount] [30px chevron]
date: mono 11.5px · inkMute · MM/DD format (slice from ISO)
payee: sans 13px · ink · 22px avatar square (first char, goodSoft if credit)
status badge: warnSoft/warn if pending
recurring: fn-tone ↻ glyph
tags: TagToken per tag
expand: inline (never modal) — see below
```

### Transaction inline expand

```
left panel: Fira Code code-block showing tx fields (id, posted, payee, amount, account, envelope, tags, status, imported_by)
right panel: action grid (recategorise · split · mark shared · add note · find similar · create rule) + audit trail log
padding-left: 96px (aligns with payee column)
background: surfaceAlt
```

### Envelope tree row

```
name: sans 12.5px · inkSoft · width 110px
bar: flex-1 · height 6px · radius 3 · surfaceAlt bg · accent fill (bad if overspent)
percent: mono 10.5px · right-aligned · bad if overspent
```

### Budget TickBar (detailed view)

```
height: 14px (parent) / 8px (child)
allocation marker: 1.5px vertical line at allocation boundary
day marker: fn-tone dot + vertical line at current-day position
overspent zone: hatched repeating-linear-gradient in bad@10%
```

### Sparkline (inline trend)

```
stroke: accent · 1.5px
fill: accent @ 14% alpha
no axis, no labels
use only for inline trends, never as a standalone chart
```

### Log line (activity stream)

```
format: [time · inkMute] [kind · keyword tone · bracketed] [text · ink]
left border: 2px solid tone colour (good / bad / warn / type)
font: Fira Code 11.5px · line-height 1.9
```

### Sidebar nav item

```
inactive: mono · inkSoft · transparent background · 2px transparent left border
active:   surfaceAlt background · 2px left border accent · ink text
count:    right-aligned · mono · inkMute
```

### Command palette item

```
kind tag: mono 9.5px uppercase · per-kind background (kindColor + '18')
label:    mono if cmd/tx · sans if nav/acct
hint:     mono 10.5px · inkMute · right-aligned
selected: accentSoft background · 2px left border accent
```

______________________________________________________________________

## 7. Shell & Navigation

### TopBar (52px fixed)

```
logo: 24px square · accent bg · white $ · radius 5
wordmark: "borrow-checker" · Fira Code 600 · 13.5px · hyphen in inkDim
tabs: Fira Code 12px · active = surfaceAlt + 2px bottom border accent + weight 600
search bar: 280px · surfaceAlt · 1px border · Fira Code 11.5px · ⌘K badge right
sync pill: StatusPill in good/warn/bad
avatar: 28px circle · surfaceHi · initials · Fira Code 11px weight 600
```

### Pages (tab order)

1. **dashboard** — net worth sparkline, cashflow, accounts, budget health, activity log, upcoming
1. **accounts** — 296px sidebar tree + main panel (header, cashflow chart, transaction register)
1. **budget** — KPI row + warning banner + 2-col envelope grid + quick-allocate terminal input
1. **reports** — net worth over time, spend by category (treemap), shared/personal split, cashflow
1. **plugins** — installed list + marketplace grid
1. **settings** — financial year start, fortnightly anchor, display currency, theme, data

### ⌘K Command Palette

```
trigger: ⌘K / Ctrl+K anywhere
width:   640px · maxHeight 480px
input:   Fira Code 14px · accent › glyph · placeholder "search payee, account, or run a command..."
items:   scroll list · keyboard nav (↑↓ ↵ esc) · result count bottom-right
kinds:   nav (type tone) · cmd (keyword tone) · acct (fn tone) · tx (string tone)
dismiss: esc · click outside · action run
```

______________________________________________________________________

## 8. Mock Data Reference

The prototype uses Australian locale conventions. New screens must match:

- Currency: AUD, `$` prefix, 2dp, en-AU locale (period as decimal, comma as thousands)
- Dates: ISO 8601 (`2026-04-30`) in data; `MM/DD` display in transaction rows; `day month` in sparkline labels
- Institutions: CommBank, Macquarie, American Express — abbreviate to `commbank` / `macquarie` / `amex` in mono metadata
- Account numbers: masked `•••• NNNN`
- Names: "Jamie" user persona, `jp` initials
- Error format: `error[E0001]: description` · hint lines follow
- Warning format: `WARN[E0107]: description`

______________________________________________________________________

## 9. Best Practices

Ten rules extracted directly from the prototype's components-reference page. All new components must satisfy all ten.

1. **Numbers are the loudest thing on screen.** Use `Num` for every monetary value. Tabular figures. Sign with `+` / `−`, never parentheses.
1. **One border weight, one rounding scale.** 1px hairlines. Cards 8, controls 4–6, tags 3. Never combine two border colours on the same edge.
1. **Colour carries meaning, not decoration.** Syntax tones for metadata. `good/bad/warn` for state. `accent` for one focal action per region.
1. **Mono for chrome and identity, Sans for sentences.** Mono on labels, kickers, code, numbers, kbd, log lines, command bars. Sans on names, descriptions, headlines, body copy.
1. **Calm shell, dense data.** Generous outer padding (18–22px). Inner rows compress to 8–10px. The contrast makes data dense without feeling cramped.
1. **Inline expand, not modal.** Transaction detail expands within its row. Drawers and modals break the audit-trail feel. The command palette is the only full-screen overlay.
1. **Talk like a CLI, not a chatbot.** Lowercase nav. Imperative actions (`allocate`, `reconcile`, `import`). Errors as `error[E0001]:`. No exclamation marks. No emoji.
1. **⌘K is the universal verb.** Anything reachable by click must also be reachable by palette command. New features need a palette entry before they need a button.
1. **Real-looking mocks only.** AUD, ISO dates, CommBank/Macquarie/Amex names, masked account numbers. Never lorem-ipsum a financial value.
1. **Both modes are first-class.** Build with token names, not hex codes. If a colour is not in the token set, add it to both modes before using it once.

______________________________________________________________________

*Source: prototype handoff `Borrow Checker-handoff.zip` · extracted 2026-05-05*
